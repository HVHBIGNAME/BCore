import java.io.BufferedWriter;
import java.io.InputStream;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/** Use the initialized server's reloadable serialization context and unchanged LootProbe algorithms. */
public final class LootCatalogReference {
    private static final List<String> REGISTRY_TYPES = List.of("DATA_COMPONENT_TYPE", "LOOT_POOL_ENTRY_TYPE",
        "LOOT_FUNCTION_TYPE", "LOOT_CONDITION_TYPE", "LOOT_NUMBER_PROVIDER_TYPE", "LOOT_NBT_PROVIDER_TYPE",
        "LOOT_SCORE_PROVIDER_TYPE");
    private static Map<?, ?> config;
    private static Object gson;
    private static boolean fills;

    static void set(String name, Object value) throws Exception {
        Field field = LootProbe.class.getDeclaredField(name);
        field.setAccessible(true);
        field.set(null, value);
    }

    static Object get(String name) throws Exception {
        return NativeAccess.field(LootProbe.class, name);
    }

    static void initialize(String path) throws Exception {
        LootProbe.initialize(path);
        gson = get("gson");
        config = (Map<?, ?>) get("config");
        String mode = (String) config.get("mode");
        if (!List.of("catalog", "fill").contains(mode)) throw new IllegalArgumentException("unknown loot mode: " + mode);
        fills = mode.equals("fill");
    }

    static boolean fillsEnabled() { return fills; }

    static Map<?, ?> version() throws Exception {
        Class<?> shared = NativeAccess.type("SharedConstants");
        Object current = NativeAccess.call(shared, "getCurrentVersion");
        String name = (String) NativeAccess.call(current, "name");
        int protocol = (int) NativeAccess.call(current, "protocolVersion");
        int world = (int) NativeAccess.call(NativeAccess.call(current, "dataVersion"), "version");
        Map<?, ?> version;
        try (InputStream stream = shared.getResourceAsStream("/version.json")) {
            if (stream == null) throw new IllegalStateException("loaded server has no version.json");
            byte[] bytes = stream.readNBytes(65537);
            if (bytes.length > 65536) throw new IllegalStateException("version.json exceeds its bound");
            String text = new String(bytes, StandardCharsets.UTF_8);
            version = (Map<?, ?>) NativeAccess.call(gson, "fromJson", text, Map.class);
        }
        if (!name.equals("26.2") || protocol != 776 || world != 4903 || !name.equals(version.get("name"))
                || ((Number) version.get("protocol_version")).intValue() != 776
                || ((Number) version.get("world_version")).intValue() != 4903) {
            throw new IllegalStateException("loaded native version differs: " + version);
        }
        return version;
    }

    static Object attach(Object level) throws Exception {
        if (!NativeAccess.type("server.level.ServerLevel").isInstance(level))
            throw new IllegalStateException("expected the initialized native ServerLevel");
        if ((boolean) get("running")) throw new IllegalStateException("duplicate server bootstrap boundary");
        set("running", true);
        set("runner", Thread.currentThread());
        Object server = NativeAccess.call(level, "getServer");
        Object registries = NativeAccess.call(level, "registryAccess");
        Object lookup = NativeAccess.call(NativeAccess.call(server, "reloadableRegistries"), "lookup");
        Object jsonOps = NativeAccess.call(lookup, "createSerializationContext",
            Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null));
        set("level", level);
        set("server", server);
        set("registries", registries);
        set("lookup", lookup);
        set("jsonOps", jsonOps);
        if (fills) set("nbtOps", NativeAccess.call(lookup, "createSerializationContext",
            NativeAccess.constant("nbt.NbtOps", "INSTANCE")));
        return server;
    }

    @SuppressWarnings("unchecked")
    static Map<String, Object> tables() throws Exception {
        return (Map<String, Object>) get("TABLES");
    }

    static void exportAllTables() throws Exception {
        Object codec = LootProbe.constant("world.level.storage.loot.LootTable", "DIRECT_CODEC");
        Map<String, Object> encoded = new TreeMap<>();
        for (Map.Entry<String, Object> entry : tables().entrySet())
            encoded.put(entry.getKey(), LootProbe.encode(codec, get("jsonOps"), entry.getValue()));
        LootProbe.write("tables.json", encoded);
    }

    static int size(String registry) throws Exception {
        return (int) LootProbe.call(LootProbe.constant("core.registries.BuiltInRegistries", registry), "size");
    }

    static Map<String, Object> counts() throws Exception {
        Map<String, Object> result = new TreeMap<>();
        result.put("items", size("ITEM"));
        result.put("item_tags", LootProbe.tags(LootProbe.constant("core.registries.BuiltInRegistries", "ITEM")).size());
        result.put("potions", size("POTION"));
        result.put("loaded_tables", tables().size());
        result.put("chest_and_trial_tables", tables().keySet().stream()
            .filter(key -> key.startsWith("minecraft:chests/") || key.contains("trial_chambers/")).count());
        for (String name : List.of("ENCHANTMENT", "INSTRUMENT")) {
            Object registry = LootProbe.call(get("registries"), "lookupOrThrow",
                LootProbe.constant("core.registries.Registries", name));
            String key = name.equals("ENCHANTMENT") ? "enchantments" : "instruments";
            result.put(key, LootProbe.stream(LootProbe.call(registry, "listElements")).size());
            result.put(key + "_tags", LootProbe.tags(registry).size());
        }
        Map<String, Object> types = new TreeMap<>();
        for (String name : REGISTRY_TYPES) types.put(name, size(name));
        result.put("registry_types", types);
        return result;
    }

    static List<Object> fillCases() throws Exception {
        List<Object> cases = new ArrayList<>();
        for (Object value : (List<?>) config.get("cases")) {
            Map<?, ?> spec = (Map<?, ?>) value;
            String name = (String) spec.get("table");
            set("caseId", spec.get("id"));
            System.out.println("LOOT_CASE " + spec.get("id"));
            boolean registered = tables().containsKey(name);
            if (!registered) {
                if (!"native_lookup".equals(spec.get("missing_table_policy"))) {
                    cases.add(LootProbe.map("input", spec, "status", "missing_native_table"));
                    continue;
                }
                Object resolved = LootProbe.call(LootProbe.call(get("server"), "reloadableRegistries"),
                    "getLootTable", LootProbe.lootKey(name));
                if (resolved != LootProbe.constant("world.level.storage.loot.LootTable", "EMPTY"))
                    throw new IllegalStateException("unexpected native missing-table lookup result");
                tables().put(name, resolved);
            }
            Map<String, Object> result = LootProbe.map("input", spec, "status", "captured",
                "direct", LootProbe.route(spec, "direct"),
                "table_resolution", registered ? "registered" : "native_reloadable_empty_fallback");
            if (((Number) spec.get("size")).intValue() == 27) result.put("unpack", LootProbe.route(spec, "unpack"));
            cases.add(result);
            if (!registered) tables().remove(name);
        }
        return cases;
    }

    public static void capture(Object level) {
        try {
            Object server = attach(level);
            Map<?, ?> version = version();
            long gameTime = (long) LootProbe.call(level, "getGameTime");
            LootProbe.exportTables();
            LootProbe.exportRegistries();
            exportAllTables();
            Map<String, Object> counts = counts();
            int caseCount = 0;
            if (fills) {
                Object preparation = LootProbe.prepareChest();
                List<Object> cases = fillCases();
                caseCount = cases.size();
                LootProbe.write("observations.raw.json", LootProbe.map("schema", 1, "minecraft", "26.2", "protocol", 776,
                    "jar_sha256", config.get("jar_sha256"), "world_seed", Long.toString((long) LootProbe.call(level, "getSeed")),
                    "game_time", Long.toString(gameTime), "gameplay_ticks", 0, "context", config.get("context"),
                    "preparation", preparation, "cases", cases, "event_count", get("eventCount")));
            }
            if ((long) LootProbe.call(level, "getGameTime") != gameTime)
                throw new IllegalStateException("game time advanced during loot capture");
            LootProbe.write("instrumentation.json", LootProbe.map("methods", get("INSTRUMENTED"), "classes", get("TRANSFORMED")));
            ((BufferedWriter) get("trace")).close();
            Map<String, Object> catalog = LootProbe.map("schema", "bcore.native-loot-catalog.v1", "mode", config.get("mode"),
                "minecraft", "26.2", "protocol", 776, "data_version", 4903, "jar_sha256", config.get("jar_sha256"),
                "native_version", version, "gameplay_ticks", 0, "game_time", Long.toString(gameTime),
                "world_seed_i64", Long.toString((long) LootProbe.call(level, "getSeed")),
                "server_class", server.getClass().getName(), "level_class", level.getClass().getName(),
                "lookup_source", "MinecraftServer.reloadableRegistries().lookup()",
                "lookup_class", get("lookup").getClass().getName(), "serialization_context_class", get("jsonOps").getClass().getName(),
                "counts", counts, "case_count", caseCount, "event_count", get("eventCount"));
            LootProbe.write("loot-catalog.json", catalog);
            System.out.println("LOOT_CATALOG_REFERENCE=" + LootProbe.call(gson, "toJson", catalog));
            System.out.flush();
            Runtime.getRuntime().halt(0);
        } catch (Throwable failure) { LootProbe.fatal(failure); }
    }
}
