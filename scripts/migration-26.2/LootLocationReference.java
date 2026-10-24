import java.io.BufferedWriter;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/** Invoke real location checks against controlled native chunk biome containers. */
public final class LootLocationReference {
    private static Object level;

    private static Object decode(Object value) throws Exception {
        Object json = LootProbe.call(LootCatalogReference.get("gson"), "toJsonTree", value);
        Object codec = LootProbe.call(LootProbe.constant("world.level.storage.loot.predicates.LocationCheck", "MAP_CODEC"), "codec");
        return LootProbe.call(LootProbe.call(codec, "parse", LootCatalogReference.get("jsonOps"), json), "getOrThrow");
    }

    private static Object blockPos(List<Double> origin, List<Integer> offset) throws Exception {
        return LootProbe.call(LootProbe.type("core.BlockPos"), "containing",
            origin.get(0) + offset.get(0), origin.get(1) + offset.get(1), origin.get(2) + offset.get(2));
    }

    private static Map<String, Object> context(List<Double> origin) throws Exception {
        List<Integer> offset = List.of(0, 0, 0);
        Object pos = blockPos(origin, offset);
        boolean loaded = (boolean) LootProbe.call(level, "isLoaded", pos);
        Map<String, Object> result = LootProbe.map("origin", origin, "loaded", loaded);
        if (loaded) result.put("biome", LootProbe.key(LootProbe.call(level, "getBiome", pos)));
        return result;
    }

    private static void setBiomes(String name) throws Exception {
        Object registry = LootProbe.call(LootCatalogReference.get("registries"), "lookupOrThrow",
            LootProbe.constant("core.registries.Registries", "BIOME"));
        Object key = LootProbe.call(LootProbe.type("resources.ResourceKey"), "create",
            LootProbe.constant("core.registries.Registries", "BIOME"),
            LootProbe.call(LootProbe.type("resources.Identifier"), "parse", name));
        Object holder = LootProbe.call(registry, "getOrThrow", key);
        for (int coordinate : new int[]{-1, 0}) {
            Object chunk = LootProbe.call(LootProbe.call(level, "getChunkSource"), "getChunk", coordinate, coordinate,
                LootProbe.constant("world.level.chunk.status.ChunkStatus", "FULL"), true);
            for (Object section : (Object[]) LootProbe.call(chunk, "getSections")) {
                Object biomes = LootProbe.call(section, "getBiomes");
                for (int x = 0; x < 4; x++) for (int y = 0; y < 4; y++) for (int z = 0; z < 4; z++)
                    LootProbe.call(biomes, "set", x, y, z, holder);
            }
        }
    }

    private static Map<String, Object> observe(String name, Map<String, Object> definition,
                                              List<Double> origin, List<Integer> offset) throws Exception {
        Object params = LootProbe.make("world.level.storage.loot.LootParams$Builder", level);
        if (origin != null) LootProbe.call(params, "withParameter",
            LootProbe.constant("world.level.storage.loot.parameters.LootContextParams", "ORIGIN"),
            LootProbe.make("world.phys.Vec3", origin.get(0), origin.get(1), origin.get(2)));
        Object built = LootProbe.call(params, "create", LootProbe.constant("world.level.storage.loot.parameters.LootContextParamSets",
            origin == null ? "EMPTY" : "CHEST"));
        Object random = LootProbe.make("world.level.levelgen.LegacyRandomSource", 17L);
        Object builder = LootProbe.make("world.level.storage.loot.LootContext$Builder", built);
        LootProbe.call(builder, "withOptionalRandomSource", random);
        Object lootContext = LootProbe.call(builder, "create", Optional.empty());
        Object condition = decode(definition);
        Object before = LootProbe.rng(random);
        boolean matched = (boolean) LootProbe.call(condition, "test", lootContext);
        Object after = LootProbe.rng(random);
        Map<String, Object> row = LootProbe.map("name", name, "condition", definition, "origin", origin,
            "matched", matched, "rng_before", before, "rng_after", after, "next_i64", LootProbe.call(random, "nextLong").toString());
        if (origin != null) {
            Object pos = blockPos(origin, offset);
            List<Integer> query = List.of((int) LootProbe.call(pos, "getX"), (int) LootProbe.call(pos, "getY"), (int) LootProbe.call(pos, "getZ"));
            boolean loaded = (boolean) LootProbe.call(level, "isLoaded", pos);
            row.put("query_pos", query);
            row.put("loaded", loaded);
            row.put("biome", loaded ? LootProbe.key(LootProbe.call(level, "getBiome", pos)) : null);
        }
        return row;
    }

    @SuppressWarnings("unchecked")
    public static void capture(Object nativeLevel) {
        try {
            level = nativeLevel;
            Object server = LootCatalogReference.attach(level);
            LootCatalogReference.version();
            long time = (long) LootProbe.call(level, "getGameTime");
            Object table = LootProbe.call(LootProbe.call(server, "reloadableRegistries"), "getLootTable",
                LootProbe.lootKey("minecraft:chests/abandoned_mineshaft"));
            Object encoded = LootProbe.encode(LootProbe.constant("world.level.storage.loot.LootTable", "DIRECT_CODEC"),
                LootCatalogReference.get("jsonOps"), table);
            Object gson = LootCatalogReference.get("gson");
            Map<?, ?> document = (Map<?, ?>) LootProbe.call(gson, "fromJson", LootProbe.call(gson, "toJson", encoded), Map.class);
            List<Map<String, Object>> conditions = new ArrayList<>();
            for (Object pool : (List<?>) document.get("pools")) for (Object entry : (List<?>) ((Map<?, ?>) pool).get("entries")) {
                Object entryConditions = ((Map<?, ?>) entry).get("conditions");
                for (Object condition : entryConditions == null ? List.of() : (List<?>) entryConditions)
                    if ("minecraft:location_check".equals(((Map<?, ?>) condition).get("condition")))
                        conditions.add((Map<String, Object>) condition);
            }
            if (conditions.size() != 1) throw new IllegalStateException("expected one stock mineshaft location condition");
            Map<String, Object> stock = conditions.get(0);
            List<Double> origin = List.of(8.5, 80.5, 8.5);
            LootProbe.call(LootProbe.call(level, "getChunkSource"), "getChunk", 0, 0,
                LootProbe.constant("world.level.chunk.status.ChunkStatus", "FULL"), true);
            Map<String, Object> actualContext = context(origin);
            List<Object> samples = new ArrayList<>();
            samples.add(observe("native_fill_context", stock, origin, List.of(0, 0, 0)));
            for (String biome : List.of("minecraft:plains", "minecraft:sulfur_caves")) {
                setBiomes(biome);
                for (List<Double> location : List.of(origin, List.of(-8.25, 80.5, -8.75))) {
                    samples.add(observe(biome + "/stock", stock, location, List.of(0, 0, 0)));
                    Map<String, Object> offset = new LinkedHashMap<>(stock);
                    offset.put("offsetX", 3);
                    offset.put("offsetY", -4);
                    offset.put("offsetZ", -3);
                    samples.add(observe(biome + "/offset", offset, location, List.of(3, -4, -3)));
                    samples.add(observe(biome + "/no_predicate", Map.of(), location, List.of(0, 0, 0)));
                    samples.add(observe(biome + "/empty_predicate", Map.of("predicate", Map.of()), location, List.of(0, 0, 0)));
                }
            }
            samples.add(observe("missing_origin", stock, null, List.of(0, 0, 0)));
            samples.add(observe("missing_origin_no_predicate", Map.of(), null, List.of(0, 0, 0)));
            samples.add(observe("unloaded", stock, List.of(1000000.5, 80.5, 1000000.5), List.of(0, 0, 0)));
            for (Object sample : samples) {
                Map<?, ?> row = (Map<?, ?>) sample;
                if (!row.get("rng_before").equals(row.get("rng_after")))
                    throw new IllegalStateException("location condition consumed randomness");
            }
            if ((long) LootProbe.call(level, "getGameTime") != time) throw new IllegalStateException("game time advanced");
            Map<String, Object> result = LootProbe.map("schema", "bcore.native-loot-location.v1", "minecraft", "26.2",
                "protocol", 776, "data_version", 4903, "jar_sha256", "183c0499c5f855570ee487dd38e141a53f0121f83a0b07a3bac2d8b6698823e8",
                "world_seed_i64", LootProbe.call(level, "getSeed").toString(), "gameplay_ticks", 0,
                "fill_context", actualContext, "samples", samples,
                "biome_mutations", "all quart cells of native FULL chunks (-1,-1) and (0,0), plains then sulfur_caves");
            ((BufferedWriter) LootCatalogReference.get("trace")).close();
            System.out.println("LOOT_LOCATION_REFERENCE=" + LootProbe.call(gson, "toJson", result));
            System.out.flush();
            Runtime.getRuntime().halt(0);
        } catch (Throwable failure) { LootProbe.fatal(failure); }
    }
}
