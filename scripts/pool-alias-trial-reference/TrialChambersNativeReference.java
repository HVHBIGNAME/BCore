import java.lang.reflect.*;
import java.util.*;
import java.util.function.Predicate;
import java.util.jar.*;

/** Native trial assembly, real StructureStart placement and typed BE load/save.
 * Controlled map-world callbacks are inherited from the shared native harness.
 * Unsupported callbacks are recorded as failures, never replaced with no-ops.
 */
public class TrialChambersNativeReference extends PoolAliasTrialReference {
    static Object context(long seed, int x, int z) throws Exception {
        return make("world.level.levelgen.structure.Structure$GenerationContext", registries, generator,
            call(generator, "getBiomeSource"), null, templateManager, seed, make("world.level.ChunkPos", x, z),
            heightAccessor, (Predicate<Object>) holder -> true);
    }

    static Object trialConfig() throws Exception {
        return resource("/data/minecraft/worldgen/structure/trial_chambers.json");
    }

    static Object registryOps(Object ops) throws Exception {
        return call(type("resources.RegistryOps"), "create", ops, registries);
    }

    static Object trialStructure(Object config, boolean registered) throws Exception {
        if (registered) return call(call(registry("STRUCTURE"), "getOrThrow", key("STRUCTURE", "minecraft:trial_chambers")), "value");
        Object codec = call(field("world.level.levelgen.structure.structures.JigsawStructure", "CODEC"), "codec");
        return call(call(codec, "parse", registryOps(jsonOps()), config), "getOrThrow");
    }

    static Map<String, Object> trialAssembly(String label, long seed, int x, int z, Map<String, Object> changes) throws Exception {
        Object config = call(trialConfig(), "getAsJsonObject");
        for (Map.Entry<String, Object> change : changes.entrySet()) call(config, "add", change.getKey(), jsonTree(change.getValue()));
        Object structure = trialStructure(config, changes.isEmpty());
        Object sampleContext = context(seed, x, z);
        int y = (int) call(member(structure, "startHeight"), "sample", call(sampleContext, "random"),
            make("world.level.levelgen.WorldGenerationContext", generator, heightAccessor));
        List<Integer> origin = xyz(make("core.BlockPos", (int) call(call(sampleContext, "chunkPos"), "getMinBlockX"), y,
            (int) call(call(sampleContext, "chunkPos"), "getMinBlockZ")));
        Object nativeContext = context(seed, x, z);
        Optional<?> optional = (Optional<?>) call(structure, "findGenerationPoint", nativeContext);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("label", label); result.put("structure", "minecraft:trial_chambers");
        result.put("seed", seed); result.put("chunk", List.of(x, z)); result.put("config", config);
        result.put("first_free_height", 65); result.put("start_origin", origin);
        result.put("aliases", resolution((List<?>) call(structure, "getPoolAliases"), seed, origin));
        result.put("admitted", optional.isPresent());
        List<Object> pieces = new ArrayList<>();
        if (optional.isPresent()) {
            Object stub = optional.orElseThrow();
            result.put("generation_point", xyz(call(stub, "position")));
            Object container = call(call(stub, "getPiecesBuilder"), "build");
            for (Object piece : (List<?>) call(container, "pieces")) {
                Map<String, Object> entry = new LinkedHashMap<>();
                entry.put("pos", xyz(call(piece, "getPosition")));
                entry.put("bounds", bounds(call(piece, "getBoundingBox")));
                entry.put("rotation", call(piece, "getRotation").toString());
                entry.put("ground_level_delta", call(piece, "getGroundLevelDelta"));
                entry.put("nbt", nbt64(call(piece, "createTag", serialContext)));
                pieces.add(entry);
            }
            if (!pieces.isEmpty()) {
                Object start = make("world.level.levelgen.structure.StructureStart", structure, make("world.level.ChunkPos", x, z), 0, container);
                result.put("reference_bounds", bounds(call(start, "getBoundingBox")));
            }
        }
        result.put("pieces", pieces);
        result.put("next_i64", call(call(nativeContext, "random"), "nextLong"));
        return result;
    }

    static Map<String, Object> snapshot(Object random) throws Exception {
        List<Pos> positions = new ArrayList<>(placed.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        List<List<Integer>> states = new ArrayList<>();
        List<Object> entities = new ArrayList<>();
        for (Pos p : positions) {
            states.add(List.of(p.x(), p.y(), p.z(), stateId(placed.get(p))));
            if (blockEntities.containsKey(p)) entities.add(Map.of("pos", List.of(p.x(), p.y(), p.z()),
                "state", stateId(placed.get(p)), "nbt", nbt64(call(blockEntities.get(p), "saveWithFullMetadata", registries))));
        }
        Map<Pos, Integer> marks = new TreeMap<>(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        Method unpack = type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates", short.class, int.class, type("world.level.ChunkPos"));
        for (Object chunk : markChunks.values()) {
            Object[] sections = (Object[]) call(chunk, "getPostProcessing");
            for (int s = 0; s < sections.length; s++) if (sections[s] != null) for (Object packed : (Iterable<?>) sections[s]) {
                Pos p = Pos.from(unpack.invoke(null, packed, s - 4, call(chunk, "getPos")));
                marks.merge(p, 1, Integer::sum);
            }
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("write_count", writes.size()); result.put("writes_md5", digest(writes));
        result.put("state_count", states.size()); result.put("states_md5", digest(states));
        result.put("block_entities", entities); result.put("ticks", ticks);
        result.put("marks", marks.entrySet().stream().map(e -> List.of(e.getKey().x(), e.getKey().y(), e.getKey().z(), e.getValue())).toList());
        result.put("next_i64", call(random, "nextLong"));
        return result;
    }

    static Map<String, Object> trialPlacement(long seed, int cx, int cz, boolean entire, String environment) throws Exception {
        Object start = nativeStart("minecraft:trial_chambers", seed, 0, 0);
        terrain = environment;
        fill = state(environment.equals("water") ? "WATER" : environment.equals("air") ? "AIR" : "STONE");
        placed = new HashMap<>(); blockEntities = new HashMap<>(); writes = new ArrayList<>();
        ticks = new ArrayList<>(); markChunks = new HashMap<>();
        Object clip = entire ? call(start, "getBoundingBox") : make("world.level.levelgen.structure.BoundingBox", cx * 16, -64, cz * 16, cx * 16 + 15, 319, cz * 16 + 15);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", 918273L));
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("seed", seed); result.put("source_chunk", List.of(0, 0)); result.put("chunk", List.of(cx, cz));
        result.put("entire", entire); result.put("terrain", environment); result.put("placement_seed", 918273L);
        result.put("clip", bounds(clip));
        try {
            call(start, "placeInChunk", world(), null, generator, random, clip, make("world.level.ChunkPos", cx, cz));
            result.put("status", "complete");
        } catch (Exception error) {
            result.put("status", "unsupported"); result.put("error", errorText(error));
            java.io.StringWriter trace = new java.io.StringWriter();
            root(error).printStackTrace(new java.io.PrintWriter(trace));
            result.put("trace", trace.toString());
        }
        result.putAll(snapshot(random));
        return result;
    }

    static List<Object> blockEntityLoads(String jarPath) throws Exception {
        Set<String> templates = new TreeSet<>(), seen = new HashSet<>();
        try (JarFile jar = new JarFile(jarPath)) {
            for (JarEntry entry : Collections.list(jar.entries())) {
                String path = entry.getName();
                if (path.startsWith("data/minecraft/structure/trial_chambers/") && path.endsWith(".nbt")) {
                    templates.add("minecraft:" + path.substring("data/minecraft/structure/".length(), path.length() - 4));
                }
            }
        }
        List<Object> result = new ArrayList<>();
        Object position = make("core.BlockPos", 17, -31, -23);
        for (String name : templates) {
            for (Object palette : (List<?>) member(template(name), "palettes")) {
                for (Object info : (List<?>) call(palette, "blocks")) {
                    Object state = call(info, "state"), tag = call(info, "nbt"), block = call(state, "getBlock");
                    String blockName = call(blockRegistry, "getKey", block).toString();
                    if (tag == null || !(blockName.equals("minecraft:trial_spawner") || blockName.equals("minecraft:vault"))) continue;
                    if (!seen.add(stateId(state) + ":" + nbt64(tag))) continue;
                    Object entity = call(block, "newBlockEntity", position, state);
                    Object problems = make("util.ProblemReporter$Collector");
                    Object input = call(type("world.level.storage.TagValueInput"), "create", problems, registries, call(tag, "copy"));
                    call(entity, "loadWithComponents", input);
                    if (!(boolean) call(problems, "isEmpty")) throw new IllegalStateException(problems.toString());
                    result.add(Map.of("template", name, "local_pos", xyz(call(info, "pos")), "pos", xyz(position),
                        "state", stateId(state), "block", blockName,
                        "type_id", call(field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE"), "getId", call(entity, "getType")),
                        "input", Map.of("nbt", nbt64(tag)), "nbt", nbt64(call(entity, "saveWithFullMetadata", registries))));
                }
            }
        }
        return result;
    }

    static Map<String, Object> trialSpawnerConfigs() throws Exception {
        Map<String, Object> result = new TreeMap<>();
        Object configs = registry("TRIAL_SPAWNER_CONFIG");
        Object codec = field("world.level.block.entity.trialspawner.TrialSpawnerConfig", "DIRECT_CODEC");
        for (Object config : (Iterable<?>) configs) {
            String name = call(configs, "getKey", config).toString();
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("nbt", nbt64(call(call(codec, "encodeStart", registryOps(field("nbt.NbtOps", "INSTANCE")), config), "getOrThrow")));
            row.put("json", call(call(codec, "encodeStart", registryOps(jsonOps()), config), "getOrThrow"));
            for (String getter : List.of("spawnRange", "totalMobs", "simultaneousMobs", "totalMobsAddedPerPlayer", "simultaneousMobsAddedPerPlayer", "ticksBetweenSpawn")) {
                row.put(getter, call(config, getter));
            }
            result.put(name, row);
        }
        return result;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        List<Object> assemblies = new ArrayList<>(), placements = new ArrayList<>();
        long[][] starts = {{0, 0, 0}, {1, 0, 0}, {42, -3, 5}, {-17, 7, -11}, {918273, -1, -1},
            {13579, -418462, 366791}, {Long.MIN_VALUE, 123, -456}, {Long.MAX_VALUE, -123, 456}};
        for (long[] sample : starts) assemblies.add(trialAssembly("registered", sample[0], (int) sample[1], (int) sample[2], Map.of()));
        assemblies.add(trialAssembly("depth_zero", 42, 0, 0, Map.of("size", 0)));
        assemblies.add(trialAssembly("depth_one", 42, 0, 0, Map.of("size", 1)));
        assemblies.add(trialAssembly("distance_sixteen", 42, 0, 0, Map.of("max_distance_from_center", 16)));
        assemblies.add(trialAssembly("anisotropic_distance", 42, 0, 0, Map.of("max_distance_from_center", Map.of("horizontal", 40, "vertical", 12))));
        assemblies.add(trialAssembly("padding_rejection", 42, 0, 0, Map.of("start_height", Map.of("absolute", -60))));
        for (long seed : new long[]{0, 42}) {
            placements.add(trialPlacement(seed, 0, 0, true, "stone"));
            placements.add(trialPlacement(seed, 0, 0, true, "water"));
            placements.add(trialPlacement(seed, 0, 0, false, "stone"));
            placements.add(trialPlacement(seed, -2, 1, false, "protected"));
        }
        output("POOLALIASTRIALREFERENCE", Map.of("assembly", assemblies, "placement", placements,
            "block_entity_loads", blockEntityLoads(args[0]), "trial_spawner_configs", trialSpawnerConfigs(),
            "scope", "Native registered trial-chambers assembly on a flat generator with full configured limits, plus native codec depth/distance/padding boundary variants. Actual StructureStart.placeInChunk in controlled stone/water/protected worlds, typed trial-spawner/vault load/save and live registry codecs. No server scheduler, gameplay ticks or entity finalization. Unsupported world callbacks produce explicit failure records."));
        call(resources, "close");
    }
}
