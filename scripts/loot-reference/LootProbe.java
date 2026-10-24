import java.io.BufferedWriter;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.security.MessageDigest;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collections;
import java.util.Deque;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.TreeMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicLong;
import java.util.stream.Stream;

/** No substitute level, loot, RNG, registry, container, or item implementation. */
public final class LootProbe {
    private static final List<String> INSTRUMENTED = new CopyOnWriteArrayList<>();
    private static final List<Object> TRANSFORMED = new CopyOnWriteArrayList<>();
    private static final ThreadLocal<Boolean> OBSERVING = ThreadLocal.withInitial(() -> false);
    private static final ThreadLocal<Deque<Frame>> FRAMES = ThreadLocal.withInitial(ArrayDeque::new);
    private static Path output;
    private static Object gson, level, server, registries, lookup, jsonOps, nbtOps, chest;
    private static Map<?, ?> config;
    private static BufferedWriter trace;
    private static Thread runner;
    private static boolean running;
    private static boolean active;
    private static int eventCount;
    private static long callCounter;
    private static String caseId, route;
    private static Object contextRandom;
    private static List<Object> stages;
    private static List<Object> placements;
    private static final Map<String, Object> TABLES = new TreeMap<>();
    private record Frame(long id, String method) { }

    static Object call(Object target, String name, Object... args) throws Exception {
        return NativeAccess.call(target, name, args);
    }
    static Object constant(String owner, String name) throws Exception { return NativeAccess.constant(owner, name); }
    static Object make(String name, Object... args) throws Exception { return NativeAccess.make(name, args); }
    static Map<String, Object> map(Object... values) { return NativeAccess.map(values); }
    static Class<?> type(String name) throws Exception { return NativeAccess.type(name); }

    static void initialize(String path) throws Exception {
        Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        gson = call(call(builder, "serializeNulls"), "create");
        config = (Map<?, ?>) call(gson, "fromJson", Files.readString(Path.of(path)), Map.class);
        output = Path.of((String) config.get("output"));
        trace = Files.newBufferedWriter(output.resolve("trace.jsonl"), StandardCharsets.UTF_8,
            StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE);
    }

    static void instrumented(String method) { INSTRUMENTED.add(method); }
    static String sha(byte[] bytes) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }
    static void transformed(String name, byte[] original, byte[] changed) throws Exception {
        TRANSFORMED.add(map("class", name, "original_sha256", sha(original), "instrumented_sha256", sha(changed)));
    }
    public static void fatal(Throwable error) {
        error.printStackTrace();
        try { if (trace != null) trace.close(); }
        catch (Exception closeError) { closeError.printStackTrace(); }
        Runtime.getRuntime().halt(3);
    }
    public static void unexpectedTick() { fatal(new IllegalStateException("oracle entered a gameplay tick")); }

    static void write(String filename, Object value) throws Exception {
        Files.writeString(output.resolve(filename), (String) call(gson, "toJson", value) + "\n",
            StandardCharsets.UTF_8, StandardOpenOption.CREATE_NEW);
    }
    static Object encode(Object codec, Object ops, Object value) throws Exception {
        return call(call(codec, "encodeStart", ops, value), "getOrThrow");
    }
    static String key(Object holder) throws Exception {
        return call(call(holder, "key"), "identifier").toString();
    }
    static List<?> stream(Object value) {
        try (Stream<?> values = (Stream<?>) value) { return values.toList(); }
    }
    static Object stackCodec(Object stack) throws Exception {
        if ((boolean) call(stack, "isEmpty")) return null;
        return encode(constant("world.item.ItemStack", "CODEC"), jsonOps, stack);
    }
    static Object nbt(Object value) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        call(type("nbt.NbtIo"), "write", value, new DataOutputStream(bytes));
        return map("base64", Base64.getEncoder().encodeToString(bytes.toByteArray()), "sha256", sha(bytes.toByteArray()));
    }
    static Object components(Object value) throws Exception {
        List<String> transientTypes = new ArrayList<>();
        for (Object component : (Iterable<?>) value) {
            Object componentType = call(component, "type");
            if (call(componentType, "codec") == null)
                transientTypes.add(call(constant("core.registries.BuiltInRegistries", "DATA_COMPONENT_TYPE"), "getKey", componentType).toString());
        }
        Collections.sort(transientTypes);
        return map("persistent", encode(constant("core.component.DataComponentMap", "CODEC"), jsonOps, value),
            "unserialized_transient_types", transientTypes);
    }
    static Object stackDetails(Object stack) throws Exception {
        if ((boolean) call(stack, "isEmpty")) return null;
        Object codec = constant("world.item.ItemStack", "CODEC");
        return map("stack", encode(codec, jsonOps, stack),
            "effective_components", components(call(stack, "getComponents")),
            "max_stack_size", call(stack, "getMaxStackSize"),
            "nbt", nbt(encode(codec, nbtOps, stack)));
    }
    static List<Object> slots(Object container) throws Exception {
        List<Object> result = new ArrayList<>();
        // Reading the backing list avoids triggering an unpack during observation.
        List<?> items = (List<?>) NativeAccess.field(container, "items");
        for (Object item : items) result.add(stackDetails(item));
        return result;
    }

    static Object primitive(Object value) {
        if (value instanceof Long number) return map("i64", number.toString());
        if (value instanceof Float number) return map("f32", number, "bits", Integer.toUnsignedString(Float.floatToRawIntBits(number)));
        if (value instanceof Double number) return map("f64", number, "bits", Long.toUnsignedString(Double.doubleToRawLongBits(number)));
        return value;
    }
    static Object rng(Object random) throws Exception {
        if (random == null) return null;
        String name = random.getClass().getName();
        if (!name.equals("net.minecraft.world.level.levelgen.LegacyRandomSource"))
            return map("class", name, "state_authority", "unavailable for this RNG implementation");
        Object gaussian = NativeAccess.field(random, "gaussianSource");
        return map("class", name, "seed48", Long.toString(((AtomicLong) NativeAccess.field(random, "seed")).get()),
            "gaussian_cached", NativeAccess.field(gaussian, "haveNextNextGaussian"),
            "gaussian_bits", Long.toUnsignedString(Double.doubleToRawLongBits((double) NativeAccess.field(gaussian, "nextNextGaussian"))));
    }
    static Object continuation(Object random) throws Exception {
        if (random == null || !random.getClass().getSimpleName().equals("LegacyRandomSource"))
            return map("status", "unavailable");
        long seed = ((AtomicLong) NativeAccess.field(random, "seed")).get();
        Object copy = make("world.level.levelgen.LegacyRandomSource", seed ^ 25214903917L);
        return map("status", "captured", "source", "native copy reconstructed from the observed 48-bit state; original not advanced",
            "next_int_1000", call(copy, "nextInt", 1000), "next_long", primitive(call(copy, "nextLong")),
            "next_boolean", call(copy, "nextBoolean"), "next_float", primitive(call(copy, "nextFloat")),
            "next_double", primitive(call(copy, "nextDouble")), "after", rng(copy));
    }
    static Object context(Object value) throws Exception {
        Object originKey = constant("world.level.storage.loot.parameters.LootContextParams", "ORIGIN");
        Object origin = call(value, "getOptionalParameter", originKey);
        Object entity = call(value, "getOptionalParameter", constant("world.level.storage.loot.parameters.LootContextParams", "THIS_ENTITY"));
        return map("luck", primitive(call(value, "getLuck")), "same_server_level", call(value, "getLevel") == level,
            "origin", origin == null ? null : List.of(call(origin, "x"), call(origin, "y"), call(origin, "z")),
            "this_entity", entity == null ? null : entity.getClass().getName(), "random", rng(call(value, "getRandom")));
    }
    static Object describe(Object value) throws Exception {
        if (value == null || value instanceof Number || value instanceof Boolean || value instanceof String) return primitive(value);
        String name = value.getClass().getName();
        if (name.equals("net.minecraft.world.item.ItemStack")) return stackCodec(value);
        if (name.equals("net.minecraft.world.level.storage.loot.LootContext")) return context(value);
        if (value instanceof List<?> list) {
            List<Object> copy = new ArrayList<>();
            for (Object entry : list) copy.add(describe(entry));
            return copy;
        }
        if (value instanceof Optional<?> optional) return optional.isPresent() ? describe(optional.get()) : null;
        if (name.equals("net.minecraft.resources.ResourceKey")) return call(value, "identifier").toString();
        if (name.equals("net.minecraft.resources.Identifier")) return value.toString();
        return map("class", name);
    }
    static boolean isRng(String method) {
        return method.startsWith("world/level/levelgen/LegacyRandomSource.") || method.startsWith("world/level/levelgen/BitRandomSource.");
    }
    static void event(Map<String, Object> row) throws Exception {
        if (++eventCount > 500000) throw new IllegalStateException("bounded loot trace exceeded 500000 events");
        row.put("event", eventCount);
        row.put("case", caseId);
        row.put("route", route);
        trace.write((String) call(gson, "toJson", row));
        trace.newLine();
    }
    static boolean observe() { return active && Thread.currentThread() == runner && !OBSERVING.get(); }

    public static void enter(String method, Object self, Object[] args) {
        if (!observe()) return;
        OBSERVING.set(true);
        try {
            Deque<Frame> frames = FRAMES.get();
            long parent = frames.isEmpty() ? 0 : frames.peek().id;
            Frame frame = new Frame(++callCounter, method);
            frames.push(frame);
            List<Object> arguments = new ArrayList<>();
            for (Object arg : args) arguments.add(describe(arg));
            Map<String, Object> row = map("boundary", "enter", "call", frame.id, "parent", parent,
                "method", method, "arguments", arguments);
            if (isRng(method)) row.put("rng", rng(self));
            if (method.startsWith("world/level/storage/loot/LootTable.fill")) row.put("loot_table_key_at_fill", call(chest, "getLootTable") == null ? null : describe(call(chest, "getLootTable")));
            if (method.contains(".setItem(")) {
                Object placement = map("slot", args[0], "stack", stackCodec(args[1]));
                placements.add(placement);
            }
            event(row);
        } catch (Throwable error) { fatal(error); }
        finally { OBSERVING.set(false); }
    }

    public static void exit(Object result, String method, Object self, Object[] args) {
        if (!observe()) return;
        OBSERVING.set(true);
        try {
            Frame frame = FRAMES.get().pop();
            if (!frame.method.equals(method)) throw new IllegalStateException("unpaired method trace: " + method);
            if (method.startsWith("world/level/storage/loot/LootContext$Builder.create("))
                contextRandom = call(result, "getRandom");
            Map<String, Object> row = map("boundary", "exit", "call", frame.id, "method", method, "result", describe(result));
            if (isRng(method)) row.put("rng", rng(self));
            if (method.contains("LootTable.getRandomItems(") && result instanceof List<?>
                    || method.contains("LootTable.getAvailableSlots(") || method.contains("LootTable.shuffleAndSplitItems(")) {
                Object values = method.contains("shuffleAndSplitItems") ? args[0] : result;
                Object stage = map("method", method, "values", describe(values), "rng", rng(contextRandom));
                stages.add(stage);
                row.put("stage", stage);
            }
            if (method.contains("LootTable.fill(")) row.put("rng_after_fill", rng(contextRandom));
            event(row);
        } catch (Throwable error) { fatal(error); }
        finally { OBSERVING.set(false); }
    }

    static Object decodeStack(Object value) throws Exception {
        if (value == null) return constant("world.item.ItemStack", "EMPTY");
        Object json = call(gson, "toJsonTree", value);
        return call(call(constant("world.item.ItemStack", "CODEC"), "parse", jsonOps, json), "getOrThrow");
    }
    static void prefill(Object container, Map<?, ?> spec) throws Exception {
        call(container, "clearContent");
        Object fill = spec.get("fill_existing");
        if (fill != null) for (int slot = 0; slot < (int) call(container, "getContainerSize"); slot++)
            call(container, "setItem", slot, decodeStack(fill));
        for (Object value : (List<?>) spec.get("preexisting")) {
            Map<?, ?> item = (Map<?, ?>) value;
            call(container, "setItem", ((Number) item.get("slot")).intValue(), decodeStack(item.get("stack")));
        }
    }
    static Object params() throws Exception {
        Map<?, ?> context = (Map<?, ?>) config.get("context");
        List<?> xyz = (List<?>) context.get("origin_block");
        Object pos = make("core.BlockPos", ((Number) xyz.get(0)).intValue(), ((Number) xyz.get(1)).intValue(), ((Number) xyz.get(2)).intValue());
        Object builder = make("world.level.storage.loot.LootParams$Builder", level);
        call(builder, "withParameter", constant("world.level.storage.loot.parameters.LootContextParams", "ORIGIN"), call(type("world.phys.Vec3"), "atCenterOf", pos));
        call(builder, "withLuck", ((Number) context.get("luck")).floatValue());
        return call(builder, "create", constant("world.level.storage.loot.parameters.LootContextParamSets", "CHEST"));
    }
    static Object lootKey(String name) throws Exception {
        return call(type("resources.ResourceKey"), "create", constant("core.registries.Registries", "LOOT_TABLE"),
            call(type("resources.Identifier"), "parse", name));
    }
    static Object route(Map<?, ?> spec, String name) throws Exception {
        route = name;
        stages = new ArrayList<>();
        placements = new ArrayList<>();
        contextRandom = null;
        callCounter = 0;
        Object container = name.equals("unpack") ? chest : make("world.SimpleContainer", ((Number) spec.get("size")).intValue());
        if (name.equals("unpack")) {
            call(chest, "setLootTable", new Object[]{null});
            call(chest, "setLootTableSeed", 0L);
        }
        prefill(container, spec);
        List<Object> before = slots(container);
        long seed = Long.parseLong((String) spec.get("seed"));
        Object key = lootKey((String) spec.get("table"));
        if (name.equals("unpack")) call(chest, "setLootTable", key, seed);
        Object savedBefore = name.equals("unpack") ? nbt(call(chest, "saveWithFullMetadata", registries)) : null;
        int firstEvent = eventCount + 1;
        active = true;
        if (name.equals("unpack")) call(chest, "unpackLootTable", new Object[]{null});
        else call(TABLES.get(spec.get("table")), "fill", container, params(), seed);
        active = false;
        if (!FRAMES.get().isEmpty()) throw new IllegalStateException("open native calls after fill");
        if (contextRandom == null) throw new IllegalStateException("no observed native LootContext");
        Object randomAfter = rng(contextRandom);
        Object next = continuation(contextRandom);
        List<Object> after = slots(container);
        Map<String, Object> result = map("route", name, "container_class", container.getClass().getName(), "before", before,
            "after", after, "stages", stages, "placements", placements, "rng_after", randomAfter, "continuation", next,
            "first_event", firstEvent, "last_event", eventCount);
        if (name.equals("unpack")) {
            result.put("saved_before", savedBefore);
            result.put("saved_after", nbt(call(chest, "saveWithFullMetadata", registries)));
            result.put("loot_table_after", describe(call(chest, "getLootTable")));
            result.put("loot_seed_after", Long.toString((long) call(chest, "getLootTableSeed")));
            call(chest, "unpackLootTable", new Object[]{null});
            result.put("second_unpack_slots", slots(chest));
            result.put("second_unpack_rng_unchanged", rng(contextRandom).equals(randomAfter));
        }
        trace.flush();
        return result;
    }

    static List<Object> tags(Object registry) throws Exception {
        List<Object> result = new ArrayList<>();
        for (Object tag : stream(call(registry, "listTags"))) {
            List<String> members = new ArrayList<>();
            for (Object holder : (Iterable<?>) tag) members.add(key(holder));
            result.add(map("tag", call(call(tag, "key"), "location").toString(), "members_native_order", members));
        }
        result.sort(java.util.Comparator.comparing(row -> ((Map<?, ?>) row).get("tag").toString()));
        return result;
    }
    static void exportRegistries() throws Exception {
        Object items = constant("core.registries.BuiltInRegistries", "ITEM");
        List<Object> itemRows = new ArrayList<>();
        for (Object item : (Iterable<?>) items) {
            Object stack = call(item, "getDefaultInstance");
            itemRows.add(map("id", call(items, "getId", item), "key", call(items, "getKey", item).toString(),
                "components", components(call(item, "components")), "max_stack_size", call(stack, "getMaxStackSize")));
        }
        write("items.json", map("items_native_order", itemRows, "tags", tags(items)));
        Map<String, Object> typeRows = new TreeMap<>();
        for (String registryName : List.of("DATA_COMPONENT_TYPE", "LOOT_POOL_ENTRY_TYPE", "LOOT_FUNCTION_TYPE",
                "LOOT_CONDITION_TYPE", "LOOT_NUMBER_PROVIDER_TYPE", "LOOT_NBT_PROVIDER_TYPE", "LOOT_SCORE_PROVIDER_TYPE")) {
            Object registry = constant("core.registries.BuiltInRegistries", registryName);
            List<Object> values = new ArrayList<>();
            for (Object value : (Iterable<?>) registry) {
                Map<String, Object> row = map("id", call(registry, "getId", value), "key", call(registry, "getKey", value).toString());
                if (registryName.equals("DATA_COMPONENT_TYPE")) row.put("persistent", call(value, "codec") != null);
                values.add(row);
            }
            typeRows.put(registryName, values);
        }
        write("registry-types.json", typeRows);
        for (String name : List.of("ENCHANTMENT", "INSTRUMENT")) {
            Object registry = call(registries, "lookupOrThrow", constant("core.registries.Registries", name));
            Object codec = constant(name.equals("ENCHANTMENT") ? "world.item.enchantment.Enchantment" : "world.item.Instrument", "DIRECT_CODEC");
            List<Object> values = new ArrayList<>();
            for (Object holder : stream(call(registry, "listElements")))
                values.add(map("key", key(holder), "value", encode(codec, jsonOps, call(holder, "value"))));
            write(name.toLowerCase(java.util.Locale.ROOT) + ".json", map("values_native_order", values, "tags", tags(registry)));
        }
        Object potions = constant("core.registries.BuiltInRegistries", "POTION");
        List<Object> potionRows = new ArrayList<>();
        for (Object potion : (Iterable<?>) potions) {
            List<Object> effects = new ArrayList<>();
            for (Object effect : (Iterable<?>) call(potion, "getEffects"))
                effects.add(encode(constant("world.effect.MobEffectInstance", "CODEC"), jsonOps, effect));
            potionRows.add(map("id", call(potions, "getId", potion), "key", call(potions, "getKey", potion).toString(),
                "name", call(potion, "name"), "effects", effects));
        }
        write("potions.json", potionRows);
    }

    static void exportTables() throws Exception {
        Object registry = call(lookup, "lookupOrThrow", constant("core.registries.Registries", "LOOT_TABLE"));
        List<Object> tableRows = new ArrayList<>();
        for (Object holder : stream(call(registry, "listElements"))) {
            String name = key(holder);
            Object value = call(holder, "value");
            TABLES.put(name, value);
            if (name.startsWith("minecraft:chests/") || name.contains("trial_chambers/"))
                tableRows.add(map("key", name, "native_codec", encode(constant("world.level.storage.loot.LootTable", "DIRECT_CODEC"), jsonOps, value)));
        }
        tableRows.sort(java.util.Comparator.comparing(row -> ((Map<?, ?>) row).get("key").toString()));
        write("loaded-tables.json", map("all_loaded_keys", new ArrayList<>(TABLES.keySet()), "chest_and_trial_tables", tableRows));
    }

    static Object prepareChest() throws Exception {
        List<?> xyz = (List<?>) ((Map<?, ?>) config.get("context")).get("origin_block");
        int x = ((Number) xyz.get(0)).intValue(), y = ((Number) xyz.get(1)).intValue(), z = ((Number) xyz.get(2)).intValue();
        Object chunk = call(call(level, "getChunkSource"), "getChunk", x >> 4, z >> 4,
            constant("world.level.chunk.status.ChunkStatus", "FULL"), true);
        Object pos = make("core.BlockPos", x, y, z);
        Object state = call(constant("world.level.block.Blocks", "CHEST"), "defaultBlockState");
        call(level, "setBlock", pos, state, 2);
        chest = call(level, "getBlockEntity", pos);
        if (!chest.getClass().getSimpleName().equals("ChestBlockEntity")) throw new IllegalStateException("expected a real placed chest");
        return map("chunk", List.of(x >> 4, z >> 4), "chunk_class", chunk.getClass().getName(),
            "chunk_status", call(call(chunk, "getPersistedStatus"), "getName"),
            "block_pos", xyz, "chest_level_is_native", call(chest, "getLevel") == level,
            "placement", "native ServerChunkCache.getChunk(FULL,true), ServerLevel.setBlock(CHEST,2), getBlockEntity");
    }

    public static void run(Object nativeLevel) {
        try {
            if (running) throw new IllegalStateException("duplicate native server boundary");
            running = true;
            runner = Thread.currentThread();
            level = nativeLevel;
            server = call(level, "getServer");
            registries = call(level, "registryAccess");
            lookup = call(call(server, "reloadableRegistries"), "lookup");
            jsonOps = call(lookup, "createSerializationContext", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null));
            nbtOps = call(lookup, "createSerializationContext", constant("nbt.NbtOps", "INSTANCE"));
            long gameTime = (long) call(level, "getGameTime");
            exportTables();
            exportRegistries();
            Object preparation = prepareChest();
            List<Object> cases = new ArrayList<>();
            for (Object value : (List<?>) config.get("cases")) {
                Map<?, ?> spec = (Map<?, ?>) value;
                caseId = (String) spec.get("id");
                System.out.println("LOOT_CASE " + caseId);
                boolean registered = TABLES.containsKey(spec.get("table"));
                if (!TABLES.containsKey(spec.get("table"))) {
                    if (!"native_lookup".equals(spec.get("missing_table_policy"))) {
                        cases.add(map("input", spec, "status", "missing_native_table"));
                        continue;
                    }
                    Object resolved = call(call(server, "reloadableRegistries"), "getLootTable", lootKey((String) spec.get("table")));
                    if (resolved != constant("world.level.storage.loot.LootTable", "EMPTY"))
                        throw new IllegalStateException("unexpected native missing-table lookup result");
                    TABLES.put((String) spec.get("table"), resolved);
                }
                Map<String, Object> result = map("input", spec, "status", "captured", "direct", route(spec, "direct"),
                    "table_resolution", registered ? "registered" : "native_reloadable_empty_fallback");
                if (((Number) spec.get("size")).intValue() == 27) result.put("unpack", route(spec, "unpack"));
                cases.add(result);
                if (!registered) TABLES.remove(spec.get("table"));
            }
            if ((long) call(level, "getGameTime") != gameTime) throw new IllegalStateException("game time advanced");
            write("observations.raw.json", map("schema", 1, "minecraft", "26.1", "protocol", 775,
                "world_seed", Long.toString((long) call(level, "getSeed")), "game_time", Long.toString(gameTime),
                "gameplay_ticks", 0, "context", config.get("context"), "preparation", preparation, "cases", cases,
                "event_count", eventCount));
            write("instrumentation.json", map("methods", INSTRUMENTED, "classes", TRANSFORMED));
            trace.close();
            System.out.println("LOOT_NATIVE_COMPLETE cases=" + cases.size() + " events=" + eventCount);
            Runtime.getRuntime().halt(0);
        } catch (Throwable error) { fatal(error); }
    }
}
