import java.lang.reflect.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.*;

/** Full native TreeFeature and placed selectors. Historical bounded driver is
 * preserved verbatim under vegetation-bounded/ for the original 504-case fixture.
 * BCORE_STANDING_PROBE=blocks extracts state predicates; otherwise capture trees.
 */
public class VegetationReference extends TreeReference {
    static final List<String> ROOTS = List.of("trees_plains", "trees_birch", "trees_birch_and_oak_leaf_litter",
        "birch_tall", "trees_taiga", "trees_snowy", "trees_savanna", "trees_windswept_hills");
    static final List<String> KINDS = List.of("oak", "birch", "spruce", "pine", "fancy_oak", "acacia",
        "oak_bees_005", "fancy_oak_bees_005", "birch_bees_0002", "super_birch_bees_0002",
        "oak_bees_0002_leaf_litter", "birch_bees_0002_leaf_litter", "fancy_oak_bees_0002_leaf_litter");
    static Object gson, registries, configured, placed, biomes, generator, heightAccessor, chunkFactory;
    static Method px, py, pz, blockInTag, fluidInTag;
    static final Map<Object, Integer> ids = new IdentityHashMap<>();
    static final Set<Object> logStates = Collections.newSetFromMap(new IdentityHashMap<>());
    static final Map<Object, Predicate<Object>> predicates = new IdentityHashMap<>();
    static final Map<String, int[]> slots = new TreeMap<>();
    static final Map<String, Object> rootBiomes = new TreeMap<>();
    static Capture current;

    record Terrain(int floor, Object soil, Object cover, Map<Pos, Object> initial, boolean reject) {}
    static final class Capture {
        final Terrain terrain;
        final Object biome;
        final Map<Pos, Object> writes = new HashMap<>(), blockEntities = new HashMap<>(), chunks = new HashMap<>();
        final List<int[]> writePrefix = new ArrayList<>(), marks = new ArrayList<>(), ticks = new ArrayList<>(), draws = new ArrayList<>();
        final MessageDigest writeDigest;
        int writeCount, readCount, standingLogWrites, fallenLogWrites, writeLimit = 16;
        Capture(Terrain terrain, Object biome) throws Exception {
            this.terrain = terrain; this.biome = biome;
            writeDigest = MessageDigest.getInstance("MD5");
        }
        Object block(Pos p) {
            if (p.y() < -64 || p.y() > 319) return air;
            return writes.getOrDefault(p, terrain.initial().getOrDefault(p,
                p.y() == terrain.floor() ? terrain.soil() : p.y() == terrain.floor() + 1 ? terrain.cover() : air));
        }
        Object read(Object p) throws Exception { readCount++; return block(position(p)); }
        int height(Object kind, int x, int z) {
            int top = Math.min(319, terrain.floor() + 1);
            for (Pos p : terrain.initial().keySet()) if (p.x() == x && p.z() == z) top = Math.max(top, p.y());
            for (Pos p : writes.keySet()) if (p.x() == x && p.z() == z) top = Math.max(top, p.y());
            for (int y = Math.min(top, 319); y >= -64; y--) {
                if (predicates.get(kind).test(block(new Pos(x, y, z)))) return y + 1;
            }
            return -64;
        }
        Object chunk(Pos p) throws Exception {
            Pos key = new Pos(p.x() >> 4, 0, p.z() >> 4);
            Object chunk = chunks.get(key);
            if (chunk == null) {
                Object sections = Array.newInstance(type("world.level.chunk.LevelChunkSection"), 24);
                for (int i = 0; i < 24; i++) Array.set(sections, i, make("world.level.chunk.LevelChunkSection", chunkFactory));
                chunk = make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", key.x(), key.z()),
                    field("world.level.chunk.UpgradeData", "EMPTY"), sections, make("world.ticks.ProtoChunkTicks"),
                    make("world.ticks.ProtoChunkTicks"), heightAccessor, chunkFactory, null);
                chunks.put(key, chunk);
            }
            return chunk;
        }
    }

    static Pos position(Object p) throws Exception { return new Pos((int) px.invoke(p), (int) py.invoke(p), (int) pz.invoke(p)); }
    static int id(Object state) { return Objects.requireNonNull(ids.get(state)); }
    static int[] xyz(Pos p) { return new int[]{p.x(), p.y(), p.z()}; }
    static Object holder(Object registry, String name) throws Exception {
        return ((Optional<?>) call(registry, "get", call(type("resources.Identifier"), "withDefaultNamespace", name))).orElseThrow();
    }
    static void digest(MessageDigest digest, int[] row) {
        ByteBuffer bytes = ByteBuffer.allocate(row.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int value : row) bytes.putInt(value);
        digest.update(bytes.array());
    }
    static Object json(String directory, String name) throws Exception {
        String path = "/data/minecraft/worldgen/" + directory + "/" + name + ".json";
        try (var input = VegetationReference.class.getResourceAsStream(path)) {
            return call(gson, "fromJson", new String(Objects.requireNonNull(input, path).readAllBytes(), StandardCharsets.UTF_8), Map.class);
        }
    }

    @SuppressWarnings("unchecked")
    static void bootstrap() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion"); call(type("server.Bootstrap"), "bootStrap");
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        registries = NativeWorldgenRegistries.load();
        configured = call(registries, "lookupOrThrow", field("core.registries.Registries", "CONFIGURED_FEATURE"));
        placed = call(registries, "lookupOrThrow", field("core.registries.Registries", "PLACED_FEATURE"));
        biomes = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        Object parameters = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
            field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomes);
        Object source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", parameters));
        Object settings = call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS"));
        generator = make("world.level.levelgen.NoiseBasedChunkGenerator", source, holder(settings, "overworld"));
        List<?> possible = new ArrayList<>((Set<?>) call(source, "possibleBiomes"));
        Function<Object, Object> features = biome -> {
            try { return call(call(call(biome, "value"), "getGenerationSettings"), "features"); }
            catch (Exception e) { throw new RuntimeException(e); }
        };
        List<?> steps = (List<?>) call(type("world.level.biome.FeatureSorter"), "buildFeaturesPerStep", possible, features, true);
        for (String root : ROOTS) {
            Object feature = call(holder(placed, root), "value");
            for (int step = 0; step < steps.size(); step++) {
                int index = ((List<?>) call(steps.get(step), "features")).indexOf(feature);
                if (index >= 0) slots.put(root, new int[]{step, index});
            }
            for (Object biome : possible) {
                if ((boolean) call(call(call(biome, "value"), "getGenerationSettings"), "hasFeature", feature)) {
                    rootBiomes.put(root, biome); break;
                }
            }
        }
        air = state("AIR"); grass = state("GRASS_BLOCK");
        int next = 0;
        // is(Object) and is(TagKey) are both public; invoke the tag overload explicitly.
        blockInTag = type("world.level.block.state.BlockBehaviour$BlockStateBase").getMethod("is", type("tags.TagKey"));
        fluidInTag = type("world.level.material.FluidState").getMethod("is", type("tags.TagKey"));
        Object logs = field("tags.BlockTags", "LOGS");
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            ids.put(state, next++);
            if ((boolean) blockInTag.invoke(state, logs)) logStates.add(state);
        }
        if (logStates.isEmpty()) throw new IllegalStateException("Native log tag was not loaded");
        px = type("core.Vec3i").getMethod("getX"); py = type("core.Vec3i").getMethod("getY"); pz = type("core.Vec3i").getMethod("getZ");
        for (String name : List.of("OCEAN_FLOOR", "WORLD_SURFACE", "MOTION_BLOCKING_NO_LEAVES")) {
            Object kind = field("world.level.levelgen.Heightmap$Types", name);
            predicates.put(kind, (Predicate<Object>) call(kind, "isOpaque"));
        }
        heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        Object blocks = call(type("world.level.chunk.Strategy"), "createForBlockStates", field("world.level.block.Block", "BLOCK_STATE_REGISTRY"));
        Object biomeStrategy = call(type("world.level.chunk.Strategy"), "createForBiomes", call(biomes, "asHolderIdMap"));
        chunkFactory = make("world.level.chunk.PalettedContainerFactory", blocks, air, null, biomeStrategy, holder(biomes, "plains"), null);
    }

    @SuppressWarnings("unchecked")
    static Object world() throws Exception {
        Object noRandomDraws = Proxy.newProxyInstance(VegetationReference.class.getClassLoader(), new Class<?>[]{type("util.RandomSource")},
            (p, m, a) -> { throw new UnsupportedOperationException("Shape-update world RNG: " + m); });
        return Proxy.newProxyInstance(VegetationReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            return switch (m.getName()) {
                case "getMinY" -> -64;
                case "getMaxY" -> 319;
                case "getHeight" -> a == null || a.length == 0 ? 384 : current.height(a[0], (int) a[1], (int) a[2]);
                case "getHeightmapPos" -> { Pos pos = position(a[1]); yield make("core.BlockPos", pos.x(), current.height(a[0], pos.x(), pos.z()), pos.z()); }
                case "getBiome" -> current.biome;
                case "getBlockState" -> current.read(a[0]);
                case "getFluidState" -> call(current.read(a[0]), "getFluidState");
                case "isStateAtPosition" -> ((Predicate<Object>) a[1]).test(current.read(a[0]));
                case "isFluidAtPosition" -> ((Predicate<Object>) a[1]).test(call(current.read(a[0]), "getFluidState"));
                case "ensureCanWrite" -> true;
                case "getRandom" -> noRandomDraws;
                case "isClientSide" -> false;
                case "scheduleTick" -> {
                    Pos pos = position(a[0]);
                    boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                    int value = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]) : id(call(a[1], "defaultBlockState"));
                    current.ticks.add(new int[]{pos.x(), pos.y(), pos.z(), value, (int) a[2], fluid ? 1 : 0});
                    yield null;
                }
                case "setBlock" -> {
                    Pos pos = position(a[0]);
                    boolean accepted = !current.terrain.reject() && pos.y() >= -64 && pos.y() <= 319;
                    int[] row = {pos.x(), pos.y(), pos.z(), id(a[1]), (int) a[2], accepted ? 1 : 0};
                    digest(current.writeDigest, row); current.writeCount++;
                    if (logStates.contains(a[1])) {
                        if ((int) a[2] == 3) current.fallenLogWrites++;
                        if ((int) a[2] == 19) current.standingLogWrites++;
                    }
                    if (current.writePrefix.size() < current.writeLimit) current.writePrefix.add(row);
                    if (accepted) {
                        current.writes.put(pos, a[1]); current.blockEntities.remove(pos);
                        if ((boolean) call(a[1], "hasBlockEntity")) {
                            current.blockEntities.put(pos, call(call(a[1], "getBlock"), "newBlockEntity", a[0], a[1]));
                        }
                        if (((int) a[2] & 16) == 0) {
                            Object mark = call(a[1], "getPostProcessPos", p, a[0]);
                            if (mark != null) current.marks.add(xyz(position(mark)));
                        }
                    }
                    yield accepted;
                }
                case "getBlockEntity" -> {
                    Object entity = current.blockEntities.get(position(a[0]));
                    yield a.length == 1 ? entity : Optional.ofNullable(entity);
                }
                case "getChunk" -> {
                    if (a.length != 1) throw new UnsupportedOperationException(m.toString());
                    Pos pos = position(a[0]); current.marks.add(xyz(pos)); yield current.chunk(pos);
                }
                default -> throw new UnsupportedOperationException(m.toString());
            };
        });
    }

    static Object random(Object delegate) throws Exception {
        return Proxy.newProxyInstance(VegetationReference.class.getClassLoader(), new Class<?>[]{type("util.RandomSource")}, (p, m, a) -> {
            Object value = m.invoke(delegate, a);
            switch (m.getName()) {
                case "nextInt" -> current.draws.add(new int[]{0, (int) a[0], (int) value});
                case "nextIntBetweenInclusive" -> current.draws.add(new int[]{0, (int) a[1] - (int) a[0] + 1, (int) value - (int) a[0]});
                case "nextFloat" -> current.draws.add(new int[]{1, 0, Float.floatToRawIntBits((float) value)});
                case "nextBoolean" -> current.draws.add(new int[]{0, 2, (boolean) value ? 1 : 0});
                default -> throw new UnsupportedOperationException(m.toString());
            }
            return value;
        });
    }

    static List<int[]> states(Map<Pos, Object> blocks) {
        List<Pos> positions = new ArrayList<>(blocks.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        return positions.stream().map(p -> new int[]{p.x(), p.y(), p.z(), id(blocks.get(p))}).toList();
    }

    static Map<String, Object> snapshot(String kind, long seed, String scenario, Pos origin, List<int[]> sources, List<Object> results) throws Exception {
        Map<String, Object> record = new LinkedHashMap<>();
        record.put("kind", kind); record.put("seed", seed); record.put("scenario", scenario); record.put("origin", xyz(origin));
        record.put("floor_y", current.terrain.floor()); record.put("soil", id(current.terrain.soil())); record.put("cover", id(current.terrain.cover()));
        record.put("initial_blocks", states(current.terrain.initial())); record.put("sources", sources); record.put("results", results);
        record.put("biome", call(call(current.biome, "key"), "identifier").toString());
        record.put("writes", states(current.writes)); record.put("write_count", current.writeCount);
        record.put("standing_log_writes", current.standingLogWrites); record.put("fallen_log_writes", current.fallenLogWrites);
        record.put("write_calls_md5", HexFormat.of().formatHex(current.writeDigest.digest())); record.put("write_prefix", current.writePrefix);
        record.put("postprocessing", current.marks); record.put("tick_requests", current.ticks); record.put("draws", current.draws);
        List<Object> entities = new ArrayList<>();
        List<Pos> positions = new ArrayList<>(current.blockEntities.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        for (Pos pos : positions) {
            Object tag = call(current.blockEntities.get(pos), "saveWithFullMetadata", registries);
            Object data = call(field("nbt.NbtOps", "INSTANCE"), "convertTo", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), tag);
            entities.add(Map.of("pos", xyz(pos), "data", data));
        }
        record.put("block_entities", entities);
        return record;
    }

    static Map<String, Object> isolated(String kind, long seed, String scenario, Pos origin) throws Exception {
        Map<Pos, Object> initial = new HashMap<>();
        if (scenario.equals("obstructed")) initial.put(new Pos(origin.x() + 1, origin.y() + 2, origin.z()), state("STONE"));
        if (scenario.equals("wet_foliage")) initial.put(new Pos(origin.x() + 1, origin.y() + 3, origin.z()), state("WATER"));
        if (scenario.equals("flowing_water")) initial.put(new Pos(origin.x() + 1, origin.y() + 3, origin.z()), call(state("WATER"), "setValue", field("world.level.block.LiquidBlock", "LEVEL"), 1));
        if (scenario.equals("persistent_leaf")) initial.put(new Pos(origin.x() + 1, origin.y() + 3, origin.z()), call(state("OAK_LEAVES"), "setValue", field("world.level.block.state.properties.BlockStateProperties", "PERSISTENT"), true));
        if (scenario.equals("vine_cover")) initial.put(new Pos(origin.x() + 1, origin.y() + 3, origin.z()), call(type("world.level.block.Block"), "stateById", 8373));
        if (scenario.equals("existing_logs")) {
            for (int x = -4; x <= 4; x++) for (int y = 0; y < 18; y++) for (int z = -4; z <= 4; z++) {
                initial.put(new Pos(origin.x()+x, origin.y()+y, origin.z()+z), state("OAK_LOG"));
            }
        }
        Terrain terrain = new Terrain(origin.y() - 1, grass, air, initial, scenario.endsWith("reject_writes"));
        current = new Capture(terrain, holder(biomes, "forest"));
        if (scenario.equals("repeat_live")) current.writeLimit = Integer.MAX_VALUE;
        Object raw = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        if (scenario.startsWith("advanced_")) { call(raw, "nextInt", 17); call(raw, "nextFloat"); call(raw, "nextInt", 1073741825); }
        List<Pos> attempts = scenario.equals("repeat_live") ? List.of(origin, new Pos(origin.x()+2, origin.y(), origin.z()+4), origin) : List.of(origin);
        List<Object> results = new ArrayList<>();
        for (int i = 0; i < attempts.size(); i++) {
            Pos p = attempts.get(i);
            boolean result = (boolean) call(call(holder(configured, kind), "value"), "place", world(), generator, random(raw), make("core.BlockPos", p.x(), p.y(), p.z()));
            Map<String, Object> outcome = new LinkedHashMap<>();
            outcome.put("placed", result); outcome.put("origin", xyz(p));
            if (i + 1 == attempts.size()) outcome.put("next_i64", call(raw, "nextLong"));
            results.add(outcome);
        }
        return snapshot(kind, seed, scenario, origin, List.of(), results);
    }

    static Map<String, Object> mixed(String root, long seed, List<int[]> sources) throws Exception {
        current = new Capture(new Terrain(64, grass, air, Map.of(), false), rootBiomes.get(root));
        List<Object> results = new ArrayList<>();
        int[] slot = slots.get(root);
        for (int[] source : sources) {
            Object raw = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
            long decoration = (long) call(raw, "setDecorationSeed", seed, source[0] * 16, source[1] * 16);
            call(raw, "setFeatureSeed", decoration, slot[1], slot[0]);
            boolean result = (boolean) call(call(holder(placed, root), "value"), "placeWithBiomeCheck", world(), generator, random(raw), make("core.BlockPos", source[0] * 16, -64, source[1] * 16));
            results.add(Map.of("placed", result, "decoration_seed", decoration, "slot", slot, "next_i64", call(raw, "nextLong")));
        }
        return snapshot(root, seed, "mixed", new Pos(0, -64, 0), sources, results);
    }

    static Map<String, Object> blockPredicates() throws Exception {
        List<int[]> ranges = new ArrayList<>();
        int[] previous = null;
        Object noLeaves = field("world.level.levelgen.Heightmap$Types", "MOTION_BLOCKING_NO_LEAVES");
        Object water = field("world.level.material.Fluids", "WATER");
        Object persistent = field("world.level.block.state.properties.BlockStateProperties", "PERSISTENT");
        Object supports = field("tags.BlockTags", "SUPPORTS_VEGETATION");
        Object snow = field("tags.BlockTags", "SNOW");
        Object bubble = field("tags.FluidTags", "BUBBLE_COLUMN_CAN_OCCUPY");
        Object bubbleDown = field("tags.BlockTags", "ENABLES_BUBBLE_COLUMN_DRAG_DOWN");
        Object bubbleUp = field("tags.BlockTags", "ENABLES_BUBBLE_COLUMN_PUSH_UP");
        Map<Class<?>, Integer> shapes = new HashMap<>();
        Object empty = Proxy.newProxyInstance(VegetationReference.class.getClassLoader(), new Class<?>[]{type("world.level.BlockGetter")},
            (p, m, a) -> { throw new UnsupportedOperationException("Contextual shape " + m); });
        Object location = make("core.BlockPos", 0, 65, 0);
        Object[] directions = type("core.Direction").getEnumConstants();
        Field cache = type("world.level.block.state.BlockBehaviour$BlockStateBase").getDeclaredField("cache"); cache.setAccessible(true);
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            Object block = call(state, "getBlock");
            Class<?> cls = block.getClass();
            Integer shape = shapes.get(cls);
            if (shape == null) {
                String owner = null;
                for (Class<?> c = cls; c != null && owner == null; c = c.getSuperclass()) {
                    for (Method m : c.getDeclaredMethods()) if (m.getName().equals("updateShape") && m.getParameterCount() == 8) { owner = c.getSimpleName(); break; }
                }
                shape = switch (Objects.requireNonNull(owner, cls.toString())) {
                    case "BlockBehaviour" -> 0;
                    case "LeavesBlock" -> 1;
                    case "VegetationBlock" -> 2;
                    case "VineBlock" -> 3;
                    case "SnowyBlock" -> 4;
                    case "LiquidBlock" -> 6;
                    case "BeehiveBlock" -> 7;
                    default -> 255;
                };
                if (cls.getSimpleName().equals("LeafLitterBlock")) shape = 5;
                if (shape == 2 && (!methodOwner(cls, "canSurvive", 3).equals("VegetationBlock")
                    || !methodOwner(cls, "mayPlaceOn", 3).equals("VegetationBlock"))) shape = 255;
                shapes.put(cls, shape);
            }
            int flags = (boolean) call(state, "isSolidRender") ? 1 : 0;
            if (predicates.get(noLeaves).test(state)) flags |= 2;
            if ((boolean) call(call(state, "getFluidState"), "isSourceOfType", water)) flags |= 4;
            if ((boolean) blockInTag.invoke(state, supports)) flags |= 8;
            if ((boolean) blockInTag.invoke(state, snow)) flags |= 16;
            if ((boolean) call(state, "getValueOrElse", persistent, false)) flags |= 32;
            Object fluid = call(state, "getFluidState");
            if ((boolean) call(fluid, "isSource")) flags |= 64;
            if ((boolean) fluidInTag.invoke(fluid, bubble) && (boolean) call(fluid, "isSource") && (boolean) call(fluid, "isFull")) flags |= 128;
            if ((boolean) blockInTag.invoke(state, bubbleDown) || (boolean) blockInTag.invoke(state, bubbleUp)) flags |= 256;
            if (type("world.level.block.FireBlock").isInstance(block)) flags |= 512;
            int distance = ((OptionalInt) call(type("world.level.block.LeavesBlock"), "getOptionalDistanceAt", state)).orElse(-1);
            int attach = -1;
            if (cache.get(state) != null) {
                attach = 0;
                for (int i = 0; i < directions.length; i++) {
                    if ((boolean) call(type("world.level.block.MultifaceBlock"), "canAttachTo", empty, directions[i], location, state)) attach |= 1 << i;
                }
            }
            int stateShape = shape == 6 && block != field("world.level.block.Blocks", "WATER") ? 255 : shape;
            int[] row = {id(state), id(state) + 1, flags, distance, stateShape, id(call(block, "defaultBlockState")), attach};
            if (previous != null && Arrays.equals(Arrays.copyOfRange(previous, 2, 7), Arrays.copyOfRange(row, 2, 7))) previous[1]++;
            else { ranges.add(row); previous = row; }
        }
        Map<String, Object> definitions = new TreeMap<>();
        for (String kind : KINDS) definitions.put(kind, json("configured_feature", kind));
        return Map.of("state_count", ids.size(), "ranges", ranges, "configurations", definitions,
            "water_fluid_ids", List.of(call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", field("world.level.material.Fluids", "FLOWING_WATER")), call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", water)),
            "bee_nest_south", id(call(state("BEE_NEST"), "setValue", field("world.level.block.BeehiveBlock", "FACING"), field("core.Direction", "SOUTH"))), "samples", List.of());
    }

    static String methodOwner(Class<?> cls, String name, int args) {
        for (Class<?> c = cls; c != null; c = c.getSuperclass()) {
            for (Method m : c.getDeclaredMethods()) if (m.getName().equals(name) && m.getParameterCount() == args) return c.getSimpleName();
        }
        throw new IllegalArgumentException(cls + "." + name);
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        if ("blocks".equals(System.getenv("BCORE_STANDING_PROBE"))) {
            System.out.println("VEGETATION_REFERENCE=" + call(gson, "toJson", blockPredicates())); return;
        }
        List<Object> samples = new ArrayList<>();
        for (String kind : KINDS) for (long seed : new long[]{0, 1, 17, 42}) {
            for (String terrain : List.of("flat", "obstructed", "wet_foliage", "reject_writes")) {
                samples.add(isolated(kind, seed, terrain, new Pos(-1, 65, 16)));
            }
        }
        for (String root : ROOTS) for (long seed : new long[]{0, 1, 17, 42, 388}) {
            samples.add(mixed(root, seed, List.of(new int[]{-1, 0})));
        }
        for (String kind : KINDS) {
            for (Pos origin : List.of(new Pos(-17,65,-17), new Pos(15,65,15), new Pos(-1,-63,16), new Pos(15,315,15), new Pos(-1,319,16), new Pos(0,65,0), new Pos(29999999,65,-29999985))) {
                samples.add(isolated(kind, -1, "flat", origin));
            }
            for (String terrain : List.of("flowing_water", "persistent_leaf", "vine_cover", "advanced_stream", "advanced_reject_writes", "repeat_live")) {
                samples.add(isolated(kind, 17, terrain, new Pos(-1,65,16)));
            }
        }
        samples.add(isolated("acacia", 17, "existing_logs", new Pos(-1,65,16)));
        // Seeds from the historical 504-case oracle enter real fallen branches;
        // on the full grass plane their subsequent standing attempts now run too.
        long[][] targeted = {{0,24},{1,78},{2,49},{2,77},{3,79},{3,228},{4,33},{5,387},{6,67},{7,388},{7,1242}};
        for (long[] entry : targeted) samples.add(mixed(ROOTS.get((int) entry[0]), entry[1], List.of(new int[]{0,0})));
        long[] regionSeeds = {24,78,49,79,33,387,67,388};
        List<int[]> sources = new ArrayList<>();
        for (int x = -1; x <= 1; x++) for (int z = -1; z <= 1; z++) sources.add(new int[]{x,z});
        for (int i = 0; i < ROOTS.size(); i++) samples.add(mixed(ROOTS.get(i), regionSeeds[i], sources));
        List<int[]> reversed = new ArrayList<>(sources); Collections.reverse(reversed);
        samples.add(mixed("trees_birch_and_oak_leaf_litter", 49, reversed));
        System.out.println("VEGETATION_REFERENCE=" + call(gson, "toJson", Map.of("version", 2, "samples", samples, "scope",
            "Complete native configured trees and eight mixed placed selectors on explicit terrain; full TreeFeature leaf/edge updates, direct flags, bee data and tick requests. No ticks executed or global dependency schedule inferred.")));
    }
}
