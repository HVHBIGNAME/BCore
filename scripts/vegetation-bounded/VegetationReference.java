import java.lang.constant.*;
import java.lang.invoke.MethodHandles;
import java.lang.reflect.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.*;
import java.util.stream.Stream;

/** Native 26.1 placed-tree definitions and bounded feature-stream probes. */
public class VegetationReference extends TreeReference {
    static final List<String> ROOTS = List.of(
        "trees_plains", "trees_birch", "trees_birch_and_oak_leaf_litter",
        "birch_tall", "trees_taiga", "trees_snowy", "trees_savanna",
        "trees_jungle", "trees_sparse_jungle", "trees_windswept_hills");
    static final Map<String, Object> placedDefinitions = new TreeMap<>();
    static final Map<String, Object> configuredDefinitions = new TreeMap<>();
    static Object gson, registries, generator, biomeRegistry, placedRegistry, chunkFactory, heightAccessor;
    static final Map<String, Object> biomes = new TreeMap<>();
    static final Map<String, int[]> slots = new TreeMap<>();
    static final Map<String, String> matchingBiomes = new TreeMap<>();
    static final Map<Object, String> leafNames = new IdentityHashMap<>();
    static final Map<Object, Object> leafDelegates = new IdentityHashMap<>();
    static final Map<Object, Object> originalValues = new IdentityHashMap<>();
    static final Map<Object, Object> instrumentedValues = new IdentityHashMap<>();
    static final Map<Object, Integer> stateIds = new IdentityHashMap<>();
    static final Map<Object, Predicate<Object>> heightPredicates = new IdentityHashMap<>();
    static Capture current;

    record Terrain(String name, int floor, String soil, String cover, Map<Pos, Object> overrides) {
        static Terrain flat(String name, String soil, String cover) {
            return new Terrain(name, 64, soil, cover, Map.of());
        }
    }

    static final class StopTree extends RuntimeException {
        final String kind;
        final Pos origin;
        StopTree(String kind, Pos origin) { super(kind); this.kind = kind; this.origin = origin; }
    }

    static final class Capture {
        final Terrain terrain;
        final Object soil, cover, biome;
        final boolean seek;
        final Map<Pos, Object> writes = new HashMap<>();
        final Map<Pos, Object> chunks = new HashMap<>();
        final List<int[]> draws = new ArrayList<>();
        final List<int[]> events = new ArrayList<>();
        final List<int[]> marks = new ArrayList<>();
        final List<Map<String, Object>> leaves = new ArrayList<>();
        int placementAttempt;
        Capture(Terrain terrain, Object biome, boolean seek) throws Exception {
            this.terrain = terrain; this.biome = biome; this.seek = seek;
            soil = state(terrain.soil()); cover = state(terrain.cover());
        }
        Object block(Pos p) {
            if (p.y() < -64 || p.y() > 319) return air;
            return writes.getOrDefault(p, terrain.overrides().getOrDefault(p,
                p.y() == terrain.floor() ? soil : p.y() == terrain.floor() + 1 ? cover : air));
        }
        Object read(Object position) throws Exception {
            Pos p = Pos.from(position);
            Object block = block(p);
            event(3, p.x(), p.y(), p.z(), id(block));
            return block;
        }
        void event(int... values) { if (!seek) events.add(values); }
        int columnHeight(Object kind, int x, int z) throws Exception {
            if (kind.toString().equals("WORLD_SURFACE")) placementAttempt++;
            Predicate<Object> opaque = heightPredicates.get(kind);
            int top = Math.min(319, terrain.floor() + 1);
            for (Pos p : terrain.overrides().keySet()) if (p.x() == x && p.z() == z) top = Math.max(top, p.y());
            for (Pos p : writes.keySet()) if (p.x() == x && p.z() == z) top = Math.max(top, p.y());
            int result = -64;
            for (int y = top; y >= -64; y--) {
                if (opaque.test(block(new Pos(x, y, z)))) { result = y + 1; break; }
            }
            event(1, kind.toString().equals("OCEAN_FLOOR") ? 0 : 1, x, z, result);
            return result;
        }
        Object chunk(Pos p) throws Exception {
            Pos key = new Pos(p.x() >> 4, 0, p.z() >> 4);
            Object result = chunks.get(key);
            if (result == null) {
                Object sections = Array.newInstance(type("world.level.chunk.LevelChunkSection"), 24);
                for (int i = 0; i < 24; i++) Array.set(sections, i, make("world.level.chunk.LevelChunkSection", chunkFactory));
                result = make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", key.x(), key.z()),
                    field("world.level.chunk.UpgradeData", "EMPTY"), sections,
                    make("world.ticks.ProtoChunkTicks"), make("world.ticks.ProtoChunkTicks"), heightAccessor, chunkFactory, null);
                chunks.put(key, result);
            }
            return result;
        }
    }

    static String name(String id) { return id.replace("minecraft:", ""); }

    static Map<?, ?> json(String directory, String id) throws Exception {
        String path = "/data/minecraft/worldgen/" + directory + "/" + name(id) + ".json";
        try (var stream = VegetationReference.class.getResourceAsStream(path)) {
            String text = new String(Objects.requireNonNull(stream, path).readAllBytes(), StandardCharsets.UTF_8);
            return (Map<?, ?>) call(gson, "fromJson", text, Map.class);
        }
    }

    static void collectPlaced(Object value) throws Exception {
        Map<?, ?> doc;
        if (value instanceof String id) {
            if (placedDefinitions.containsKey(name(id))) return;
            doc = json("placed_feature", id);
            placedDefinitions.put(name(id), doc);
        } else {
            doc = (Map<?, ?>) value;
        }
        collectConfigured(doc.get("feature"));
    }

    static void collectConfigured(Object value) throws Exception {
        Map<?, ?> doc;
        if (value instanceof String id) {
            if (configuredDefinitions.containsKey(name(id))) return;
            doc = json("configured_feature", id);
            configuredDefinitions.put(name(id), doc);
        } else {
            doc = (Map<?, ?>) value;
        }
        Map<?, ?> config = (Map<?, ?>) doc.get("config");
        switch ((String) doc.get("type")) {
            case "minecraft:random_selector" -> {
                for (Object entry : (List<?>) config.get("features")) collectPlaced(((Map<?, ?>) entry).get("feature"));
                collectPlaced(config.get("default"));
            }
            case "minecraft:simple_random_selector" -> {
                for (Object entry : (List<?>) config.get("features")) collectPlaced(entry);
            }
            case "minecraft:random_boolean_selector" -> {
                collectPlaced(config.get("feature_true"));
                collectPlaced(config.get("feature_false"));
            }
        }
    }

    static Object holder(Object registry, String id) throws Exception {
        return ((Optional<?>) call(registry, "get", call(type("resources.Identifier"), "withDefaultNamespace", name(id)))).orElseThrow();
    }

    @SuppressWarnings("unchecked")
    static void bootstrap() throws Exception {
        registries = NativeWorldgenRegistries.load();
        biomeRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        placedRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "PLACED_FEATURE"));
        Object parameters = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
            field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomeRegistry);
        Object source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", parameters));
        Object settings = call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS"));
        generator = make("world.level.levelgen.NoiseBasedChunkGenerator", source, holder(settings, "overworld"));
        List<?> possible = new ArrayList<>((Set<?>) call(source, "possibleBiomes"));
        for (Object biome : possible) biomes.put(name(call(call(biome, "key"), "identifier").toString()), biome);
        Function<Object, Object> features = biome -> {
            try { return call(call(call(biome, "value"), "getGenerationSettings"), "features"); }
            catch (Exception e) { throw new RuntimeException(e); }
        };
        List<?> steps = (List<?>) call(type("world.level.biome.FeatureSorter"), "buildFeaturesPerStep", possible, features, true);
        for (String root : ROOTS) {
            Object placed = call(holder(placedRegistry, root), "value");
            for (int step = 0; step < steps.size(); step++) {
                int index = ((List<?>) call(steps.get(step), "features")).indexOf(placed);
                if (index >= 0) slots.put(root, new int[]{step, index});
            }
            for (var biome : biomes.entrySet()) {
                if ((boolean) call(call(call(biome.getValue(), "value"), "getGenerationSettings"), "hasFeature", placed)) {
                    matchingBiomes.put(root, biome.getKey());
                    break;
                }
            }
            Objects.requireNonNull(slots.get(root), root);
            Objects.requireNonNull(matchingBiomes.get(root), root);
        }
        air = state("AIR"); grass = state("GRASS_BLOCK");
        heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        Object blockStrategy = call(type("world.level.chunk.Strategy"), "createForBlockStates", field("world.level.block.Block", "BLOCK_STATE_REGISTRY"));
        Object biomeStrategy = call(type("world.level.chunk.Strategy"), "createForBiomes", call(biomeRegistry, "asHolderIdMap"));
        chunkFactory = make("world.level.chunk.PalettedContainerFactory", blockStrategy, air, null, biomeStrategy, biomes.get("plains"), null);
        for (String kind : List.of("OCEAN_FLOOR", "WORLD_SURFACE")) {
            Object heightmap = field("world.level.levelgen.Heightmap$Types", kind);
            heightPredicates.put(heightmap, (Predicate<Object>) call(heightmap, "isOpaque"));
        }
        installLeafObservers();
    }

    static Object classfileCall(String owner, Object target, String method, Object... args) throws Exception {
        for (Method m : Class.forName(owner).getMethods()) {
            if (m.getName().equals(method) && matches(m.getParameterTypes(), args)) return m.invoke(target, args);
        }
        throw new NoSuchMethodException(owner + "." + method);
    }

    // This observer delegates fallen trees unchanged. Standing trees throw at
    // their native entry boundary instead of inventing their draws or writes.
    static void installLeafObservers() throws Exception {
        ClassDesc feature = ClassDesc.of("net.minecraft.world.level.levelgen.feature.Feature");
        ClassDesc codec = ClassDesc.of("com.mojang.serialization.Codec");
        ClassDesc context = ClassDesc.of("net.minecraft.world.level.levelgen.feature.FeaturePlaceContext");
        Consumer<Object> builder = b -> {
            try {
                classfileCall("java.lang.classfile.ClassBuilder", b, "withFlags", 1);
                classfileCall("java.lang.classfile.ClassBuilder", b, "withSuperclass", feature);
                Consumer<Object> constructor = code -> {
                    try {
                        classfileCall("java.lang.classfile.CodeBuilder", code, "aload", 0);
                        classfileCall("java.lang.classfile.CodeBuilder", code, "aload", 1);
                        classfileCall("java.lang.classfile.CodeBuilder", code, "invokespecial", feature, "<init>", MethodTypeDesc.of(ConstantDescs.CD_void, codec));
                        classfileCall("java.lang.classfile.CodeBuilder", code, "return_");
                    } catch (Exception e) { throw new RuntimeException(e); }
                };
                classfileCall("java.lang.classfile.ClassBuilder", b, "withMethodBody", "<init>", MethodTypeDesc.of(ConstantDescs.CD_void, codec), 1, constructor);
                Consumer<Object> place = code -> {
                    try {
                        classfileCall("java.lang.classfile.CodeBuilder", code, "aload", 0);
                        classfileCall("java.lang.classfile.CodeBuilder", code, "aload", 1);
                        classfileCall("java.lang.classfile.CodeBuilder", code, "invokestatic", ClassDesc.of("VegetationReference"), "observeLeaf",
                            MethodTypeDesc.of(ConstantDescs.CD_boolean, ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                        classfileCall("java.lang.classfile.CodeBuilder", code, "ireturn");
                    } catch (Exception e) { throw new RuntimeException(e); }
                };
                classfileCall("java.lang.classfile.ClassBuilder", b, "withMethodBody", "place", MethodTypeDesc.of(ConstantDescs.CD_boolean, context), 1, place);
            } catch (Exception e) { throw new RuntimeException(e); }
        };
        Object classfile = classfileCall("java.lang.classfile.ClassFile", null, "of");
        byte[] bytes = (byte[]) classfileCall("java.lang.classfile.ClassFile", classfile, "build", ClassDesc.of("VegetationLeafObserver"), builder);
        Class<?> observer = MethodHandles.lookup().defineClass(bytes);
        Object registry = call(registries, "lookupOrThrow", field("core.registries.Registries", "CONFIGURED_FEATURE"));
        for (var entry : configuredDefinitions.entrySet()) {
            String kind = (String) ((Map<?, ?>) entry.getValue()).get("type");
            if (!kind.equals("minecraft:tree") && !kind.equals("minecraft:fallen_tree")) continue;
            Object reference = holder(registry, entry.getKey());
            Object configured = call(reference, "value");
            Object original = call(configured, "feature");
            Object wrapped = observer.getConstructors()[0].newInstance(field("world.level.levelgen.feature.configurations.NoneFeatureConfiguration", "CODEC"));
            leafNames.put(wrapped, entry.getKey()); leafDelegates.put(wrapped, original);
            originalValues.put(reference, configured);
            Object replacement = make("world.level.levelgen.feature.ConfiguredFeature", wrapped, call(configured, "config"));
            instrumentedValues.put(reference, replacement);
            call(reference, "bindValue", replacement);
        }
    }

    public static boolean observeLeaf(Object observer, Object context) throws Exception {
        String kind = Objects.requireNonNull(leafNames.get(observer));
        Pos origin = Pos.from(call(context, "origin"));
        current.leaves.add(Map.of("kind", kind, "origin", xyz(origin), "draw_count", current.draws.size(),
            "placement_attempt", current.placementAttempt));
        if (current.seek || !kind.startsWith("fallen_")) throw new StopTree(kind, origin);
        return (boolean) call(leafDelegates.get(observer), "place", context);
    }

    static int id(Object state) throws Exception {
        Integer result = stateIds.get(state);
        if (result == null) {
            result = (int) call(type("world.level.block.Block"), "getId", state);
            stateIds.put(state, result);
        }
        return result;
    }

    static int[] xyz(Pos p) { return new int[]{p.x(), p.y(), p.z()}; }

    @SuppressWarnings("unchecked")
    static Object world() throws Exception {
        return Proxy.newProxyInstance(VegetationReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            return switch (m.getName()) {
                case "getMinY" -> -64;
                case "getMaxY" -> 319;
                case "getHeight" -> a == null || a.length == 0 ? 384 : current.columnHeight(a[0], (int) a[1], (int) a[2]);
                case "getBiome" -> {
                    Pos pos = Pos.from(a[0]);
                    String key = call(call(current.biome, "key"), "identifier").toString();
                    current.event(2, pos.x(), pos.y(), pos.z(), key.hashCode());
                    yield current.biome;
                }
                case "getBlockState" -> current.read(a[0]);
                case "isStateAtPosition" -> ((Predicate<Object>) a[1]).test(current.read(a[0]));
                case "isFluidAtPosition" -> ((Predicate<Object>) a[1]).test(call(current.read(a[0]), "getFluidState"));
                case "ensureCanWrite" -> {
                    Pos pos = Pos.from(a[0]);
                    boolean allowed = !current.terrain.name().equals("deny_origin");
                    current.event(4, pos.x(), pos.y(), pos.z(), allowed ? 1 : 0);
                    yield allowed;
                }
                case "setBlock" -> {
                    Pos pos = Pos.from(a[0]);
                    boolean allowed = pos.y() >= -64 && pos.y() <= 319;
                    current.event(5, pos.x(), pos.y(), pos.z(), id(a[1]), (int) a[2], allowed ? 1 : 0);
                    if (allowed) {
                        current.writes.put(pos, a[1]);
                        if (((int) a[2] & 16) == 0) {
                            Object mark = call(a[1], "getPostProcessPos", p, a[0]);
                            if (mark != null) throw new IllegalStateException("Unmodelled automatic postprocessing: " + a[1]);
                        }
                    }
                    yield allowed;
                }
                case "getChunk" -> {
                    if (a.length != 1) throw new UnsupportedOperationException(m.toString());
                    Pos pos = Pos.from(a[0]);
                    current.event(6, pos.x(), pos.y(), pos.z());
                    if (pos.y() >= -64 && pos.y() <= 319) current.marks.add(xyz(pos));
                    yield current.chunk(pos);
                }
                default -> throw new UnsupportedOperationException(m.toString());
            };
        });
    }

    static Object tracedRandom(Object random) throws Exception {
        if (current.seek) return random;
        return Proxy.newProxyInstance(VegetationReference.class.getClassLoader(), new Class<?>[]{type("util.RandomSource")}, (p, m, a) -> {
            Object value = m.invoke(random, a);
            switch (m.getName()) {
                case "nextInt" -> current.draws.add(new int[]{0, (int) a[0], (int) value});
                case "nextFloat" -> current.draws.add(new int[]{1, 0, Float.floatToRawIntBits((float) value)});
                default -> throw new UnsupportedOperationException(m.toString());
            }
            return value;
        });
    }

    static Map<String, Object> run(String root, long seed, int cx, int cz, boolean advance) throws Exception {
        current.placementAttempt = 0;
        int[] slot = slots.get(root);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        long decorationSeed = (long) call(random, "setDecorationSeed", seed, cx * 16, cz * 16);
        call(random, "setFeatureSeed", decorationSeed, slot[1], slot[0]);
        if (advance) { call(random, "nextInt", 17); call(random, "nextFloat"); call(random, "nextInt", 1073741825); }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("source", new int[]{cx, cz}); result.put("decoration_seed", decorationSeed);
        result.put("slot", slot); result.put("advance", advance);
        Object placed = call(holder(placedRegistry, root), "value");
        try {
            result.put("placed", call(placed, "placeWithBiomeCheck", world(), generator, tracedRandom(random), make("core.BlockPos", cx * 16, -64, cz * 16)));
        } catch (Exception error) {
            Throwable cause = error;
            while (cause.getCause() != null) cause = cause.getCause();
            if (!(cause instanceof StopTree stopped)) throw error;
            result.put("blocked", Map.of("kind", stopped.kind, "origin", xyz(stopped.origin)));
        }
        if (!current.seek) result.put("next_i64", call(random, "nextLong"));
        return result;
    }

    static String digest(List<int[]> rows) throws Exception {
        MessageDigest digest = MessageDigest.getInstance("MD5");
        for (int[] row : rows) {
            ByteBuffer buffer = ByteBuffer.allocate(row.length * 4).order(ByteOrder.LITTLE_ENDIAN);
            for (int value : row) buffer.putInt(value);
            digest.update(buffer.array());
        }
        return HexFormat.of().formatHex(digest.digest());
    }

    static List<int[]> states(Map<Pos, Object> blocks) throws Exception {
        List<Pos> positions = new ArrayList<>(blocks.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        List<int[]> result = new ArrayList<>();
        for (Pos p : positions) result.add(new int[]{p.x(), p.y(), p.z(), id(blocks.get(p))});
        return result;
    }

    static void verifyWithoutObservers(String root, long seed, List<int[]> sources, boolean advance, List<Object> expectedStreams) throws Exception {
        Capture expected = current;
        try {
            for (var entry : originalValues.entrySet()) call(entry.getKey(), "bindValue", entry.getValue());
            current = new Capture(expected.terrain, expected.biome, false);
            List<Object> streams = new ArrayList<>();
            for (int[] source : sources) streams.add(run(root, seed, source[0], source[1], advance));
            List<Object> actual = List.of(streams, current.draws, digest(current.events), states(current.writes), current.marks);
            List<Object> reference = List.of(expectedStreams, expected.draws, digest(expected.events), states(expected.writes), expected.marks);
            if (!call(gson, "toJson", actual).equals(call(gson, "toJson", reference))) {
                throw new IllegalStateException("Leaf observer changed native output for " + root + " seed=" + seed);
            }
        } finally {
            for (var entry : instrumentedValues.entrySet()) call(entry.getKey(), "bindValue", entry.getValue());
            current = expected;
        }
    }

    static Map<String, Object> sample(String root, long seed, List<int[]> sources, Terrain terrain, String biome, boolean advance) throws Exception {
        current = new Capture(terrain, biomes.get(biome), false);
        List<Object> streams = new ArrayList<>();
        for (int[] source : sources) {
            Map<String, Object> stream = run(root, seed, source[0], source[1], advance);
            streams.add(stream);
            if (stream.containsKey("blocked")) break;
        }
        if (streams.stream().noneMatch(s -> ((Map<?, ?>) s).containsKey("blocked"))) {
            verifyWithoutObservers(root, seed, sources, advance, streams);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("root", root); result.put("seed", seed); result.put("biome", biome);
        result.put("biome_id", call(biomeRegistry, "getId", call(current.biome, "value")));
        result.put("terrain", terrain.name()); result.put("floor_y", terrain.floor());
        result.put("soil", id(current.soil)); result.put("cover", id(current.cover));
        result.put("initial_blocks", states(terrain.overrides())); result.put("sources", sources);
        result.put("streams", streams); result.put("leaves", current.leaves);
        result.put("draws", current.draws); result.put("events_md5", digest(current.events));
        result.put("event_prefix", current.events.subList(0, Math.min(8, current.events.size())));
        result.put("event_count", current.events.size()); result.put("writes", states(current.writes));
        result.put("postprocessing", current.marks);
        return result;
    }

    static List<Object> samples() throws Exception {
        List<Object> samples = new ArrayList<>();
        List<Terrain> terrains = List.of(Terrain.flat("grass", "GRASS_BLOCK", "AIR"),
            Terrain.flat("stone", "STONE", "AIR"), Terrain.flat("water", "GRASS_BLOCK", "WATER"),
            Terrain.flat("empty", "AIR", "AIR"), Terrain.flat("grass_cover", "GRASS_BLOCK", "SHORT_GRASS"),
            Terrain.flat("deny_origin", "GRASS_BLOCK", "AIR"));
        long[] seeds = {0, 1, 17, 42, -1, Long.MIN_VALUE, Long.MAX_VALUE};
        for (String root : ROOTS) {
            for (long seed : seeds) {
                for (Terrain terrain : terrains) samples.add(sample(root, seed, List.of(new int[]{-2, 3}), terrain, matchingBiomes.get(root), false));
            }
            samples.add(sample(root, 42, List.of(new int[]{0, 0}), terrains.get(0), "desert", false));
            samples.add(sample(root, 42, List.of(new int[]{0, 0}), terrains.get(0), matchingBiomes.get(root), true));
        }
        // Find real native streams entering each reachable fallen variant. The
        // stone plane + one grass support then lets all other attempts run and
        // fail their native predicates without skipping their selector draws.
        for (String root : ROOTS) {
            Set<String> wanted = new TreeSet<>();
            Object placed = call(holder(placedRegistry, root), "value");
            try (Stream<?> features = (Stream<?>) call(placed, "getFeatures")) {
                for (Object h : features.toList()) {
                    Object key = ((Optional<?>) call(h, "unwrapKey")).orElseThrow();
                    String kind = name(call(key, "identifier").toString());
                    Map<?, ?> definition = (Map<?, ?>) configuredDefinitions.get(kind);
                    if (Set.of("minecraft:tree", "minecraft:fallen_tree").contains(definition.get("type"))) wanted.add(kind);
                }
            }
            for (long seed = 0; seed < 65536 && !wanted.isEmpty(); seed++) {
                current = new Capture(terrains.get(0), biomes.get(matchingBiomes.get(root)), true);
                run(root, seed, 0, 0, false);
                if (current.leaves.isEmpty()) continue;
                Map<String, Object> leaf = current.leaves.get(0);
                String kind = (String) leaf.get("kind");
                if (!wanted.contains(kind)) continue;
                if (!kind.startsWith("fallen_")) {
                    samples.add(sample(root, seed, List.of(new int[]{0, 0}), terrains.get(0), matchingBiomes.get(root), false));
                    wanted.remove(kind);
                    continue;
                }
                int[] origin = (int[]) leaf.get("origin");
                Terrain planted = new Terrain("single_support", 64, "STONE", "AIR", Map.of(new Pos(origin[0], 64, origin[2]), grass));
                Map<String, Object> full = sample(root, seed, List.of(new int[]{0, 0}), planted, matchingBiomes.get(root), false);
                if (((List<?>) full.get("streams")).stream().anyMatch(s -> ((Map<?, ?>) s).containsKey("blocked"))) continue;
                samples.add(full);
                samples.add(sample(root, seed, List.of(new int[]{0, 0}), terrains.get(0), matchingBiomes.get(root), false));
                List<int[]> sources = new ArrayList<>();
                for (int x = -1; x <= 1; x++) for (int z = -1; z <= 1; z++) sources.add(new int[]{x, z});
                samples.add(sample(root, seed, sources, planted, matchingBiomes.get(root), false));
                wanted.remove(kind);
            }
            if (!wanted.isEmpty()) throw new IllegalStateException("No native stream found for " + root + ": " + wanted);
        }
        samples.addAll(delayedIncomingSamples());
        return samples;
    }

    static List<Object> delayedIncomingSamples() throws Exception {
        String root = "trees_birch_and_oak_leaf_litter";
        Map<Pos, Object> edge = new HashMap<>();
        for (int x = 0; x < 16; x++) for (int z = 0; z < 16; z++) {
            if (x == 0 || x == 15 || z == 0 || z == 15) edge.put(new Pos(x, 64, z), grass);
        }
        Terrain searchTerrain = new Terrain("edge_support", 64, "STONE", "AIR", edge);
        for (long seed = 0; seed < 65536; seed++) {
            current = new Capture(searchTerrain, biomes.get(matchingBiomes.get(root)), true);
            run(root, seed, 0, 0, false);
            if (current.leaves.isEmpty()) continue;
            Map<String, Object> leaf = current.leaves.get(0);
            if (!((String) leaf.get("kind")).startsWith("fallen_") || (int) leaf.get("placement_attempt") < 3) continue;
            int[] origin = (int[]) leaf.get("origin");
            Terrain planted = new Terrain("delayed_single_support", 64, "STONE", "AIR", Map.of(new Pos(origin[0], 64, origin[2]), grass));
            Map<String, Object> full = sample(root, seed, List.of(new int[]{0, 0}), planted, matchingBiomes.get(root), false);
            if (((List<?>) full.get("streams")).stream().anyMatch(s -> ((Map<?, ?>) s).containsKey("blocked"))) continue;
            boolean spills = current.writes.keySet().stream().anyMatch(p -> (p.x() >> 4) != 0 || (p.z() >> 4) != 0);
            if (!spills) continue;
            List<int[]> sources = new ArrayList<>();
            for (int x = -1; x <= 1; x++) for (int z = -1; z <= 1; z++) sources.add(new int[]{x, z});
            return List.of(full, sample(root, seed, sources, planted, matchingBiomes.get(root), false));
        }
        throw new IllegalStateException("No delayed incoming native fallen-tree stream found");
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        for (String root : ROOTS) collectPlaced(root);
        bootstrap();
        System.out.println("VEGETATION_REFERENCE=" + call(gson, "toJson", Map.of(
            "placed_features", placedDefinitions, "configured_features", configuredDefinitions,
            "roots", ROOTS, "samples", samples(), "scope", Map.of(
                "placement", "Native registered PlacedFeature.placeWithBiomeCheck; modifiers/selectors and FallenTreeFeature execute unchanged",
                "standing", "Stop before the first standing TreeFeature body; blocked records are stream prefixes, not complete vegetation",
                "observer_check", "Every completed sample is rerun with the original unmodified configured-feature holders and must match streams, RNG draws, world events, writes and postprocessing",
                "terrain", "Explicit single-layer synthetic soil, cover and overrides; live height predicates and real registry biome membership",
                "biome_trace", "Uniform biomes matched by resource-key name; biome events encode Java String.hashCode of that key, with native biome_id recorded separately",
                "region", "Finite caller-supplied source list in x/z order; complete streams until a recorded blocker, no ChunkPyramid scheduling claim",
                "writes", "Unbounded horizontal storage, air/rejected writes outside [-64,319]; postprocessing recorded without ticks",
                "rng", "One native WorldgenRandom per source; native decoration/feature seeds; optional prior draws; no per-attempt reseeding"))));
    }
}
