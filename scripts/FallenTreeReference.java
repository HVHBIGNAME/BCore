import java.lang.constant.*;
import java.lang.invoke.MethodHandles;
import java.lang.reflect.*;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.Consumer;
import java.util.function.Predicate;

/** Invoke native 26.1 FallenTreeFeature, configs and decorators on explicit terrain.
 * Reads are unbounded horizontally; build-height reads/writes use air/rejection.
 * Captures direct writes and postprocessing requests, not subsequent ticks or
 * neighbour updates. Support fixtures require native cached (context-free) shapes.
 */
public class FallenTreeReference extends TreeReference {
    static final List<String> KINDS = List.of("fallen_oak_tree", "fallen_birch_tree",
        "fallen_super_birch_tree", "fallen_jungle_tree", "fallen_spruce_tree");
    static final Pos ORIGIN = new Pos(8, 65, 8);
    static final Comparator<Pos> POSITION_ORDER = Comparator.comparingInt(Pos::x)
        .thenComparingInt(Pos::y).thenComparingInt(Pos::z);
    record Case(String kind, long seed, boolean ground, Pos origin, int floorY, String soil, String terrain) {}

    static Case current;
    static Object soil, gson, postprocessingSink;
    static Field shapeCache;
    static final Map<String, Object> configurations = new HashMap<>();
    static final Map<Object, Integer> stateIds = new IdentityHashMap<>();
    static final Map<Pos, Object> initial = new HashMap<>();
    static final Set<Object> observedStates = new HashSet<>();
    static final List<Pos> postprocessing = new ArrayList<>();
    static MessageDigest readsDigest, writesDigest;
    static int readCount, writeCount;

    @SuppressWarnings("unchecked")
    static Object fallenLevel() throws Exception {
        return Proxy.newProxyInstance(FallenTreeReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            return switch (m.getName()) {
                case "getMinY" -> -64;
                case "getMaxY" -> 319;
                case "getHeight" -> 384;
                case "getBlockState" -> getFallen(a[0]);
                case "isStateAtPosition" -> ((Predicate<Object>) a[1]).test(getFallen(a[0]));
                case "isFluidAtPosition" -> ((Predicate<Object>) a[1]).test(call(getFallen(a[0]), "getFluidState"));
                case "setBlock" -> {
                    Pos pos = Pos.from(a[0]);
                    boolean accepted = insideHeight(pos) && !current.terrain().equals("reject_writes");
                    putInts(writesDigest, pos.x(), pos.y(), pos.z(), id(a[1]), (int) a[2], accepted ? 1 : 0);
                    writeCount++;
                    observedStates.add(a[1]);
                    if (accepted) blocks.put(pos, a[1]);
                    yield accepted;
                }
                case "getChunk" -> {
                    if (a.length != 1 || !type("core.BlockPos").isInstance(a[0])) {
                        throw new UnsupportedOperationException(m.toString());
                    }
                    yield postprocessingSink;
                }
                default -> throw new UnsupportedOperationException(m.toString());
            };
        });
    }

    static boolean insideHeight(Pos pos) { return pos.y() >= -64 && pos.y() <= 319; }

    static Object blockAt(Pos pos) {
        if (!insideHeight(pos)) return air;
        Object state = blocks.get(pos);
        if (state != null) return state;
        state = initial.get(pos);
        if (state != null) return state;
        return current.ground() && pos.y() == current.floorY() ? soil : air;
    }

    static Object getFallen(Object pos) throws Exception {
        Pos p = Pos.from(pos);
        Object state = blockAt(p);
        putInts(readsDigest, p.x(), p.y(), p.z(), id(state));
        readCount++;
        observedStates.add(state);
        return state;
    }

    static int id(Object state) throws Exception {
        Integer id = stateIds.get(state);
        if (id == null) {
            id = (int) call(type("world.level.block.Block"), "getId", state);
            stateIds.put(state, id);
        }
        return id;
    }

    static void putInts(MessageDigest digest, int... values) {
        ByteBuffer bytes = ByteBuffer.allocate(values.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int value : values) bytes.putInt(value);
        digest.update(bytes.array());
    }

    static String hash() throws Exception {
        ByteBuffer buffer = ByteBuffer.allocate(384 * 256 * 4).order(ByteOrder.LITTLE_ENDIAN);
        int bx = Math.floorDiv(current.origin().x(), 16) * 16;
        int bz = Math.floorDiv(current.origin().z(), 16) * 16;
        for (int y = -64; y < 320; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
            buffer.putInt(id(blockAt(new Pos(bx + x, y, bz + z))));
        }
        return HexFormat.of().formatHex(MessageDigest.getInstance("MD5").digest(buffer.array()));
    }

    static Object config(String kind) throws Exception {
        String path = "/data/minecraft/worldgen/configured_feature/" + kind + ".json";
        String json;
        try (var stream = FallenTreeReference.class.getResourceAsStream(path)) {
            json = new String(Objects.requireNonNull(stream, path).readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
        }
        Object document = call(Class.forName("com.google.gson.JsonParser"), "parseString", json);
        Object value = call(call(document, "getAsJsonObject"), "get", "config");
        Object ops = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        return call(call(field("world.level.levelgen.feature.configurations.FallenTreeConfiguration", "CODEC"), "parse", ops, value), "getOrThrow");
    }

    static String probe(Object feature, String kind, long seed, boolean ground) throws Exception {
        return probe(feature, new Case(kind, seed, ground, ORIGIN, 64, "GRASS_BLOCK", "flat"));
    }

    static Object soilState(String name) throws Exception {
        if (!name.equals("OAK_SLAB_TOP")) return state(name);
        Object slab = state("OAK_SLAB");
        Object property = call(call(field("world.level.block.Blocks", "OAK_SLAB"), "getStateDefinition"), "getProperty", "type");
        Object top = ((Optional<?>) call(property, "getValue", "top")).orElseThrow();
        return call(slab, "setValue", property, top);
    }

    static void prepare(Case sample) throws Exception {
        current = sample;
        blocks = new HashMap<>();
        initial.clear(); observedStates.clear(); postprocessing.clear();
        readsDigest = MessageDigest.getInstance("MD5");
        writesDigest = MessageDigest.getInstance("MD5");
        readCount = 0; writeCount = 0;
        soil = soilState(sample.soil());
        Pos origin = sample.origin();
        if (sample.terrain().equals("stump_cover")) {
            initial.put(origin, state("BEDROCK"));
            initial.put(new Pos(origin.x(), origin.y() + 1, origin.z()), state("SHORT_GRASS"));
            initial.put(new Pos(origin.x(), origin.y() + 2, origin.z()), state("STONE"));
            initial.put(new Pos(origin.x(), origin.y() + 3, origin.z()), state("STONE"));
            return;
        }
        for (int[] direction : new int[][]{{0,-1}, {1,0}, {0,1}, {-1,0}}) {
            for (int distance = 2; distance <= 16; distance++) {
                int x = origin.x() + direction[0] * distance;
                int z = origin.z() + direction[1] * distance;
                switch (sample.terrain()) {
                    case "flat", "reject_writes" -> {}
                    case "covered" -> initial.put(new Pos(x, origin.y() + 1, z), state("STONE"));
                    case "obstacle", "replaceable" -> {
                        if (distance == 4) initial.put(new Pos(x, origin.y(), z), state(sample.terrain().equals("obstacle") ? "STONE" : "WATER"));
                    }
                    case "gap_two", "gap_three", "repeated_gaps" -> {
                        boolean gap = distance == 4 || distance == 5
                            || (sample.terrain().equals("gap_three") && distance == 6)
                            || (sample.terrain().equals("repeated_gaps") && (distance == 7 || distance == 8));
                        if (gap) initial.put(new Pos(x, sample.floorY(), z), air);
                    }
                    default -> throw new IllegalArgumentException(sample.terrain());
                }
            }
        }
    }

    static List<int[]> sortedBlocks(Map<Pos, Object> states) throws Exception {
        List<Pos> positions = new ArrayList<>(states.keySet());
        positions.sort(POSITION_ORDER);
        List<int[]> result = new ArrayList<>();
        for (Pos pos : positions) result.add(new int[]{pos.x(), pos.y(), pos.z(), id(states.get(pos))});
        return result;
    }

    static List<List<Object>> supportStates(Object world, Object origin) throws Exception {
        Map<Integer, Boolean> support = new TreeMap<>();
        int before = readCount;
        for (Object state : observedStates) {
            if (shapeCache.get(state) == null) {
                throw new IllegalStateException("Context-dependent support shape is outside this probe's terrain model: " + state);
            }
            support.put(id(state), (boolean) call(state, "isFaceSturdy", world, origin, field("core.Direction", "UP")));
        }
        if (readCount != before) throw new IllegalStateException("A cached support query unexpectedly read the world");
        List<List<Object>> result = new ArrayList<>();
        for (var entry : support.entrySet()) result.add(List.of(entry.getKey(), entry.getValue()));
        return result;
    }

    static String probe(Object feature, Case sample) throws Exception {
        prepare(sample);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", sample.seed()));
        Pos pos = sample.origin();
        Object origin = make("core.BlockPos", pos.x(), pos.y(), pos.z());
        Object world = fallenLevel();
        Object context = make("world.level.levelgen.feature.FeaturePlaceContext", Optional.empty(), world, null, random,
            origin, Objects.requireNonNull(configurations.get(sample.kind()), sample.kind()));
        boolean placed = (boolean) call(feature, "place", context);
        long next = (long) call(random, "nextLong");
        Map<String, Object> record = new LinkedHashMap<>();
        record.put("kind", sample.kind()); record.put("seed", sample.seed());
        record.put("flat_ground", sample.ground()); record.put("placed", placed);
        record.put("states_md5", hash()); record.put("next_i64", next);
        record.put("origin", new int[]{pos.x(), pos.y(), pos.z()});
        record.put("chunk", new int[]{Math.floorDiv(pos.x(), 16), Math.floorDiv(pos.z(), 16)});
        record.put("floor_y", sample.floorY()); record.put("soil", id(soil));
        record.put("soil_name", sample.soil()); record.put("terrain", sample.terrain());
        record.put("initial_blocks", sortedBlocks(initial)); record.put("writes", sortedBlocks(blocks));
        record.put("read_count", readCount); record.put("reads_md5", HexFormat.of().formatHex(readsDigest.digest()));
        record.put("write_count", writeCount); record.put("write_calls_md5", HexFormat.of().formatHex(writesDigest.digest()));
        record.put("postprocessing", postprocessing.stream().map(p -> new int[]{p.x(), p.y(), p.z()}).toList());
        record.put("cached_up_support", supportStates(world, origin));
        return (String) call(gson, "toJson", record);
    }

    static Set<Case> cases() {
        Set<Case> result = new LinkedHashSet<>();
        // Preserve the original ten inputs and their order as the first subset.
        for (String kind : KINDS.subList(0, 2)) {
            for (long seed : new long[]{0, 1, 17, 42}) result.add(new Case(kind, seed, true, ORIGIN, 64, "GRASS_BLOCK", "flat"));
            result.add(new Case(kind, 0, false, ORIGIN, 64, "GRASS_BLOCK", "flat"));
        }
        Set<Long> seeds = new LinkedHashSet<>();
        for (long seed = 0; seed < 64; seed++) seeds.add(seed);
        seeds.addAll(List.of(42L, -1L, Long.MIN_VALUE, Long.MAX_VALUE));
        for (String kind : KINDS) {
            for (long seed : seeds) result.add(new Case(kind, seed, true, ORIGIN, 64, "GRASS_BLOCK", "flat"));
            for (long seed : new long[]{0, 1, 17, 42, -1}) result.add(new Case(kind, seed, false, ORIGIN, 64, "GRASS_BLOCK", "flat"));
            for (String soil : List.of("DIRT", "COARSE_DIRT", "PODZOL", "ROOTED_DIRT", "MOSS_BLOCK", "MUD", "FARMLAND", "DIRT_PATH", "STONE", "WATER", "OAK_LEAVES", "OAK_SLAB", "OAK_SLAB_TOP")) {
                result.add(new Case(kind, 42, true, ORIGIN, 64, soil, "flat"));
            }
            for (Pos origin : List.of(new Pos(0,65,0), new Pos(-1,65,16), new Pos(-17,65,-17), new Pos(29999984,65,-29999985))) {
                for (long seed : new long[]{0, 1, 17, 42}) result.add(new Case(kind, seed, true, origin, 64, "GRASS_BLOCK", "flat"));
            }
            for (Pos origin : List.of(new Pos(-1,-63,16), new Pos(15,-64,-16), new Pos(-17,319,-17))) {
                for (long seed : new long[]{0, 1}) result.add(new Case(kind, seed, true, origin, origin.y() - 1, "GRASS_BLOCK", "flat"));
            }
            for (String terrain : List.of("obstacle", "replaceable", "gap_two", "gap_three", "repeated_gaps", "covered", "stump_cover", "reject_writes")) {
                for (long seed : new long[]{0, 1}) result.add(new Case(kind, seed, true, new Pos(-1,65,16), 64, "GRASS_BLOCK", terrain));
            }
            for (int floor : new int[]{65, 61, 59, 58}) {
                for (long seed : new long[]{0, 1}) result.add(new Case(kind, seed, true, new Pos(15,65,-16), floor, "GRASS_BLOCK", "flat"));
            }
        }
        return result;
    }

    public static void recordPostprocessing(Object pos) throws Exception {
        postprocessing.add(Pos.from(pos));
    }

    static Object classfileCall(String owner, Object target, String name, Object... args) throws Exception {
        for (Method method : Class.forName(owner).getMethods()) {
            if (method.getName().equals(name) && matches(method.getParameterTypes(), args)) return method.invoke(target, args);
        }
        throw new NoSuchMethodException(owner + "." + name);
    }

    /** Instrument only the ChunkAccess callback reached by native Feature.
     * Its override records world positions; no uninitialized chunk machinery is used.
     * Java 25's classfile API is reflected so the source still compiles on javac 21.
     */
    static Object postprocessingSink() throws Exception {
        Object classfile = classfileCall("java.lang.classfile.ClassFile", null, "of");
        Consumer<Object> build = builder -> {
            try {
                classfileCall("java.lang.classfile.ClassBuilder", builder, "withFlags", 1);
                classfileCall("java.lang.classfile.ClassBuilder", builder, "withSuperclass", ClassDesc.of("net.minecraft.world.level.chunk.ProtoChunk"));
                Consumer<Object> body = code -> {
                    try {
                        classfileCall("java.lang.classfile.CodeBuilder", code, "aload", 1);
                        classfileCall("java.lang.classfile.CodeBuilder", code, "invokestatic", ClassDesc.of("FallenTreeReference"), "recordPostprocessing", MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object));
                        classfileCall("java.lang.classfile.CodeBuilder", code, "return_");
                    } catch (Exception e) { throw new RuntimeException(e); }
                };
                classfileCall("java.lang.classfile.ClassBuilder", builder, "withMethodBody", "markPosForPostprocessing",
                    MethodTypeDesc.of(ConstantDescs.CD_void, ClassDesc.of("net.minecraft.core.BlockPos")), 1, body);
            } catch (Exception e) { throw new RuntimeException(e); }
        };
        byte[] bytes = (byte[]) classfileCall("java.lang.classfile.ClassFile", classfile, "build", ClassDesc.of("FallenTreePostprocessingSink"), build);
        Class<?> sink = MethodHandles.lookup().defineClass(bytes);
        Class<?> unsafe = Class.forName("sun.misc.Unsafe");
        Field singleton = unsafe.getDeclaredField("theUnsafe"); singleton.setAccessible(true);
        return unsafe.getMethod("allocateInstance", Class.class).invoke(singleton.get(null), sink);
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        bindTreeTags();
        air = state("AIR"); grass = state("GRASS_BLOCK");
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        shapeCache = type("world.level.block.state.BlockBehaviour$BlockStateBase").getDeclaredField("cache");
        shapeCache.setAccessible(true);
        postprocessingSink = postprocessingSink();
        for (String kind : KINDS) configurations.put(kind, config(kind));
        Object feature = make("world.level.levelgen.feature.FallenTreeFeature",
            field("world.level.levelgen.feature.configurations.FallenTreeConfiguration", "CODEC"));
        List<String> records = new ArrayList<>();
        for (Case sample : cases()) records.add(probe(feature, sample));
        System.out.println("FALLEN_REFERENCE=[" + String.join(",", records) + "]");
    }
}
