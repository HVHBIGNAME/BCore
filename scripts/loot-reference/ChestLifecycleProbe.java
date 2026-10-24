import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Base64;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicLong;

/** Fresh actual ChestBlockEntities loaded with original BlockEntity.loadStatic. */
public final class ChestLifecycleProbe {
    private static final List<String> METHODS = new CopyOnWriteArrayList<>();
    private static final List<Object> CLASSES = new CopyOnWriteArrayList<>();
    private static final List<Object> RANDOMS = new ArrayList<>();
    private static Object gson, level, server, registries, jsonOps, position, state;
    private static Path output;
    private static Map<?, ?> config;
    private static Thread runner;
    private static boolean started;
    private static Map<String, Integer> counts;
    private static List<Object> contexts, resolutions;
    private static List<String> fillSeeds;

    private interface Action { Object run() throws Exception; }
    private static Object call(Object target, String method, Object... args) throws Exception { return NativeAccess.call(target, method, args); }
    private static Object constant(String owner, String field) throws Exception { return NativeAccess.constant(owner, field); }
    private static Class<?> type(String name) throws Exception { return NativeAccess.type(name); }
    private static Map<String, Object> map(Object... pairs) { return NativeAccess.map(pairs); }
    static String sha(byte[] bytes) throws Exception { return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)); }

    static void initialize(String path) throws Exception {
        Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        gson = call(call(builder, "serializeNulls"), "create");
        config = (Map<?, ?>) call(gson, "fromJson", Files.readString(Path.of(path)), Map.class);
        output = Path.of((String) config.get("output"));
    }
    static void instrumented(String method) { METHODS.add(method); }
    static void transformed(String name, byte[] before, byte[] after) throws Exception {
        CLASSES.add(map("class", name, "original_sha256", sha(before), "instrumented_sha256", sha(after)));
    }
    public static void fatal(Throwable error) { error.printStackTrace(); Runtime.getRuntime().halt(3); }
    public static void unexpectedTick() { fatal(new IllegalStateException("unexpected gameplay tick")); }
    private static boolean observingAction() { return Thread.currentThread() == runner && counts != null; }
    public static void enter(String method) {
        if (observingAction()) counts.merge(method, 1, Integer::sum);
    }
    public static void nativeFill(long seed) {
        if (!observingAction()) return;
        if (seed == 0) fatal(new IllegalStateException("seed-zero loot fill is outside this capture"));
        fillSeeds.add(Long.toString(seed));
    }
    public static void resolvedTable(Object table) {
        if (!observingAction()) return;
        try { resolutions.add(map("is_native_empty", table == constant("world.level.storage.loot.LootTable", "EMPTY"))); }
        catch (Throwable error) { fatal(error); }
    }
    public static void createdContext(Object context) {
        if (!observingAction()) return;
        try {
            Object random = call(context, "getRandom");
            RANDOMS.add(random);
            Object origin = call(context, "getOptionalParameter", constant("world.level.storage.loot.parameters.LootContextParams", "ORIGIN"));
            Object entity = call(context, "getOptionalParameter", constant("world.level.storage.loot.parameters.LootContextParams", "THIS_ENTITY"));
            contexts.add(map("rng_index", RANDOMS.size() - 1, "random", rng(random),
                "luck_f32_bits", Integer.toUnsignedString(Float.floatToRawIntBits((float) call(context, "getLuck"))),
                "same_server_level", call(context, "getLevel") == level,
                "origin", List.of(call(origin, "x"), call(origin, "y"), call(origin, "z")),
                "this_entity", entity == null ? null : entity.getClass().getName()));
        } catch (Throwable error) { fatal(error); }
    }

    private static Object rng(Object random) throws Exception {
        if (!random.getClass().getSimpleName().equals("LegacyRandomSource"))
            throw new IllegalStateException("expected explicit-seed LegacyRandomSource");
        Object gaussian = NativeAccess.field(random, "gaussianSource");
        return map("class", random.getClass().getName(), "seed48", Long.toString(((AtomicLong) NativeAccess.field(random, "seed")).get()),
            "gaussian_cached", NativeAccess.field(gaussian, "haveNextNextGaussian"),
            "gaussian_bits", Long.toUnsignedString(Double.doubleToRawLongBits((double) NativeAccess.field(gaussian, "nextNextGaussian"))));
    }
    private static List<Object> rngStates() throws Exception {
        List<Object> result = new ArrayList<>();
        for (Object random : RANDOMS) result.add(rng(random));
        return result;
    }
    private static Object measure(List<Object> operations, String name, Action action) throws Exception {
        List<Object> before = rngStates();
        counts = new TreeMap<>(); contexts = new ArrayList<>(); resolutions = new ArrayList<>(); fillSeeds = new ArrayList<>();
        Object result;
        try { result = action.run(); }
        finally {
            Map<String, Integer> observed = counts;
            counts = null;
            operations.add(map("operation", name, "method_entries", observed, "fill_seeds_i64", fillSeeds,
                "new_contexts", contexts, "table_resolutions", resolutions,
                "loot_rng_states_before", before, "loot_rng_states_after", rngStates()));
        }
        return result;
    }
    private static byte[] bytes(Object tag) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        call(type("nbt.NbtIo"), "write", tag, new DataOutputStream(bytes));
        return bytes.toByteArray();
    }
    private static Object read(byte[] bytes) throws Exception {
        if (bytes.length > 1024 * 1024) throw new IllegalArgumentException("bounded NBT input exceeds 1 MiB");
        return call(type("nbt.NbtIo"), "read", new DataInputStream(new ByteArrayInputStream(bytes)),
            call(type("nbt.NbtAccounter"), "create", 4L * 1024 * 1024));
    }
    private static Object blob(Object tag) throws Exception {
        byte[] bytes = bytes(tag);
        return map("base64", Base64.getEncoder().encodeToString(bytes), "sha256", sha(bytes), "snbt", tag.toString());
    }
    private static Object encode(Object codec, Object value) throws Exception {
        return call(call(codec, "encodeStart", jsonOps, value), "getOrThrow");
    }
    private static Object memory(Object chest) throws Exception {
        List<Object> slots = new ArrayList<>();
        for (Object item : (List<?>) NativeAccess.field(chest, "items")) {
            slots.add((boolean) call(item, "isEmpty") ? null : map(
                "stack", encode(constant("world.item.ItemStack", "CODEC"), item),
                "effective_components", encode(constant("core.component.DataComponentMap", "CODEC"), call(item, "getComponents"))));
        }
        Object table = call(chest, "getLootTable");
        return map("loot_table", table == null ? null : call(table, "identifier").toString(),
            "loot_seed_i64", Long.toString((long) call(chest, "getLootTableSeed")), "slots", slots,
            "native_level_attached", call(chest, "getLevel") == level);
    }
    private static Object snapshot(Object chest, String boundary, List<Object> snapshots, List<Object> operations) throws Exception {
        Object before = memory(chest);
        Object saved = measure(operations, "save/" + boundary, () -> call(chest, "saveWithFullMetadata", registries));
        Object after = memory(chest);
        if (!before.equals(after)) throw new IllegalStateException("native save changed chest memory");
        snapshots.add(map("boundary", boundary, "memory", after, "saved", blob(saved)));
        return saved;
    }
    private static Object fresh(Object tag, List<Object> operations, String operation) throws Exception {
        Object previous = call(level, "getBlockEntity", position);
        Object loaded = measure(operations, operation, () -> {
            Object chest = call(type("world.level.block.entity.BlockEntity"), "loadStatic", position, state, read(bytes(tag)), registries);
            if (chest == null || !chest.getClass().getSimpleName().equals("ChestBlockEntity"))
                throw new IllegalStateException("native saved chest load failed");
            call(level, "setBlockEntity", chest);
            if (chest == previous || call(level, "getBlockEntity", position) != chest || call(chest, "getLevel") != level)
                throw new IllegalStateException("fresh loaded chest was not registered in the real ServerLevel");
            return chest;
        });
        return loaded;
    }
    private static Object input(Map<?, ?> spec) throws Exception {
        if (spec.get("input_kind").equals("preset"))
            return call(type("nbt.TagParser"), "parseCompoundFully", spec.get("snbt"));
        Map<?, ?> encoded = (Map<?, ?>) spec.get("base_nbt");
        byte[] raw = Base64.getDecoder().decode((String) encoded.get("base64"));
        if (!sha(raw).equals(encoded.get("sha256"))) throw new IllegalArgumentException("prior native input hash mismatch");
        Object tag = read(raw);
        if (spec.get("input_kind").equals("deferred_plus_items")) {
            Object preset = call(type("nbt.TagParser"), "parseCompoundFully", spec.get("items_snbt"));
            call(tag, "put", "Items", call(call(preset, "get", "Items"), "copy"));
        }
        return tag;
    }
    private static Object captureCase(Map<?, ?> spec) throws Exception {
        RANDOMS.clear();
        List<Object> operations = new ArrayList<>(), snapshots = new ArrayList<>();
        Object tag = input(spec);
        Object chest = fresh(tag, operations, "load_initial");
        snapshot(chest, "loaded_before_unpack", snapshots, operations);
        measure(operations, "first_unpack_null", () -> { call(chest, "unpackLootTable", new Object[]{null}); return null; });
        Object saved = snapshot(chest, "after_first_unpack", snapshots, operations);
        measure(operations, "second_unpack_null", () -> { call(chest, "unpackLootTable", new Object[]{null}); return null; });
        snapshot(chest, "after_second_unpack", snapshots, operations);
        Object reopened = fresh(saved, operations, "load_saved_materialized_into_fresh_chest");
        snapshot(reopened, "fresh_loaded_materialized", snapshots, operations);
        measure(operations, "reopened_unpack_null", () -> { call(reopened, "unpackLootTable", new Object[]{null}); return null; });
        snapshot(reopened, "after_reopened_unpack", snapshots, operations);
        measure(operations, "reopened_getItem_all_slots", () -> {
            for (int slot = 0; slot < 27; slot++) call(reopened, "getItem", slot);
            return null;
        });
        snapshot(reopened, "after_reopened_getItem", snapshots, operations);
        return map("id", spec.get("id"), "input", blob(tag), "fresh_load_api", "BlockEntity.loadStatic(pos,state,NbtIo.read(savedBytes),registries); ServerLevel.setBlockEntity",
            "snapshots", snapshots, "operations", operations);
    }
    private static void write(String name, Object value) throws Exception {
        Files.writeString(output.resolve(name), (String) call(gson, "toJson", value) + "\n", StandardCharsets.UTF_8, StandardOpenOption.CREATE_NEW);
    }

    public static void run(Object serverLevel) {
        try {
            if (started) throw new IllegalStateException("duplicate native lifecycle boundary");
            started = true; runner = Thread.currentThread(); level = serverLevel;
            server = call(level, "getServer"); registries = call(level, "registryAccess");
            Object lookup = call(call(server, "reloadableRegistries"), "lookup");
            jsonOps = call(lookup, "createSerializationContext", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null));
            List<?> xyz = (List<?>) config.get("block_pos");
            int x = ((Number) xyz.get(0)).intValue(), y = ((Number) xyz.get(1)).intValue(), z = ((Number) xyz.get(2)).intValue();
            position = NativeAccess.make("core.BlockPos", x, y, z);
            state = call(constant("world.level.block.Blocks", "CHEST"), "defaultBlockState");
            long gameTime = (long) call(level, "getGameTime");
            Object chunk = call(call(level, "getChunkSource"), "getChunk", x >> 4, z >> 4,
                constant("world.level.chunk.status.ChunkStatus", "FULL"), true);
            call(level, "setBlock", position, state, 2);
            List<Object> cases = new ArrayList<>();
            for (Object spec : (List<?>) config.get("cases")) cases.add(captureCase((Map<?, ?>) spec));
            if (gameTime != (long) call(level, "getGameTime")) throw new IllegalStateException("game time advanced");
            write("observations.raw.json", map("schema", "bcore.native-chest-lifecycle.raw.v1", "minecraft", "26.1", "protocol", 775,
                "world_seed_i64", Long.toString((long) call(level, "getSeed")), "block_pos", xyz,
                "chunk_status", call(call(chunk, "getPersistedStatus"), "getName"), "gameplay_ticks", 0,
                "player", null, "menu_creation_exercised", false, "cases", cases));
            write("instrumentation.json", map("methods", METHODS.stream().sorted().toList(),
                "classes", CLASSES.stream().sorted(java.util.Comparator.comparing(row -> ((Map<?, ?>) row).get("class").toString())).toList()));
            System.out.println("CHEST_LIFECYCLE_NATIVE_COMPLETE cases=" + cases.size());
            Runtime.getRuntime().halt(0);
        } catch (Throwable error) { fatal(error); }
    }
}
