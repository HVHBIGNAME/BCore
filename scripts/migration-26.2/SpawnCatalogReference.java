import java.util.*;

/** Metadata-only entry points over the unchanged generation-spawn probe methods. */
public final class SpawnCatalogReference extends TreeReference {
    static Map<String, Object> memoryIntervals() throws Exception {
        Map<String, Object> intervals = new LinkedHashMap<>();
        for (String[] item : new String[][]{{"goat_jump", "goat.GoatAi", "TIME_BETWEEN_LONG_JUMPS"},
                {"goat_ram", "goat.GoatAi", "TIME_BETWEEN_RAMS"}, {"frog_jump", "frog.FrogAi", "TIME_BETWEEN_LONG_JUMPS"}}) {
            Object provider = GenerationSpawnProbe.constant("world.entity.animal." + item[1], item[2]);
            intervals.put(item[0], List.of(call(provider, "minInclusive"), call(provider, "maxInclusive")));
        }
        return intervals;
    }

    static Map<String, Object> metadata(Object level, String mode) throws Exception {
        GenerationSpawnProbe.level = level;
        GenerationSpawnProbe.registries = call(level, "registryAccess");
        GenerationSpawnProbe.biomeRegistry = GenerationSpawnProbe.registry("BIOME");
        GenerationSpawnProbe.entityRegistry = field("core.registries.BuiltInRegistries", "ENTITY_TYPE");
        @SuppressWarnings("unchecked")
        Map<String, Object> catalog = (Map<String, Object>) GenerationSpawnProbe.catalog();
        List<?> types = (List<?>) catalog.get("types");
        if (((List<?>) catalog.get("biomes")).size() != 66 || types.size() > 64)
            throw new IllegalStateException("unexpected biome count or spawn mask width");
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("catalog", catalog);
        switch (mode) {
            case "spawn_assets" -> {
                result.put("blocks", GenerationSpawnProbe.blockMetadata(types));
                result.put("constructors", GenerationSpawnProbe.templates(types));
            }
            case "spawn_callbacks" -> {
                result.put("constructors", GenerationSpawnProbe.templates(types));
                result.put("memory_intervals", memoryIntervals());
            }
            case "spawn_handoff" -> result.put("handoff_catalog", GenerationSpawnHandoff.catalog(types));
            default -> throw new IllegalArgumentException(mode);
        }
        if (GenerationSpawnProbe.instrumented.stream().noneMatch(name -> name.endsWith("#RandomSource.create")))
            throw new IllegalStateException("native constructor entropy boundary was not instrumented");
        result.put("minecraft", "26.2");
        result.put("protocol", 776);
        result.put("schema", 1);
        result.put("gameplay_ticks", 0);
        result.put("capture_scope", "Native loaded registries, predicates and constructor metadata at MinecraftServer.setInitialSpawn; no generation-spawn sample or task-history claim.");
        result.put("entity_entropy", "Unchanged GenerationSpawnProbe.entityRandom supplies explicit independent LegacyRandomSource seeds at Entity constructor RandomSource.create only.");
        result.put("instrumented", List.copyOf(GenerationSpawnProbe.instrumented));
        return result;
    }

    public static void capture(Object level) {
        try {
            if (GenerationSpawnProbe.running) throw new IllegalStateException("duplicate server bootstrap boundary");
            GenerationSpawnProbe.running = true;
            String mode = (String) GenerationSpawnProbe.config.get("mode");
            Object result = metadata(level, mode);
            Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
            Object gson = call(call(builder, "serializeNulls"), "create");
            System.out.println("CATALOG_REFERENCE=" + call(gson, "toJson", result));
            System.out.flush();
            Runtime.getRuntime().halt(0);
        } catch (Throwable failure) { GenerationSpawnProbe.fatal(failure); }
    }

    public static void gameplayTick() {
        GenerationSpawnProbe.fatal(new IllegalStateException("gameplay tick entered during catalog capture"));
    }
}
