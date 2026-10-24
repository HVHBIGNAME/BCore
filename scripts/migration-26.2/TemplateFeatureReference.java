import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;
import java.util.function.Function;

/** Calls native configured features with the preserved jigsaw map-world observer. */
public final class TemplateFeatureReference extends JigsawReference {
    static Object gson, nativeLevel;
    static Map<?, ?> specification;
    static List<Object> writeCalls, neighbourUpdates, implicitMarks;
    static Map<Pos, Object> blockTickContainers, fluidTickContainers;
    static Set<String> operations;
    static boolean rejectWrites;

    static void initialize(String config) throws Exception {
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        specification = (Map<?, ?>) call(gson, "fromJson", Files.readString(Path.of(config)), Map.class);
    }

    static Object resource(String path) throws Exception {
        try (var stream = TemplateFeatureReference.class.getResourceAsStream(path)) {
            return call(gson, "fromJson", new String(Objects.requireNonNull(stream, path).readAllBytes(), StandardCharsets.UTF_8), Map.class);
        }
    }

    static Object codecOps() throws Exception {
        return call(type("resources.RegistryOps"), "create",
            Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), registries);
    }

    static Object decode(Object document) throws Exception {
        Object json = call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", document));
        return call(call(field("world.level.levelgen.feature.ConfiguredFeature", "DIRECT_CODEC"), "parse", codecOps(), json), "getOrThrow");
    }

    static Object canonical(Object feature) throws Exception {
        Object encoded = call(call(field("world.level.levelgen.feature.ConfiguredFeature", "DIRECT_CODEC"),
            "encodeStart", codecOps(), feature), "getOrThrow");
        return call(gson, "fromJson", encoded.toString(), Map.class);
    }

    static Map<String, Object> document(List<?> entries) {
        return Map.of("type", "minecraft:template", "config", Map.of("templates", entries));
    }

    static Map<String, Object> entry(String name, Object rotations, Object weight) {
        Map<String, Object> data = new LinkedHashMap<>();
        data.put("id", name);
        if (rotations != null) data.put("rotations", rotations);
        return Map.of("data", data, "weight", weight);
    }

    static Map<String, Object> admission(String name, Object document) {
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("name", name); result.put("document", document);
        try {
            result.put("canonical_document", canonical(decode(document)));
            result.put("admitted", true);
        } catch (Exception failure) {
            Throwable cause = failure;
            while (cause.getCause() != null) cause = cause.getCause();
            result.put("admitted", false);
            result.put("error", cause.getClass().getName() + ": " + cause.getMessage());
        }
        return result;
    }

    static List<Object> admissions() {
        String name = "minecraft:spring/sulfur_spring_small_1";
        List<Object> result = new ArrayList<>();
        result.add(admission("default_rotations", document(List.of(entry(name, null, 1)))));
        result.add(admission("empty_templates", document(List.of())));
        result.add(admission("missing_templates", Map.of("type", "minecraft:template", "config", Map.of())));
        result.add(admission("missing_id", document(List.of(Map.of("data", Map.of(), "weight", 1)))));
        for (Object rotations : List.of(List.of(), List.of("none", "none"), List.of("unknown"), false))
            result.add(admission("rotations_" + rotations, document(List.of(entry(name, rotations, 1)))));
        for (Object weight : List.of(0, -1, 1.9, true))
            result.add(admission("weight_" + weight, document(List.of(entry(name, null, weight)))));
        return result;
    }

    static Object tickAccess(boolean fluid) throws Exception {
        Map<Pos, Object> containers = fluid ? fluidTickContainers : blockTickContainers;
        return make("world.ticks.WorldGenTickAccess", (Function<Object, Object>) position -> {
            try {
                Pos pos = Pos.from(position);
                Pos chunk = new Pos(pos.x() >> 4, 0, pos.z() >> 4);
                if (!containers.containsKey(chunk)) containers.put(chunk, make("world.ticks.ProtoChunkTicks"));
                return containers.get(chunk);
            } catch (Exception failure) { throw new RuntimeException(failure); }
        });
    }

    static Object observedWorld(Object random) throws Exception {
        Object delegate = world();
        Object blockTicks = tickAccess(false), fluidTicks = tickAccess(true);
        return Proxy.newProxyInstance(TemplateFeatureReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.WorldGenLevel")}, (proxy, method, arguments) -> {
                operations.add(method.getName());
                try {
                    return switch (method.getName()) {
                        case "getLevel" -> nativeLevel;
                        case "getBiome" -> call(registry("BIOME"), "getOrThrow", key("BIOME", "minecraft:plains"));
                        case "getSeaLevel" -> 63;
                        case "getRandom" -> random;
                        case "getBlockTicks" -> blockTicks;
                        case "getFluidTicks" -> fluidTicks;
                        case "blockUpdated", "updateNeighborsAt" -> {
                            neighbourUpdates.add(List.of(method.getName(), xyz(arguments[0]), stateId(call(arguments[1], "defaultBlockState"))));
                            yield null;
                        }
                        case "setBlock" -> {
                            boolean accepted = !rejectWrites && (boolean) method.invoke(delegate, arguments);
                            List<Integer> pos = xyz(arguments[0]);
                            writeCalls.add(List.of(pos.get(0), pos.get(1), pos.get(2), stateId(arguments[1]), arguments[2], accepted ? 1 : 0));
                            if (accepted && ((int) arguments[2] & 16) == 0) {
                                Object mark = call(arguments[1], "getPostProcessPos", proxy, arguments[0]);
                                if (mark != null) implicitMarks.add(xyz(mark));
                            }
                            yield accepted;
                        }
                        case "scheduleTick" -> {
                            boolean fluid = type("world.level.material.Fluid").isInstance(arguments[1]);
                            int target = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", arguments[1])
                                : stateId(call(arguments[1], "defaultBlockState"));
                            List<Integer> pos = xyz(arguments[0]);
                            ticks.add(List.of(pos.get(0), pos.get(1), pos.get(2), target, arguments[2], fluid ? 1 : 0));
                            call(fluid ? fluidTicks : blockTicks, "schedule",
                                make("world.ticks.ScheduledTick", arguments[1], arguments[0], (long) (int) arguments[2], (long) ticks.size()));
                            yield null;
                        }
                        default -> method.invoke(delegate, arguments);
                    };
                } catch (InvocationTargetException failure) { throw failure.getCause(); }
            });
    }

    static List<Object> explicitSectionMarks() throws Exception {
        Map<Pos, Object> chunks = new TreeMap<>(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::z));
        chunks.putAll(markChunks);
        Method unpack = type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates", short.class, int.class, type("world.level.ChunkPos"));
        List<Object> result = new ArrayList<>();
        for (Object chunk : chunks.values()) {
            Object[] sections = (Object[]) call(chunk, "getPostProcessing");
            for (int section = 0; section < sections.length; section++) {
                if (sections[section] == null) continue;
                for (Object packed : (Iterable<?>) sections[section]) {
                    Object pos = unpack.invoke(null, packed, section - 4, call(chunk, "getPos"));
                    result.add(xyz(pos));
                }
            }
        }
        return result;
    }

    static Map<String, Object> sample(String name, Object document, long seed, String mode, int y) throws Exception {
        terrain = mode;
        fill = state(mode.equals("water") ? "WATER" : mode.equals("air") ? "AIR" : "STONE");
        rejectWrites = mode.equals("reject_writes");
        placed = new HashMap<>(); blockEntities = new HashMap<>(); writes = new ArrayList<>();
        ticks = new ArrayList<>(); markChunks = new HashMap<>(); writeCalls = new ArrayList<>();
        neighbourUpdates = new ArrayList<>(); implicitMarks = new ArrayList<>(); operations = new TreeSet<>();
        blockTickContainers = new HashMap<>(); fluidTickContainers = new HashMap<>();
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        Object origin = make("core.BlockPos", 15, y, -17);
        boolean result = (boolean) call(decode(document), "place", observedWorld(random), generator, random, origin);
        List<Pos> positions = new ArrayList<>(placed.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        List<Object> finalStates = new ArrayList<>(), entities = new ArrayList<>();
        for (Pos pos : positions) {
            finalStates.add(List.of(pos.x(), pos.y(), pos.z(), stateId(placed.get(pos))));
            if (blockEntities.containsKey(pos)) {
                Object entity = blockEntities.get(pos);
                entities.add(Map.of("pos", List.of(pos.x(), pos.y(), pos.z()), "state", stateId(placed.get(pos)),
                    "full", Map.of("nbt", nbt64(call(entity, "saveWithFullMetadata", registries))),
                    "update", Map.of("nbt", nbt64(call(entity, "getUpdateTag", registries)))));
            }
        }
        Map<String, Object> record = new LinkedHashMap<>();
        record.put("name", name); record.put("document", document); record.put("seed", seed);
        record.put("terrain", mode); record.put("fill", stateId(fill)); record.put("origin", xyz(origin));
        record.put("placed", result); record.put("write_calls", writeCalls); record.put("states", finalStates);
        record.put("ticks", ticks); record.put("neighbour_updates", neighbourUpdates); record.put("block_entities", entities);
        record.put("implicit_marks", implicitMarks); record.put("rng_count", call(random, "getCount"));
        record.put("explicit_section_marks", explicitSectionMarks());
        record.put("operations", operations); record.put("next_i64", call(random, "nextLong"));
        return record;
    }

    static List<Object> samples() throws Exception {
        List<Object> result = new ArrayList<>();
        for (String suffix : List.of("small_1", "small_2", "small_3", "small_4", "medium_1", "medium_2", "medium_3", "large_1", "large_2", "extra_large_1")) {
            String name = "minecraft:spring/sulfur_spring_" + suffix;
            for (Object rotationValue : type("world.level.block.Rotation").getEnumConstants()) {
                String rotation = (String) call(rotationValue, "getSerializedName");
                for (String mode : List.of("stone", "water", "reject_writes"))
                    result.add(sample(name + "/" + rotation, document(List.of(entry(name, List.of(rotation), 1))), 17, mode, 64));
            }
        }
        for (long seed : new long[]{0, 1, 17, 42})
            for (String mode : List.of("stone", "slope", "water", "reject_writes"))
                result.add(sample("sulfur_spring", resource("/data/minecraft/worldgen/configured_feature/sulfur_spring.json"), seed, mode, 65));
        for (long seed : new long[]{0, 1, 42})
            for (String mode : List.of("stone", "water", "air"))
                result.add(sample("sulfur_pool", resource("/data/minecraft/worldgen/configured_feature/sulfur_pool.json"), seed, mode, 64));
        for (long seed : new long[]{0, 42})
            for (String mode : List.of("slope", "stone"))
                result.add(sample("rooted_sulfur_spring", resource("/data/minecraft/worldgen/configured_feature/rooted_sulfur_spring.json"), seed, mode, 60));
        for (int y : new int[]{-65, -64, 319, 320})
            result.add(sample("build_boundary", document(List.of(entry("minecraft:spring/sulfur_spring_large_1", null, 1))), 42, "stone", y));
        return result;
    }

    static List<Object> potentLoads() throws Exception {
        List<Object> result = new ArrayList<>();
        Object block = field("world.level.block.Blocks", "POTENT_SULFUR");
        List<String> inputs = List.of("{}", "{countdown:-1}", "{countdown:20}", "{countdown:7b}", "{countdown:9s}",
            "{countdown:4294967297L}", "{countdown:3.75f}", "{countdown:-1.5d}", "{countdown:'bad'}",
            "{countdown:[]}", "{countdown:42,eruptionTick:123L,unknown:7,components:{}}");
        for (Object state : (Iterable<?>) call(call(block, "getStateDefinition"), "getPossibleStates")) {
            for (String snbt : inputs) {
                Object load = call(type("nbt.TagParser"), "parseCompoundFully", snbt);
                Object pos = make("core.BlockPos", -17, 21, 128);
                Object entity = call(block, "newBlockEntity", pos, state);
                Object problems = make("util.ProblemReporter$Collector");
                Object input = call(type("world.level.storage.TagValueInput"), "create", problems, registries, call(load, "copy"));
                call(entity, "loadWithComponents", input);
                result.add(Map.of("state", stateId(state), "pos", xyz(pos), "snbt", snbt,
                    "load", Map.of("nbt", nbt64(load)), "load_has_problems", !(boolean) call(problems, "isEmpty"),
                    "full", Map.of("nbt", nbt64(call(entity, "saveWithFullMetadata", registries))),
                    "update", Map.of("nbt", nbt64(call(entity, "getUpdateTag", registries)))));
            }
        }
        return result;
    }

    public static void capture(Object level) {
        try {
            nativeLevel = level;
            registries = call(level, "registryAccess");
            blockRegistry = field("core.registries.BuiltInRegistries", "BLOCK");
            templateManager = call(call(level, "getServer"), "getStructureManager");
            if (!specification.get("world_seed").equals(Long.toString((long) call(level, "getSeed"))))
                throw new IllegalStateException("native template world seed differs");
            includeBuiltInRegistries();
            generator = call(call(level, "getChunkSource"), "getGenerator");
            heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
            output("TEMPLATE_FEATURE_REFERENCE", Map.of("native_version", resource("/version.json"),
                "samples", samples(), "admission", admissions(), "potent_sulfur_loads", potentLoads(),
                "gameplay_ticks", 0, "manager_source", "initialized-server-structure-manager",
                "scope", "Native template/sequence/weighted/root/pool features in fresh controlled map worlds; initialized server only supplies registries and its manager/data fixer. No live-world feature writes or gameplay ticks."));
            System.out.flush();
            System.err.flush();
            Runtime.getRuntime().halt(0);
        } catch (Throwable failure) { fatal(failure); }
    }

    public static void unexpectedTick() {
        fatal(new IllegalStateException("unexpected gameplay tick before template capture"));
    }

    public static void fatal(Throwable failure) {
        failure.printStackTrace(System.err);
        Runtime.getRuntime().halt(2);
    }
}
