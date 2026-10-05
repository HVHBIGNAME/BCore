import java.lang.reflect.*;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Function;
import java.util.function.Predicate;

/** Actual 26.1 GeodeFeature/WorldGenRegion/ProtoChunk differential oracle.
 * No geode algorithm is implemented here. The recording proxy delegates block
 * storage, radius guards, postprocessing and tick retention to native methods.
 * The server constructor is bypassed using existing read-only probe helpers;
 * the unrelated server POI callback is record-only. No scheduled tick is run.
 */
public class GeodeReference extends TreeReference {
    static final String REGION = "server.level.WorldGenRegion";
    static final String STATUS = "world.level.chunk.status.ChunkStatus";
    static final Comparator<Pos> ORDER = Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z);
    static Object registries, codecRegistries, gson, factory, height, dimension, serverLevel, jsonOps;
    static Object generator, inTag, cannotReplace, invalidBlocks;
    static Method sectionSet, idMethod, blockRead, fluidRead;
    static final Map<String, Object> stateCache = new HashMap<>();
    static final Map<Object, Object> factoryCache = new IdentityHashMap<>();
    static final Map<String, Object> documents = new LinkedHashMap<>();
    static final Map<String, Object> nativeConfigs = new HashMap<>();

    static int id(Object s) throws Exception { return (int) idMethod.invoke(null, s); }
    static Object block(String name) throws Exception {
        if (!stateCache.containsKey(name)) stateCache.put(name, state(name));
        return stateCache.get(name);
    }
    static Object at(Pos p) throws Exception { return make("core.BlockPos", p.x(), p.y(), p.z()); }
    static int[] xyz(Pos p) { return new int[]{p.x(), p.y(), p.z()}; }
    static Object member(Object object, String name) throws Exception {
        for (Class<?> c = object.getClass(); c != null; c = c.getSuperclass()) {
            try { Field f = c.getDeclaredField(name); f.setAccessible(true); return f.get(object); }
            catch (NoSuchFieldException ignored) { }
        }
        throw new NoSuchFieldException(name);
    }
    static Object holder(Object registry, String name) throws Exception {
        return ((Optional<?>) call(registry, "get", call(type("resources.Identifier"), "withDefaultNamespace", name))).orElseThrow();
    }
    static Object resource(String path) throws Exception {
        try (var stream = GeodeReference.class.getResourceAsStream("/data/minecraft/" + path + ".json")) {
            return call(gson, "fromJson", new String(Objects.requireNonNull(stream, path).readAllBytes(), StandardCharsets.UTF_8), Map.class);
        }
    }
    static Object decode(Object document) throws Exception {
        Object ops = call(type("resources.RegistryOps"), "create", jsonOps, codecRegistries);
        Object json = call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", document));
        return call(call(field("world.level.levelgen.feature.ConfiguredFeature", "DIRECT_CODEC"), "parse", ops, json), "getOrThrow");
    }
    @SuppressWarnings("unchecked")
    static Map<String, Object> copy(Object value) throws Exception {
        return (Map<String, Object>) call(gson, "fromJson", call(gson, "toJson", value), Map.class);
    }
    @SuppressWarnings("unchecked")
    static Map<String, Object> config(Object document) { return (Map<String, Object>) ((Map<?, ?>) document).get("config"); }
    @SuppressWarnings("unchecked")
    static Map<String, Object> group(Map<String, Object> config, String key) { return (Map<String, Object>) config.get(key); }
    static Object provider(String name) { return Map.of("type", "minecraft:simple_state_provider", "state", Map.of("Name", "minecraft:" + name)); }
    static Object stateDoc(String name, Map<String, String> properties) { return Map.of("Name", "minecraft:" + name, "Properties", properties); }
    static Object uniform(int min, int max) { return Map.of("type", "minecraft:uniform", "min_inclusive", min, "max_inclusive", max); }
    static Object factoryFor(Object state) throws Exception {
        if (!factoryCache.containsKey(state)) {
            factoryCache.put(state, make("world.level.chunk.PalettedContainerFactory",
                member(factory, "blockStatesStrategy"), state, member(factory, "blockStatesContainerCodec"),
                member(factory, "biomeStrategy"), member(factory, "defaultBiome"), member(factory, "biomeContainerCodec")));
        }
        return factoryCache.get(state);
    }

    static void bootstrap() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        call(builder, "serializeNulls");
        call(builder, "setObjectToNumberStrategy", Class.forName("com.google.gson.ToNumberPolicy").getField("LONG_OR_DOUBLE").get(null));
        gson = call(builder, "create");
        registries = NativeWorldgenRegistries.load();
        Object builtins = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", field("core.registries.BuiltInRegistries", "REGISTRY"));
        codecRegistries = call(type("core.HolderLookup$Provider"), "create", java.util.stream.Stream.concat(
            (java.util.stream.Stream<?>) call(builtins, "listRegistries"), (java.util.stream.Stream<?>) call(registries, "listRegistries")));
        jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        factory = call(type("world.level.chunk.PalettedContainerFactory"), "create", registries);
        height = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        Object dimensions = call(registries, "lookupOrThrow", field("core.registries.Registries", "DIMENSION_TYPE"));
        dimension = call(call(dimensions, "getOrThrow", field("world.level.dimension.BuiltinDimensionTypes", "OVERWORLD")), "value");
        serverLevel = TreeEffectReference.probeLevel();
        Object biomes = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        Object settings = call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS"));
        generator = make("world.level.levelgen.NoiseBasedChunkGenerator", make("world.level.biome.FixedBiomeSource", holder(biomes, "plains")), holder(settings, "overworld"));
        sectionSet = type("world.level.chunk.LevelChunkSection").getMethod("setBlockState", int.class, int.class, int.class, type("world.level.block.state.BlockState"), boolean.class);
        idMethod = type("world.level.block.Block").getMethod("getId", type("world.level.block.state.BlockState"));
        blockRead = type(REGION).getMethod("getBlockState", type("core.BlockPos"));
        fluidRead = type(REGION).getMethod("getFluidState", type("core.BlockPos"));
        cannotReplace = field("tags.BlockTags", "FEATURES_CANNOT_REPLACE");
        invalidBlocks = field("tags.BlockTags", "GEODE_INVALID_BLOCKS");
        inTag = type("world.level.block.state.BlockBehaviour$BlockStateBase").getMethod("is", type("tags.TagKey"));
        documents.put("amethyst_geode", resource("worldgen/configured_feature/amethyst_geode"));
    }

    record Setup(Pos origin, Object background, Map<Pos, Object> initial, int radius, int readRadius, boolean reject, boolean advance, int repetitions, boolean reseed) {
        static Setup stone(Pos origin) throws Exception { return new Setup(origin, block("STONE"), Map.of(), 1, 2, false, false, 1, false); }
        Setup background(Object state) { return new Setup(origin, state, initial, radius, readRadius, reject, advance, repetitions, reseed); }
        Setup initial(Map<Pos, Object> states) { return new Setup(origin, background, states, radius, readRadius, reject, advance, repetitions, reseed); }
        Setup radius(int value) { return new Setup(origin, background, initial, value, readRadius, reject, advance, repetitions, reseed); }
        Setup reads(int value) { return new Setup(origin, background, initial, radius, value, reject, advance, repetitions, reseed); }
        Setup rejectWrites() { return new Setup(origin, background, initial, radius, readRadius, true, advance, repetitions, reseed); }
        Setup repeat(boolean advance, int count, boolean reseed) { return new Setup(origin, background, initial, radius, readRadius, reject, advance, count, reseed); }
    }

    static class World {
        final Setup setup;
        final Object region, proxy;
        final Map<Pos, Object> chunks = new TreeMap<>(ORDER);
        final List<int[]> writes = new ArrayList<>(), ticks = new ArrayList<>(), origins = new ArrayList<>();
        final Set<Pos> touched = new TreeSet<>(ORDER);
        final MessageDigest readHash = MessageDigest.getInstance("SHA-256");
        final List<int[]> firstReads = new ArrayList<>();
        final Set<String> operations = new TreeSet<>();
        int readCount, seedQueries;
        Pos failedRead;

        World(Setup setup, long worldSeed) throws Exception {
            this.setup = setup;
            Object carvers = field(STATUS, "CARVERS");
            Object initializer = Proxy.newProxyInstance(GeodeReference.class.getClassLoader(), new Class<?>[]{type("util.StaticCache2D$Initializer")}, (p, m, a) -> {
                if (!m.getName().equals("get")) throw new UnsupportedOperationException(m.toString());
                int cx = (int) a[0], cz = (int) a[1];
                Object chunk = createChunk(cx, cz);
                call(chunk, "setPersistedStatus", carvers);
                Object owner = call(chunk, "getPos");
                Object holder = make("server.level.ChunkHolder", owner, 0, height, null, null, null);
                call(holder, "completeFuture", carvers, chunk);
                chunks.put(new Pos(cx, 0, cz), chunk);
                return holder;
            });
            Object cache = call(type("util.StaticCache2D"), "create", setup.origin.x() >> 4, setup.origin.z() >> 4, setup.readRadius, initializer);
            Object nativeStep = call(field("world.level.chunk.status.ChunkPyramid", "GENERATION_PYRAMID"), "getStepTo", field(STATUS, "FEATURES"));
            // Explicit CARVERS dependencies permit independent cross-chunk input;
            // native ensureCanWrite still enforces the requested write radius.
            Object dependencies = make("world.level.chunk.status.ChunkDependencies", call(Class.forName("com.google.common.collect.ImmutableList"), "copyOf", List.of(carvers, carvers, carvers)));
            Object step = make("world.level.chunk.status.ChunkStep", field(STATUS, "FEATURES"), dependencies,
                call(nativeStep, "accumulatedDependencies"), setup.radius, call(nativeStep, "task"));
            region = TreeEffectReference.allocate(type(REGION));
            for (var e : Map.of("cache", cache, "center", chunks.get(new Pos(setup.origin.x() >> 4, 0, setup.origin.z() >> 4)),
                    "level", serverLevel, "generatingStep", step, "subTickCount", new AtomicLong(), "dimensionType", dimension, "seed", worldSeed).entrySet())
                TreeEffectReference.setField(region, REGION, e.getKey(), e.getValue());
            Object levelData = Proxy.newProxyInstance(GeodeReference.class.getClassLoader(), new Class<?>[]{type("world.level.storage.LevelData")}, (p, m, a) -> {
                if (m.getName().equals("getGameTime")) return 100L;
                throw new UnsupportedOperationException(m.toString());
            });
            TreeEffectReference.setField(region, REGION, "levelData", levelData);
            for (String kind : List.of("Block", "Fluid")) {
                Function<Object, Object> getter = p -> {
                    try { return call(call(region, "getChunk", p), "get" + kind + "Ticks"); }
                    catch (Exception e) { throw new RuntimeException(e); }
                };
                TreeEffectReference.setField(region, REGION, kind.toLowerCase(Locale.ROOT) + "Ticks", make("world.ticks.WorldGenTickAccess", getter));
            }
            proxy = Proxy.newProxyInstance(GeodeReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
                operations.add(m.getName());
                switch (m.getName()) {
                    case "getSeed": seedQueries++; break;
                    case "ensureCanWrite": origins.add(xyz(Pos.from(a[0]))); break;
                    case "getBlockState", "getFluidState": {
                        Object value;
                        try { value = blockRead.invoke(region, a[0]); }
                        catch (InvocationTargetException e) { failedRead = Pos.from(a[0]); throw e.getCause(); }
                        recordRead(Pos.from(a[0]), value);
                        return m.getName().equals("getBlockState") ? value : call(value, "getFluidState");
                    }
                    case "isStateAtPosition": {
                        Object value = blockRead.invoke(region, a[0]);
                        recordRead(Pos.from(a[0]), value);
                        @SuppressWarnings("unchecked") Predicate<Object> predicate = (Predicate<Object>) a[1];
                        return predicate.test(value);
                    }
                    case "setBlock": {
                        Pos pos = Pos.from(a[0]);
                        boolean accepted = !setup.reject && (boolean) call(region, "setBlock", a[0], a[1], a[2], a.length == 4 ? a[3] : 512);
                        writes.add(new int[]{pos.x(), pos.y(), pos.z(), id(a[1]), (int) a[2], accepted ? 1 : 0});
                        touched.add(pos);
                        return accepted;
                    }
                    case "scheduleTick": {
                        Pos pos = Pos.from(a[0]);
                        boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                        int target = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]) : id(call(a[1], "defaultBlockState"));
                        ticks.add(new int[]{pos.x(), pos.y(), pos.z(), target, (int) a[2], fluid ? 1 : 0});
                        break;
                    }
                    // No broad silent stubs: every unexpected feature query is
                    // an error, even if WorldGenRegion happens to implement it.
                    default: throw new UnsupportedOperationException("unobserved geode operation: " + m);
                }
                try { return m.invoke(region, a); }
                catch (InvocationTargetException e) { throw e.getCause(); }
            });
        }

        void recordRead(Pos p, Object state) throws Exception {
            int[] row = new int[]{p.x(), p.y(), p.z(), id(state)};
            addHash(readHash, row);
            if (firstReads.size() < 24) firstReads.add(row);
            readCount++;
        }

        Object createChunk(int cx, int cz) throws Exception {
            Object sections = Array.newInstance(type("world.level.chunk.LevelChunkSection"), 24);
            for (int s = 0; s < 24; s++) Array.set(sections, s, make("world.level.chunk.LevelChunkSection", factoryFor(setup.background)));
            for (var e : setup.initial.entrySet()) {
                Pos p = e.getKey();
                if (p.x() >> 4 != cx || p.z() >> 4 != cz || p.y() < -64 || p.y() > 319) continue;
                Object section = Array.get(sections, (p.y() + 64) >> 4);
                sectionSet.invoke(section, p.x() & 15, p.y() & 15, p.z() & 15, e.getValue(), false);
            }
            for (int s = 0; s < 24; s++) call(Array.get(sections, s), "recalcBlockCounts");
            return make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", cx, cz), field("world.level.chunk.UpgradeData", "EMPTY"),
                sections, make("world.ticks.ProtoChunkTicks"), make("world.ticks.ProtoChunkTicks"), height, factory, null);
        }

        Map<String, Object> result() throws Exception {
            List<int[]> finalBlocks = new ArrayList<>(), marks = new ArrayList<>(), nativeTicks = new ArrayList<>();
            Map<Integer, Integer> counts = new TreeMap<>();
            for (Pos p : touched) {
                int state = id(blockRead.invoke(region, at(p)));
                finalBlocks.add(new int[]{p.x(), p.y(), p.z(), state});
                counts.merge(state, 1, Integer::sum);
            }
            Method unpack = type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates", short.class, int.class, type("world.level.ChunkPos"));
            for (Object chunk : chunks.values()) {
                Object[] sections = (Object[]) call(chunk, "getPostProcessing");
                for (int s = 0; s < sections.length; s++) if (sections[s] != null)
                    for (Object packed : (Iterable<?>) sections[s]) marks.add(xyz(Pos.from(unpack.invoke(null, packed, s - 4, call(chunk, "getPos")))));
                for (String kind : List.of("Block", "Fluid")) {
                    for (Object tick : (List<?>) call(call(chunk, "get" + kind + "Ticks"), "scheduledTicks")) {
                        Pos p = Pos.from(call(tick, "pos"));
                        Object target = call(tick, "type");
                        boolean fluid = kind.equals("Fluid");
                        int targetId = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", target) : id(call(target, "defaultBlockState"));
                        nativeTicks.add(new int[]{p.x(), p.y(), p.z(), targetId, (int) call(tick, "delay"), fluid ? 1 : 0, (int) call(call(tick, "priority"), "getValue")});
                    }
                }
            }
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("write_calls", writes); result.put("final_sha256", hash(finalBlocks)); result.put("final_state_counts", counts);
            result.put("marks", marks); result.put("tick_requests", ticks); result.put("native_ticks", nativeTicks);
            result.put("origins", origins); result.put("read_count", readCount); result.put("read_sha256", HexFormat.of().formatHex(readHash.digest()));
            result.put("first_reads", firstReads); result.put("seed_queries", seedQueries); result.put("operations", operations);
            result.put("failed_read", failedRead == null ? null : xyz(failedRead));
            return result;
        }
    }

    static void addHash(MessageDigest digest, int[] row) {
        ByteBuffer bytes = ByteBuffer.allocate(row.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int value : row) bytes.putInt(value);
        digest.update(bytes.array());
    }
    static String hash(List<int[]> rows) throws Exception {
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        for (int[] row : rows) addHash(digest, row);
        return HexFormat.of().formatHex(digest.digest());
    }

    static Map<String, Object> sample(String name, String documentName, long featureSeed, long worldSeed, Setup setup) throws Exception {
        Object feature = nativeConfigs.get(documentName);
        if (feature == null) { feature = decode(documents.get(documentName)); nativeConfigs.put(documentName, feature); }
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", featureSeed));
        if (setup.advance) { call(random, "nextInt", 17); call(random, "nextFloat"); call(random, "nextInt", 1073741825); }
        List<Double> floats = new ArrayList<>();
        Object randomProxy = Proxy.newProxyInstance(GeodeReference.class.getClassLoader(), new Class<?>[]{type("util.RandomSource")}, (p, m, a) -> {
            try {
                Object result = m.invoke(random, a);
                if (m.getName().equals("nextFloat") && floats.size() < 8) floats.add(((Float) result).doubleValue());
                return result;
            } catch (InvocationTargetException e) { throw e.getCause(); }
        });
        World world = new World(setup, worldSeed);
        List<Object> results = new ArrayList<>();
        List<int[]> checkpoints = new ArrayList<>();
        String nativeError = null;
        for (int i = 0; i < setup.repetitions; i++) {
            if (setup.reseed && i > 0) call(random, "setSeed", featureSeed + i);
            try { results.add(call(feature, "place", world.proxy, generator, randomProxy, at(setup.origin))); }
            catch (Exception e) {
                if (!name.equals("unavailable_neighbor") || world.failedRead == null) throw e;
                Throwable cause = e;
                while (cause.getCause() != null) cause = cause.getCause();
                nativeError = cause.getClass().getName();
                results.add(null);
            }
            checkpoints.add(new int[]{(int) call(random, "getCount"), world.writes.size(), world.ticks.size(), world.readCount});
            if (nativeError != null) break;
        }
        Map<String, Object> result = world.result();
        result.put("name", name); result.put("config", documentName); result.put("feature_seed", featureSeed); result.put("world_seed", worldSeed);
        result.put("origin", xyz(setup.origin)); result.put("background", id(setup.background));
        List<int[]> initial = new ArrayList<>();
        List<Pos> positions = new ArrayList<>(setup.initial.keySet()); positions.sort(ORDER);
        for (Pos p : positions) initial.add(new int[]{p.x(), p.y(), p.z(), id(setup.initial.get(p))});
        result.put("initial", initial); result.put("write_radius", setup.radius); result.put("read_radius", setup.readRadius);
        result.put("reject_writes", setup.reject); result.put("advance", setup.advance); result.put("reseed", setup.reseed);
        result.put("results", results); result.put("checkpoints", checkpoints); result.put("native_error", nativeError);
        result.put("rng_count", call(random, "getCount")); result.put("next_i64", call(random, "nextLong")); result.put("first_floats", floats);
        System.err.println(name + ": " + results + ", " + world.writes.size() + " writes, " + world.ticks.size() + " tick requests");
        return result;
    }

    static void putConfig(String name, Map<String, Object> document) { documents.put(name, document); }
    static Map<String, Object> custom() throws Exception { return copy(documents.get("amethyst_geode")); }
    static Map<Pos, Object> obstacles(Pos o) throws Exception {
        Map<Pos, Object> result = new HashMap<>();
        String[] names = {"BEDROCK", "CHEST", "SPAWNER", "REINFORCED_DEEPSLATE", "TRIAL_SPAWNER", "VAULT", "END_PORTAL_FRAME", "WATER", "LAVA", "ICE", "CAVE_AIR", "OAK_LEAVES"};
        for (int x = -2; x <= 12; x += 2) for (int y = -2; y <= 12; y += 2) for (int z = -2; z <= 12; z += 2)
            result.put(new Pos(o.x() + x, o.y() + y, o.z() + z), block(names[Math.floorMod(x * 3 + y * 5 + z * 7, names.length)]));
        return result;
    }

    static List<Object> samples() throws Exception {
        List<Object> samples = new ArrayList<>();
        Pos origin = new Pos(15, 20, -17);
        Setup stone = Setup.stone(origin);
        for (long seed : new long[]{0, 1, 17, 42, -1, 846692123413862008L})
            samples.add(sample("stock_seed_" + seed, "amethyst_geode", seed, 846692123413862008L, stone));
        for (long seed : new long[]{0, 1, 17, 42, -1, Long.MIN_VALUE, Long.MAX_VALUE})
            samples.add(sample("world_seed_" + seed, "amethyst_geode", 42, seed, stone));
        for (String background : List.of("AIR", "CAVE_AIR", "BEDROCK", "WATER", "LAVA", "ICE", "PACKED_ICE", "BLUE_ICE", "CHEST", "REINFORCED_DEEPSLATE", "DEEPSLATE"))
            samples.add(sample("background_" + background, "amethyst_geode", 17, 42, stone.background(block(background))));
        samples.add(sample("mixed_obstacles", "amethyst_geode", 42, 17, stone.initial(obstacles(origin))));
        samples.add(sample("rejected_setter", "amethyst_geode", 42, 17, stone.rejectWrites()));
        samples.add(sample("write_radius_zero", "amethyst_geode", 42, 17, stone.radius(0)));
        samples.add(sample("denied_origin", "amethyst_geode", 42, 17, stone.radius(-1)));
        samples.add(sample("unavailable_neighbor", "amethyst_geode", 42, 17, stone.reads(0)));
        samples.add(sample("advanced_repeat", "amethyst_geode", 42, 17, stone.repeat(true, 3, false)));
        samples.add(sample("reseed_repeat", "amethyst_geode", 42, 17, stone.repeat(false, 3, true)));
        for (Pos p : List.of(new Pos(-16, -32, 31), new Pos(-17, -32, 16), new Pos(29999983, 20, -29999984), new Pos(-29999985, 20, 29999983)))
            samples.add(sample("coordinate_" + p, "amethyst_geode", 17, -1, Setup.stone(p)));

        Map<String, Object> allowed = custom(); config(allowed).put("invalid_blocks_threshold", 20); putConfig("allow_invalid", allowed);
        for (int y : new int[]{-80, -70, -65, -64, -63, 305, 314, 319, 320}) {
            Setup boundary = Setup.stone(new Pos(-17, y, 15));
            samples.add(sample("stock_boundary_" + y, "amethyst_geode", 42, 17, boundary));
            samples.add(sample("allowed_boundary_" + y, "allow_invalid", 42, 17, boundary));
        }
        for (int threshold : new int[]{-1, 0, 1, 2, 3, 4}) {
            Map<String, Object> document = custom();
            config(document).put("distribution_points", 4); config(document).put("invalid_blocks_threshold", threshold);
            String name = "invalid_threshold_" + threshold; putConfig(name, document);
            samples.add(sample(name, name, 42, 17, stone.background(block("WATER"))));
        }

        Map<String, Object> dense = custom();
        config(dense).put("placements_require_layer0_alternate", false);
        config(dense).put("use_potential_placements_chance", 1.0);
        group(config(dense), "crack").put("generate_crack_chance", 0.0);
        putConfig("all_placements", dense);
        for (int seed = 0; seed < 4; seed++) samples.add(sample("all_directions_" + seed, "all_placements", seed, 17, stone));
        samples.add(sample("all_placements_rejected", "all_placements", 0, 17, stone.rejectWrites()));
        samples.add(sample("all_placements_guarded", "all_placements", 0, 17, stone.radius(0)));

        for (String fill : List.of("water", "flowing_water", "falling_water", "falling_flowing_water", "lava", "cave_air", "void_air")) {
            Map<String, Object> document = copy(dense);
            Object value = fill.contains("water") && !fill.equals("water") ? Map.of("type", "minecraft:simple_state_provider", "state",
                stateDoc("water", Map.of("level", fill.equals("flowing_water") ? "1" : fill.equals("falling_water") ? "8" : "9"))) : provider(fill);
            group(config(document), "blocks").put("filling_provider", value);
            String name = "fill_" + fill; putConfig(name, document);
            samples.add(sample(name, name, 17, 42, stone));
        }
        for (String fill : List.of("water", "lava")) {
            Map<String, Object> document = copy(documents.get("fill_" + fill));
            group(config(document), "crack").put("generate_crack_chance", 1.0);
            String name = "crack_" + fill; putConfig(name, document);
            samples.add(sample(name, name, 42, 17, stone));
            samples.add(sample(name + "_guarded", name, 42, 17, stone.radius(0)));
        }
        // Known native postprocess blocks exercise setter-owned marks with the
        // very same kernel. Crystal orientation only tests the FACING property.
        for (String placement : List.of("oak_stairs", "stone", "red_mushroom", "brown_mushroom", "sugar_cane", "cactus")) {
            Map<String, Object> document = copy(dense);
            group(config(document), "blocks").put("inner_placements", List.of(Map.of("Name", "minecraft:" + placement)));
            String name = "placement_" + placement; putConfig(name, document);
            samples.add(sample(name, name, 17, 42, stone));
        }
        Map<String, Object> customTag = copy(dense);
        group(config(customTag), "blocks").put("cannot_replace", "#minecraft:air");
        putConfig("protected_air", customTag);
        samples.add(sample("protected_air", "protected_air", 17, 42, stone));

        Map<String, Object> defaults = custom();
        config(defaults).keySet().retainAll(Set.of("blocks", "layers", "crack", "invalid_blocks_threshold"));
        config(defaults).put("layers", Map.of()); config(defaults).put("crack", Map.of());
        putConfig("codec_defaults", defaults);
        samples.add(sample("codec_defaults", "codec_defaults", 17, 42, stone));
        Map<String, Object> fallback = custom();
        config(fallback).put("noise_multiplier", 2.0); config(fallback).put("outer_wall_distance", 21);
        group(config(fallback), "layers").put("filling", 0.0); group(config(fallback), "crack").put("crack_point_offset", -1);
        putConfig("codec_fallback", fallback);
        samples.add(sample("codec_fallback", "codec_fallback", 17, 42, stone));

        for (String mode : List.of("one_point", "zero_offset", "reversed_offsets", "empty_shape", "no_alternate", "all_alternate", "wide_points")) {
            Map<String, Object> document = custom(); Map<String, Object> c = config(document);
            switch (mode) {
                case "one_point": c.put("distribution_points", 1); c.put("outer_wall_distance", 4); break;
                case "zero_offset": c.put("distribution_points", 3); c.put("outer_wall_distance", 4); c.put("point_offset", 0); break;
                case "reversed_offsets": c.put("min_gen_offset", 12); c.put("max_gen_offset", -4); break;
                case "empty_shape": c.put("distribution_points", 1); c.put("outer_wall_distance", 20); c.put("layers", Map.of("filling", 0.01, "inner_layer", 0.01, "middle_layer", 0.01, "outer_layer", 0.01)); break;
                case "no_alternate": c.put("use_alternate_layer0_chance", 0.0); break;
                case "all_alternate": c.put("use_alternate_layer0_chance", 1.0); c.put("use_potential_placements_chance", 1.0); break;
                case "wide_points": c.put("outer_wall_distance", uniform(1, 20)); c.put("distribution_points", 20); break;
            }
            putConfig(mode, document);
            samples.add(sample(mode, mode, 17, 42, stone));
        }
        addProviders(samples, stone);
        addPrecisionCases(samples, stone);
        return samples;
    }

    static void addProviders(List<Object> samples, Setup stone) throws Exception {
        List<Object> providers = List.of(
            Map.of("type", "minecraft:constant", "value", 4), uniform(4, 4),
            Map.of("type", "minecraft:biased_to_bottom", "min_inclusive", 1, "max_inclusive", 8),
            Map.of("type", "minecraft:clamped", "source", uniform(-2, 10), "min_inclusive", 2, "max_inclusive", 6),
            Map.of("type", "minecraft:weighted_list", "distribution", List.of(Map.of("data", 4, "weight", 0), Map.of("data", 2, "weight", 1))),
            Map.of("type", "minecraft:clamped_normal", "mean", 4.5, "deviation", 2.5, "min_inclusive", 1, "max_inclusive", 9));
        for (int index = 0; index < providers.size(); index++) {
            Map<String, Object> document = custom();
            config(document).put("outer_wall_distance", providers.get(index));
            config(document).put("distribution_points", 3);
            String name = "int_provider_" + index; putConfig(name, document);
            samples.add(sample(name, name, 42, 17, stone.repeat(true, 2, index == 5)));
        }
        Map<String, Object> weighted = custom();
        Object choices = Map.of("type", "minecraft:weighted_state_provider", "entries", List.of(
            Map.of("data", Map.of("Name", "minecraft:stone"), "weight", 0),
            Map.of("data", Map.of("Name", "minecraft:calcite"), "weight", 1),
            Map.of("data", Map.of("Name", "minecraft:amethyst_block"), "weight", 2)));
        for (String key : List.of("filling_provider", "inner_layer_provider", "alternate_inner_layer_provider", "middle_layer_provider", "outer_layer_provider"))
            group(config(weighted), "blocks").put(key, choices);
        putConfig("weighted_layers", weighted);
        samples.add(sample("weighted_layers", "weighted_layers", 42, 17, stone));
        samples.add(sample("weighted_protected", "weighted_layers", 42, 17, stone.background(block("CHEST"))));
        samples.add(sample("weighted_rejected", "weighted_layers", 42, 17, stone.rejectWrites()));
        Map<String, Object> rotated = custom();
        group(config(rotated), "blocks").put("outer_layer_provider", Map.of("type", "minecraft:rotated_block_provider", "state", Map.of("Name", "minecraft:basalt")));
        putConfig("rotated_layer", rotated); samples.add(sample("rotated_layer", "rotated_layer", 42, 17, stone));
        Map<String, Object> randomized = custom();
        group(config(randomized), "blocks").put("middle_layer_provider", Map.of("type", "minecraft:randomized_int_state_provider", "source", provider("snow"), "property", "layers", "values", uniform(1, 8)));
        putConfig("randomized_layer", randomized); samples.add(sample("randomized_layer", "randomized_layer", 42, 17, stone));
        Map<String, Object> rules = custom();
        group(config(rules), "blocks").put("outer_layer_provider", Map.of("type", "minecraft:rule_based_state_provider", "rules", List.of(
            Map.of("if_true", Map.of("type", "minecraft:matching_blocks", "blocks", "minecraft:stone"), "then", provider("calcite")))));
        putConfig("rule_layer", rules); samples.add(sample("rule_layer", "rule_layer", 42, 17, stone));
        samples.add(sample("rule_layer_fallback", "rule_layer", 42, 17, stone.background(block("DEEPSLATE"))));
        Map<String, Object> noise = custom();
        group(config(noise), "blocks").put("outer_layer_provider", Map.of("type", "minecraft:noise_provider", "seed", 1729,
            "noise", Map.of("firstOctave", -3, "amplitudes", List.of(1.0, 0.5)), "scale", 0.33,
            "states", List.of(Map.of("Name", "minecraft:calcite"), Map.of("Name", "minecraft:smooth_basalt"))));
        putConfig("noise_layer", noise); samples.add(sample("noise_layer", "noise_layer", 42, 17, stone));
    }

    @SuppressWarnings("unchecked")
    static void addPrecisionCases(List<Object> samples, Setup stone) throws Exception {
        Map<String, Object> document = custom();
        config(document).put("placements_require_layer0_alternate", false);
        config(document).put("use_alternate_layer0_chance", 0.0);
        putConfig("precision_base", document);
        Map<String, Object> nativeBase = sample("precision_base", "precision_base", 17, 42, stone);
        samples.add(nativeBase);
        List<Double> draws = (List<Double>) nativeBase.get("first_floats");
        String[] groups = {"crack", "config", "config"};
        String[] fields = {"generate_crack_chance", "use_alternate_layer0_chance", "use_potential_placements_chance"};
        for (int i = 0; i < 3; i++) for (int side = -1; side <= 1; side++) {
            Map<String, Object> threshold = copy(document);
            double draw = draws.get(i), value = side < 0 ? Math.nextDown(draw) : side > 0 ? Math.nextUp(draw) : draw;
            (groups[i].equals("crack") ? group(config(threshold), "crack") : config(threshold)).put(fields[i], value);
            String name = "double_threshold_" + i + "_" + side; putConfig(name, threshold);
            samples.add(sample(name, name, 17, 42, stone));
        }
    }

    static Map<String, Object> predicateData() throws Exception {
        List<int[]> ranges = new ArrayList<>();
        List<int[]> postprocess = new ArrayList<>();
        Object facing = field("world.level.block.state.properties.BlockStateProperties", "FACING");
        Object wet = field("world.level.block.state.properties.BlockStateProperties", "WATERLOGGED");
        Object noReads = Proxy.newProxyInstance(GeodeReference.class.getClassLoader(), new Class<?>[]{type("world.level.BlockGetter")}, (p, m, a) -> { throw new UnsupportedOperationException(m.toString()); });
        Object zero = at(new Pos(0, 0, 0));
        int[] previous = null;
        Method tag = (Method) inTag;
        Method grow = type("world.level.block.BuddingAmethystBlock").getMethod("canClusterGrowAtState", type("world.level.block.state.BlockState"));
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            Object fluid = call(state, "getFluidState");
            int number = id(state);
            int flags = ((boolean) call(state, "isAir") ? 1 : 0)
                | ((boolean) tag.invoke(state, invalidBlocks) ? 2 : 0)
                | ((boolean) tag.invoke(state, cannotReplace) ? 4 : 0)
                | ((boolean) grow.invoke(null, state) ? 8 : 0)
                | ((boolean) call(fluid, "isSource") ? 16 : 0)
                | ((boolean) call(fluid, "isFull") ? 32 : 0)
                | ((boolean) call(state, "hasProperty", facing) ? 64 : 0)
                | ((boolean) call(state, "hasProperty", wet) ? 128 : 0);
            int fluidId = (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", call(fluid, "getType"));
            int amount = (int) call(fluid, "getAmount");
            if (previous != null && previous[2] == flags && previous[3] == fluidId && previous[4] == amount) previous[1] = number + 1;
            else { previous = new int[]{number, number + 1, flags, fluidId, amount}; ranges.add(previous); }
            Object mark = call(state, "getPostProcessPos", noReads, zero);
            if (mark != null) { Pos p = Pos.from(mark); postprocess.add(new int[]{number, p.x(), p.y(), p.z()}); }
        }
        return Map.of("ranges", ranges, "postprocess", postprocess);
    }

    static List<Object> numericData() throws Exception {
        List<Object> result = new ArrayList<>();
        Method create = type("world.level.levelgen.synth.NormalNoise").getMethod("create", type("util.RandomSource"), int.class, double[].class);
        Method value = type("world.level.levelgen.synth.NormalNoise").getMethod("getValue", double.class, double.class, double.class);
        for (long seed : new long[]{0, 1, -1, 17, 42, 846692123413862008L, Long.MIN_VALUE, Long.MAX_VALUE}) {
            Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.LegacyRandomSource", seed));
            Object noise = create.invoke(null, random, -4, new double[]{1.0});
            List<Object> values = new ArrayList<>();
            for (int x : new int[]{-30000000, -16777217, -17, -1, 0, 1, 15, 16777217, 29999999}) {
                for (int y : new int[]{-65, -64, 0, 20, 319, 320}) {
                    int z = 13 - x;
                    double sample = (double) value.invoke(noise, (double) x, (double) y, (double) z);
                    values.add(List.of(x, y, z, Double.doubleToRawLongBits(sample)));
                }
            }
            result.add(Map.of("seed", seed, "noise_bits", values, "independent_random_count", call(random, "getCount")));
        }
        return result;
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        List<Object> samples = samples();
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("setup", "Actual GeodeFeature.place via ConfiguredFeature; constructed CARVERS ProtoChunks/holders; native WorldGenRegion guards, storage, marks and tick queues; game time 100; server POI callback record-only. reject_writes is an explicit injected setter failure. No postprocessing/tick execution or global scheduling.");
        result.put("java_runtime", System.getProperty("java.runtime.version"));
        result.put("configs", documents); result.put("predicates", predicateData()); result.put("numerics", numericData()); result.put("samples", samples);
        System.out.println("GEODE_REFERENCE=" + call(gson, "toJson", result));
    }
}
