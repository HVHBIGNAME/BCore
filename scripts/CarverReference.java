import java.lang.constant.*;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.lang.reflect.*;
import java.util.*;
import java.util.function.Consumer;
import java.util.function.Function;

/** Native carver kernels and bounded voxel operations, without terrain generation.
 * Geometry cases intercept carveEllipsoid. Voxel cases run native carveEllipsoid
 * and carveBlock against real ProtoChunk sections and explicitly scripted fluids
 * and surface callbacks; recording overrides delegate to the native implementations.
 * Surface cases additionally execute CarvingContext.topMaterial with a real
 * RandomState, NoiseChunk and overworld rule tree, without generating terrain.
 */
public class CarverReference extends OreReference {
    static final String CARVER = "world.level.levelgen.carver.";
    static final String[] KINDS = {"cave", "cave_extra_underground", "canyon"};
    static final long CANYON_WITNESS = 87921060185259L;
    static final long[] SEEDS = {0, 1, -1, 846692123413862008L,
        Long.MIN_VALUE, Long.MAX_VALUE - 1, Long.MAX_VALUE, CANYON_WITNESS};
    static final Object[] configured = new Object[3];
    static final Object[] configs = new Object[3];
    static final Object[] carvers = new Object[3];
    static Object registries, context, plains, generator;
    static Method reach, carve, tunnel, replace;
    static List<List<String>> trace;

    static Method method(String owner, String name, Class<?>... parameters) throws Exception {
        Method result = type(owner).getDeclaredMethod(name, parameters);
        result.setAccessible(true);
        return result;
    }

    static String bits(float value) { return HexFormat.of().toHexDigits(Float.floatToRawIntBits(value)); }
    static String bits(double value) { return HexFormat.of().toHexDigits(Double.doubleToRawLongBits(value)); }
    static List<String> bits(double... values) {
        List<String> result = new ArrayList<>();
        for (double value : values) result.add(bits(value));
        return result;
    }

    static Object registryValue(String registry, String name) throws Exception {
        Object key = call(type("resources.ResourceKey"), "create",
            field("core.registries.Registries", registry), call(type("resources.Identifier"), "parse", name));
        return call(call(registries, "lookupOrThrow", field("core.registries.Registries", registry)), "getOrThrow", key);
    }

    static Object sourceRandom(long seed, int index, int x, int z) throws Exception {
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.LegacyRandomSource", 0L));
        call(random, "setLargeFeatureSeed", seed + index, x, z);
        return random;
    }

    public static boolean recordEllipsoid(double x, double y, double z, double horizontal, double vertical) {
        if (trace == null) throw new IllegalStateException("carver trace is not active");
        trace.add(bits(x, y, z, horizontal, vertical));
        return false;
    }

    static Object recordingCarver(String kind) throws Exception {
        ClassDesc parent = ClassDesc.of(MC + CARVER + kind + "WorldCarver");
        MethodTypeDesc constructor = MethodTypeDesc.of(ConstantDescs.CD_void, ClassDesc.of("com.mojang.serialization.Codec"));
        Method ellipsoid = method(CARVER + "WorldCarver", "carveEllipsoid",
            type(CARVER + "CarvingContext"), type(CARVER + "CarverConfiguration"), type("world.level.chunk.ChunkAccess"),
            Function.class, type("world.level.levelgen.Aquifer"), double.class, double.class, double.class,
            double.class, double.class, type("world.level.chunk.CarvingMask"), type(CARVER + "WorldCarver$CarveSkipChecker"));
        MethodTypeDesc signature = MethodTypeDesc.ofDescriptor(MethodType.methodType(boolean.class, ellipsoid.getParameterTypes()).descriptorString());
        Consumer<Object> build = builder -> {
            try {
                NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withFlags", 1);
                NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withSuperclass", parent);
                Consumer<Object> init = code -> {
                    try {
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "aload", 0);
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "aload", 1);
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "invokespecial", parent, "<init>", constructor);
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "return_");
                    } catch (Exception e) { throw new RuntimeException(e); }
                };
                NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withMethodBody", "<init>", constructor, 1, init);
                Consumer<Object> record = code -> {
                    try {
                        // Use the reflected descriptor to obtain local slots, including double-width arguments.
                        for (int parameter = 5; parameter < 10; parameter++) {
                            int slot = (int) NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "parameterSlot", parameter);
                            NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "dload", slot);
                        }
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "invokestatic", ClassDesc.of("CarverReference"),
                            "recordEllipsoid", MethodTypeDesc.ofDescriptor("(DDDDD)Z"));
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "ireturn");
                    } catch (Exception e) { throw new RuntimeException(e); }
                };
                NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withMethodBody", "carveEllipsoid", signature, 4, record);
            } catch (Exception e) { throw new RuntimeException(e); }
        };
        Object classFile = NativeEntityLevel.api("java.lang.classfile.ClassFile", null, "of");
        byte[] bytes = (byte[]) NativeEntityLevel.api("java.lang.classfile.ClassFile", classFile, "build",
            ClassDesc.of("NativeCarverTrace" + kind), build);
        Class<?> generated = MethodHandles.lookup().defineClass(bytes);
        return generated.getConstructor(Class.forName("com.mojang.serialization.Codec"))
            .newInstance(field(CARVER + kind + "CarverConfiguration", "CODEC"));
    }

    static void bootstrap() throws Exception {
        bootstrapOre();
        registries = NativeWorldgenRegistries.load();
        for (int i = 0; i < KINDS.length; i++) {
            configured[i] = call(registryValue("CONFIGURED_CARVER", "minecraft:" + KINDS[i]), "value");
            configs[i] = call(configured[i], "config");
        }
        carvers[0] = recordingCarver("Cave");
        carvers[1] = carvers[0];
        carvers[2] = recordingCarver("Canyon");
        plains = registryValue("BIOME", "minecraft:plains");
        generator = make("world.level.levelgen.NoiseBasedChunkGenerator", make("world.level.biome.FixedBiomeSource", plains),
            registryValue("NOISE_SETTINGS", "minecraft:overworld"));
        // Geometry and height providers only use minY/depth; the overridden sink never queries surface/aquifer state.
        context = make(CARVER + "CarvingContext", generator, registries, height, null, null, null);
        reach = method(CARVER + "WorldCarver", "canReach", type("world.level.ChunkPos"),
            double.class, double.class, int.class, int.class, float.class);
        replace = method(CARVER + "WorldCarver", "canReplaceBlock", type(CARVER + "CarverConfiguration"), type("world.level.block.state.BlockState"));
        carve = method(CARVER + "WorldCarver", "carve", type(CARVER + "CarvingContext"), type(CARVER + "CarverConfiguration"),
            type("world.level.chunk.ChunkAccess"), Function.class, type("util.RandomSource"), type("world.level.levelgen.Aquifer"),
            type("world.level.ChunkPos"), type("world.level.chunk.CarvingMask"));
        tunnel = method(CARVER + "CaveWorldCarver", "createTunnel", type(CARVER + "CarvingContext"), type(CARVER + "CaveCarverConfiguration"),
            type("world.level.chunk.ChunkAccess"), Function.class, long.class, type("world.level.levelgen.Aquifer"),
            double.class, double.class, double.class, double.class, double.class, float.class, float.class, float.class,
            int.class, int.class, double.class, type("world.level.chunk.CarvingMask"), type(CARVER + "WorldCarver$CarveSkipChecker"));
        terrain = "air";
        surface = 64;
        baseFactory = airFactory;
        chunks = new HashMap<>();
    }

    static Map<String, Object> reachSample(String id, int cx, int cz, double x, double z, int step, int distance, float thickness) throws Exception {
        return Map.of("op", "can_reach", "id", id, "target", List.of(cx, cz), "xz_bits", bits(x, z),
            "step", step, "distance", distance, "thickness_bits", bits(thickness),
            "result", reach.invoke(null, make("world.level.ChunkPos", cx, cz), x, z, step, distance, thickness));
    }

    static void reachSamples(List<Object> samples) throws Exception {
        float[] thicknesses = {0, 0.005f, 0.1f, 0.505f, Math.nextDown(1.0f), 1, Math.nextUp(1.0f), 2.3f, 6};
        int n = 0;
        for (float thickness : thicknesses) for (int[] range : new int[][]{{0, 0}, {0, 1}, {0, 100}, {57, 112}, {111, 112}}) {
            // This formula selects sensitive inputs; every expected boolean comes from the native method above.
            double radius = (thickness + 2.0f) + 16.0f;
            double remaining = range[1] - range[0];
            double boundary = 8 + Math.sqrt(radius * radius + remaining * remaining);
            for (double p : new double[]{Math.nextDown(boundary), boundary, Math.nextUp(boundary)}) for (int axis = 0; axis < 2; axis++)
                samples.add(reachSample("reach/boundary/" + n++, 0, 0, axis == 0 ? p : 8, axis == 1 ? p : 8, range[0], range[1], thickness));
        }
        for (int[] target : new int[][]{{-17, 29}, {1874999, -1875000}}) for (int sign : new int[]{-1, 1}) for (int axis = 0; axis < 2; axis++) {
            double mx = target[0] * 16 + 8, mz = target[1] * 16 + 8;
            double boundary = (axis == 0 ? mx : mz) + sign * 18.5;
            for (double p : new double[]{Math.nextDown(boundary), boundary, Math.nextUp(boundary)})
                samples.add(reachSample("reach/translated/" + n++, target[0], target[1], axis == 0 ? p : mx, axis == 1 ? p : mz, 0, 0, 0.5f));
        }
        samples.add(reachSample("reach/widen_square", 0, 0, 110.03964899695846, 8, 0, 100, 2.3f));
        samples.add(reachSample("reach/group_add", 0, 0, 26.505, 8, 0, 0, 0.505f));
    }

    static void rngSamples(List<Object> samples) throws Exception {
        Method thickness = method(CARVER + "CaveWorldCarver", "getThickness", type("util.RandomSource"));
        Method widths = method(CARVER + "CanyonWorldCarver", "initWidthFactors", type(CARVER + "CarvingContext"),
            type(CARVER + "CanyonCarverConfiguration"), type("util.RandomSource"));
        for (long seed : SEEDS) {
            for (int[] source : new int[][]{{0, 0}, {1, -1}, {-8, 8}, {1874999, -1875000}}) for (int index = 0; index < KINDS.length; index++) {
                Object random = sourceRandom(seed, index, source[0], source[1]);
                boolean start = (boolean) call(configured[index], "isStartChunk", random);
                samples.add(Map.of("op", "source_rng", "id", "rng/" + seed + "/" + source[0] + "/" + source[1] + "/" + index,
                    "seed", seed, "source", List.of(source[0], source[1]), "index", index,
                    "start", start, "next_i64", call(random, "nextLong")));
            }
            Object random = call(type("util.RandomSource"), "createThreadLocalInstance", seed);
            List<String> values = new ArrayList<>();
            for (int i = 0; i < 16; i++) values.add(bits((float) thickness.invoke(carvers[0], random)));
            samples.add(Map.of("op", "cave_thickness", "id", "thickness/" + seed, "seed", seed, "bits", values, "next_i64", call(random, "nextLong")));
            random = call(type("util.RandomSource"), "createThreadLocalInstance", seed);
            values = new ArrayList<>();
            for (float value : (float[]) widths.invoke(carvers[2], context, configs[2], random)) values.add(bits(value));
            samples.add(Map.of("op", "canyon_widths", "id", "widths/" + seed, "seed", seed, "bits", values, "next_i64", call(random, "nextLong")));
        }
    }

    static Map<String, Object> carveSample(String id, long seed, int index, int sx, int sz, int tx, int tz, boolean force) throws Exception {
        Object random = sourceRandom(seed, index, sx, sz);
        boolean start = (boolean) call(configured[index], "isStartChunk", random);
        trace = new ArrayList<>();
        try {
            if (start || force) carve.invoke(carvers[index], context, configs[index], chunk(tx, tz),
                (Function<Object, Object>) p -> plains, random, null, make("world.level.ChunkPos", sx, sz),
                make("world.level.chunk.CarvingMask", 384, -64));
            if (id.equals("carve/canyon_length_witness") && (!start || trace.isEmpty()))
                throw new IllegalStateException("canyon witness did not exercise native geometry");
            Map<String, Object> sample = new LinkedHashMap<>();
            sample.put("op", "carve_trace"); sample.put("id", id); sample.put("seed", seed); sample.put("index", index);
            sample.put("source", List.of(sx, sz)); sample.put("target", List.of(tx, tz));
            sample.put("force", force); sample.put("start", start); sample.put("ellipsoids", trace);
            sample.put("next_i64", call(random, "nextLong"));
            return sample;
        } finally { trace = null; }
    }

    static Map<String, Object> tunnelSample(long seed, float thickness) throws Exception {
        double[] origin = {8, 40, 8}, multipliers = {(double) 0.9f, (double) 1.2f};
        float yaw = 0.2f, pitch = 0.075f;
        trace = new ArrayList<>();
        try {
            tunnel.invoke(carvers[0], context, configs[0], chunk(0, 0), (Function<Object, Object>) p -> plains,
                seed, null, origin[0], origin[1], origin[2], multipliers[0], multipliers[1], thickness, yaw, pitch,
                0, 100, 1.0, make("world.level.chunk.CarvingMask", 384, -64), null);
            Map<String, Object> sample = new LinkedHashMap<>();
            sample.put("op", "tunnel_trace"); sample.put("id", "tunnel/" + seed + "/" + bits(thickness)); sample.put("seed", seed);
            sample.put("target", List.of(0, 0)); sample.put("origin_bits", bits(origin)); sample.put("multiplier_bits", bits(multipliers));
            sample.put("thickness_bits", bits(thickness)); sample.put("yaw_bits", bits(yaw)); sample.put("pitch_bits", bits(pitch));
            sample.put("step", 0); sample.put("distance", 100); sample.put("ellipsoids", trace);
            return sample;
        } finally { trace = null; }
    }

    static Map<String, Object> predicateTables() throws Exception {
        int count = (int) call(field("world.level.block.Block", "BLOCK_STATE_REGISTRY"), "size");
        Method stateById = type("world.level.block.Block").getMethod("stateById", int.class);
        Map<String, List<Integer>> accepted = new LinkedHashMap<>();
        for (String kind : KINDS) accepted.put(kind, new ArrayList<>());
        Map<String, List<Integer>> blocks = new TreeMap<>();
        for (int id = 0; id < count; id++) {
            Object state = stateById.invoke(null, id);
            for (int index = 0; index < KINDS.length; index++) {
                if (!(boolean) replace.invoke(carvers[index], configs[index], state)) continue;
                accepted.get(KINDS[index]).add(id);
                if (index == 0) {
                    String block = call(field("core.registries.BuiltInRegistries", "BLOCK"), "getKey", call(state, "getBlock")).toString();
                    blocks.computeIfAbsent(block, b -> new ArrayList<>()).add(id);
                }
            }
        }
        return Map.of("state_count", count, "accepted", accepted, "blocks", blocks);
    }

    @FunctionalInterface
    interface BuildAction { void accept(Object builder) throws Exception; }

    static Consumer<Object> building(BuildAction action) {
        return builder -> {
            try { action.accept(builder); }
            catch (Exception e) { throw new RuntimeException(e); }
        };
    }

    static void emit(Object code, String operation, Object... args) throws Exception {
        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, operation, args);
    }

    static MethodTypeDesc descriptor(Class<?> result, Class<?>[] args) {
        return MethodTypeDesc.ofDescriptor(MethodType.methodType(result, args).descriptorString());
    }

    static void loadParameter(Object code, Class<?>[] parameters, int index) throws Exception {
        Class<?> type = parameters[index];
        String opcode = type == double.class ? "dload" : type == float.class ? "fload"
            : type == long.class ? "lload" : type.isPrimitive() ? "iload" : "aload";
        emit(code, opcode, NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "parameterSlot", index));
    }

    static int saveResult(Object code, boolean primitive) throws Exception {
        Object kind = Class.forName("java.lang.classfile.TypeKind").getField(primitive ? "INT" : "REFERENCE").get(null);
        int slot = (int) NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "allocateLocal", kind);
        emit(code, primitive ? "istore" : "astore", slot);
        return slot;
    }

    static void superCall(Object code, Class<?> parent, Method method) throws Exception {
        emit(code, "aload", 0);
        for (int i = 0; i < method.getParameterCount(); i++) loadParameter(code, method.getParameterTypes(), i);
        emit(code, "invokespecial", ClassDesc.of(parent.getName()), method.getName(), descriptor(method.getReturnType(), method.getParameterTypes()));
    }

    static void override(Object builder, Method method, BuildAction action) throws Exception {
        NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withMethodBody", method.getName(),
            descriptor(method.getReturnType(), method.getParameterTypes()), Modifier.PUBLIC, building(action));
    }

    static void callback(Object code, String name, String descriptor) throws Exception {
        emit(code, "invokestatic", ClassDesc.of("CarverReference"), name, MethodTypeDesc.ofDescriptor(descriptor));
    }

    static Class<?> subclass(String name, Class<?> parent, Class<?>[] constructor, BuildAction methods) throws Exception {
        parent.getConstructor(constructor); // Validate the native constructor before defining bytecode.
        MethodTypeDesc init = descriptor(void.class, constructor);
        Consumer<Object> build = building(builder -> {
            NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withFlags", Modifier.PUBLIC);
            NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withSuperclass", ClassDesc.of(parent.getName()));
            NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withMethodBody", "<init>", init, Modifier.PUBLIC,
                building(code -> {
                    emit(code, "aload", 0);
                    for (int i = 0; i < constructor.length; i++) loadParameter(code, constructor, i);
                    emit(code, "invokespecial", ClassDesc.of(parent.getName()), "<init>", init);
                    emit(code, "return_");
                }));
            methods.accept(builder);
        });
        Object api = NativeEntityLevel.api("java.lang.classfile.ClassFile", null, "of");
        byte[] bytes = (byte[]) NativeEntityLevel.api("java.lang.classfile.ClassFile", api, "build", ClassDesc.of(name), build);
        return MethodHandles.lookup().defineClass(bytes);
    }

    static Constructor<?> voxelChunkConstructor, voxelMaskConstructor, voxelContextConstructor;
    static Method nativeEllipsoid, nativeBlock, nativeCaveSkip, nativeCanyonSkip, nativeWidths, blockId, blockById;
    static final Object[] voxelCarvers = new Object[3];
    static final Map<Integer, Boolean> fluidStates = new TreeMap<>();
    static VoxelRun active;

    static int stateId(Object state) throws Exception {
        return state == null ? -1 : (int) blockId.invoke(null, state);
    }

    static int stateId(String block) throws Exception { return stateId(state(block)); }

    static Object voxelState(int id) throws Exception {
        if (id == -1) return null;
        Object state = blockById.invoke(null, id);
        if (stateId(state) != id) throw new IllegalArgumentException("invalid block state " + id);
        fluidStates.putIfAbsent(id, !(boolean) call(call(state, "getFluidState"), "isEmpty"));
        return state;
    }

    public static void recordVoxelWrite(Object position, Object old, Object state, int flags) throws Exception {
        if (active == null) return;
        Pos p = Pos.from(position);
        active.written.add(p);
        active.events.add(List.of("write", p.x(), p.y(), p.z(), stateId(old), stateId(state), flags));
    }

    public static void recordVoxelMark(Object position) throws Exception {
        if (active == null) return;
        Pos p = Pos.from(position);
        active.events.add(List.of("post", p.x(), p.y(), p.z()));
    }

    public static void recordMaskGet(int x, int y, int z, boolean result) {
        if (active != null) active.events.add(List.of("mask_get", x, y, z, result));
    }

    public static void recordMaskSet(int x, int y, int z) {
        if (active != null) active.events.add(List.of("mask_set", x, y, z));
    }

    public static void recordBlockResult(Object position, boolean result, Object flag) throws Exception {
        if (active == null) return;
        Pos p = Pos.from(position);
        active.events.add(List.of("block_result", p.x(), p.y(), p.z(), result, call(flag, "isTrue")));
    }

    public static Optional<Object> scriptedTopMaterial(Object position, boolean fluid) throws Exception {
        if (active == null) throw new IllegalStateException("surface callback outside voxel fixture");
        Pos p = Pos.from(position);
        int state = active.spec.top;
        active.events.add(List.of("top_material", p.x(), p.y(), p.z(), fluid, state));
        return Optional.ofNullable(voxelState(state));
    }

    static void bootstrapVoxels() throws Exception {
        blockId = type("world.level.block.Block").getMethod("getId", type("world.level.block.state.BlockState"));
        blockById = type("world.level.block.Block").getMethod("stateById", int.class);
        Class<?> chunk = type("world.level.chunk.ProtoChunk"), pos = type("core.BlockPos"), state = type("world.level.block.state.BlockState");
        Class<?>[] chunkArgs = {type("world.level.ChunkPos"), type("world.level.chunk.UpgradeData"),
            type("world.level.LevelHeightAccessor"), type("world.level.chunk.PalettedContainerFactory"), type("world.level.levelgen.blending.BlendingData")};
        Method write = chunk.getMethod("setBlockState", pos, state, int.class), mark = chunk.getMethod("markPosForPostprocessing", pos);
        Class<?> recordedChunk = subclass("NativeCarverVoxelChunk", chunk, chunkArgs, builder -> {
            override(builder, write, code -> {
                superCall(code, chunk, write);
                int old = saveResult(code, false);
                loadParameter(code, write.getParameterTypes(), 0);
                emit(code, "aload", old);
                loadParameter(code, write.getParameterTypes(), 1);
                loadParameter(code, write.getParameterTypes(), 2);
                callback(code, "recordVoxelWrite", "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;I)V");
                emit(code, "aload", old); emit(code, "areturn");
            });
            override(builder, mark, code -> {
                superCall(code, chunk, mark);
                loadParameter(code, mark.getParameterTypes(), 0);
                callback(code, "recordVoxelMark", "(Ljava/lang/Object;)V"); emit(code, "return_");
            });
        });
        voxelChunkConstructor = recordedChunk.getConstructor(chunkArgs);
        Class<?> mask = type("world.level.chunk.CarvingMask");
        Method get = mask.getMethod("get", int.class, int.class, int.class), set = mask.getMethod("set", int.class, int.class, int.class);
        Class<?> recordedMask = subclass("NativeCarverVoxelMask", mask, new Class<?>[]{int.class, int.class}, builder -> {
            override(builder, get, code -> {
                superCall(code, mask, get); int result = saveResult(code, true);
                for (int i = 0; i < 3; i++) loadParameter(code, get.getParameterTypes(), i);
                emit(code, "iload", result); callback(code, "recordMaskGet", "(IIIZ)V");
                emit(code, "iload", result); emit(code, "ireturn");
            });
            override(builder, set, code -> {
                superCall(code, mask, set);
                for (int i = 0; i < 3; i++) loadParameter(code, set.getParameterTypes(), i);
                callback(code, "recordMaskSet", "(III)V"); emit(code, "return_");
            });
        });
        voxelMaskConstructor = recordedMask.getConstructor(int.class, int.class);
        Class<?> carvingContext = type(CARVER + "CarvingContext");
        Class<?>[] contextArgs = {type("world.level.levelgen.NoiseBasedChunkGenerator"), type("core.RegistryAccess"),
            type("world.level.LevelHeightAccessor"), type("world.level.levelgen.NoiseChunk"), type("world.level.levelgen.RandomState"),
            type("world.level.levelgen.SurfaceRules$RuleSource")};
        Method top = carvingContext.getMethod("topMaterial", Function.class, type("world.level.chunk.ChunkAccess"), pos, boolean.class);
        Class<?> recordedContext = subclass("NativeCarverVoxelContext", carvingContext, contextArgs, builder ->
            override(builder, top, code -> {
                loadParameter(code, top.getParameterTypes(), 2); loadParameter(code, top.getParameterTypes(), 3);
                callback(code, "scriptedTopMaterial", "(Ljava/lang/Object;Z)Ljava/util/Optional;"); emit(code, "areturn");
            }));
        voxelContextConstructor = recordedContext.getConstructor(contextArgs);
        nativeBlock = method(CARVER + "WorldCarver", "carveBlock", carvingContext, type(CARVER + "CarverConfiguration"),
            type("world.level.chunk.ChunkAccess"), Function.class, mask, type("core.BlockPos$MutableBlockPos"),
            type("core.BlockPos$MutableBlockPos"), type("world.level.levelgen.Aquifer"), Class.forName("org.apache.commons.lang3.mutable.MutableBoolean"));
        for (int index : new int[]{0, 2}) {
            String kind = index == 0 ? "Cave" : "Canyon";
            Class<?> parent = type(CARVER + kind + "WorldCarver"), codec = Class.forName("com.mojang.serialization.Codec");
            Class<?> recorded = subclass("NativeCarverVoxel" + kind, parent, new Class<?>[]{codec}, builder ->
                override(builder, nativeBlock, code -> {
                    superCall(code, parent, nativeBlock); int result = saveResult(code, true);
                    loadParameter(code, nativeBlock.getParameterTypes(), 5); emit(code, "iload", result);
                    loadParameter(code, nativeBlock.getParameterTypes(), 8);
                    callback(code, "recordBlockResult", "(Ljava/lang/Object;ZLjava/lang/Object;)V");
                    emit(code, "iload", result); emit(code, "ireturn");
                }));
            voxelCarvers[index] = recorded.getConstructor(codec).newInstance(field(CARVER + kind + "CarverConfiguration", "CODEC"));
        }
        voxelCarvers[1] = voxelCarvers[0];
        nativeEllipsoid = method(CARVER + "WorldCarver", "carveEllipsoid", carvingContext, type(CARVER + "CarverConfiguration"),
            type("world.level.chunk.ChunkAccess"), Function.class, type("world.level.levelgen.Aquifer"),
            double.class, double.class, double.class, double.class, double.class, mask, type(CARVER + "WorldCarver$CarveSkipChecker"));
        nativeCaveSkip = method(CARVER + "CaveWorldCarver", "shouldSkip", double.class, double.class, double.class, double.class);
        nativeCanyonSkip = method(CARVER + "CanyonWorldCarver", "shouldSkip", carvingContext, float[].class,
            double.class, double.class, double.class, int.class);
        nativeWidths = method(CARVER + "CanyonWorldCarver", "initWidthFactors", carvingContext, type(CARVER + "CanyonCarverConfiguration"), type("util.RandomSource"));
    }

    record Reply(int state, boolean schedule) {
        List<Object> json() { return List.of(state, schedule); }
    }

    static final class VoxelCase {
        final String id;
        int cx, cz, fill, top = -1;
        boolean initialSchedule, initialSurface;
        Long surfaceSeed;
        boolean nativeAquifer;
        List<Reply> replies = List.of(new Reply(0, false));
        final List<List<Integer>> overrides = new ArrayList<>(), premarked = new ArrayList<>(), additional = new ArrayList<>();
        final List<Map<String, Object>> steps = new ArrayList<>();

        VoxelCase(String id) throws Exception { this.id = id; fill = stateId("STONE"); }
        VoxelCase fill(String block) throws Exception { fill = stateId(block); return this; }
        VoxelCase fluids(Reply... cycle) { replies = List.of(cycle); return this; }
        VoxelCase at(int x, int z) { cx = x; cz = z; return this; }
        VoxelCase put(int x, int y, int z, String block) throws Exception { return put(x, y, z, stateId(block)); }
        VoxelCase put(int x, int y, int z, int state) { overrides.add(List.of(x, y, z, state)); return this; }
        VoxelCase ellipse(double x, double y, double z, double hr, double vr, double floor, Long widthSeed) {
            Map<String, Object> step = new LinkedHashMap<>();
            step.put("kind", widthSeed == null ? "cave" : "canyon");
            step.put("center_bits", bits(cx * 16 + x, y, cz * 16 + z));
            step.put("radius_bits", bits(hr, vr)); step.put("floor_bits", bits(floor));
            if (widthSeed != null) step.put("width_seed", widthSeed);
            steps.add(step); return this;
        }
        VoxelCase ellipse(double x, double y, double z, double hr, double vr) { return ellipse(x, y, z, hr, vr, -1, null); }
        VoxelCase block(int x, int y, int z) {
            steps.add(Map.of("kind", "block", "position", List.of(cx * 16 + x, y, cz * 16 + z))); return this;
        }
    }

    static final class VoxelRun {
        final VoxelCase spec;
        final Object chunk, mask, ctx;
        final SurfaceColumn surfaceColumn;
        final Function<Object, Object> biomeGetter;
        final List<Object> events = new ArrayList<>();
        final Set<Pos> written = new HashSet<>();
        int draws;
        boolean schedule;

        VoxelRun(VoxelCase spec) throws Exception {
            this.spec = spec; schedule = spec.initialSchedule;
            chunk = voxelChunkConstructor.newInstance(make("world.level.ChunkPos", spec.cx, spec.cz), field("world.level.chunk.UpgradeData", "EMPTY"),
                height, factory(voxelState(spec.fill)), null);
            mask = voxelMaskConstructor.newInstance(384, -64);
            for (List<Integer> p : spec.overrides) {
                Object section = chunkSection.invoke(chunk, (p.get(1) + 64) >> 4);
                sectionSet.invoke(section, p.get(0), p.get(1) & 15, p.get(2), voxelState(p.get(3)), false);
            }
            // Non-air default palettes need native counters initialized before ProtoChunk.getBlockState.
            for (Object section : (Object[]) call(chunk, "getSections")) call(section, "recalcBlockCounts");
            if (stateId(call(chunk, "getBlockState", make("core.BlockPos", spec.cx * 16, -64, spec.cz * 16))) != spec.fill)
                throw new IllegalStateException("incorrect native fill for " + spec.id);
            for (List<Integer> p : spec.overrides)
                if (stateId(call(chunk, "getBlockState", make("core.BlockPos", spec.cx * 16 + p.get(0), p.get(1), spec.cz * 16 + p.get(2)))) != p.get(3))
                    throw new IllegalStateException("incorrect native override for " + spec.id + ": " + p);
            for (List<Integer> p : spec.premarked) call(mask, "set", p.get(0), p.get(1), p.get(2));
            Object extra = Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type("world.level.chunk.CarvingMask$Mask")}, (p, m, a) -> {
                if (!m.getName().equals("test")) throw new UnsupportedOperationException(m.toString());
                return spec.additional.contains(List.of((int) a[0], (int) a[1], (int) a[2]));
            });
            call(mask, "setAdditionalMask", extra);
            for (Reply reply : spec.replies) voxelState(reply.state());
            voxelState(spec.top);
            surfaceColumn = spec.surfaceSeed == null ? null : new SurfaceColumn(surfaceSeed(spec.surfaceSeed), chunk, null);
            ctx = surfaceColumn == null ? voxelContextConstructor.newInstance(generator, registries, height, null, null, null) : surfaceColumn.ctx;
            biomeGetter = surfaceColumn == null ? p -> plains : surfaceColumn.biomeGetter;
        }

        Object aquifer() throws Exception {
            Object nativeFluid = spec.nativeAquifer ? call(surfaceColumn.noiseChunk, "aquifer") : null;
            Method compute = type("world.level.levelgen.Aquifer").getMethod("computeSubstance",
                type("world.level.levelgen.DensityFunction$FunctionContext"), double.class);
            return Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type("world.level.levelgen.Aquifer")}, (p, m, a) -> {
                if (m.getName().equals("shouldScheduleFluidUpdate")) {
                    events.add(List.of("schedule", schedule)); return schedule;
                }
                if (!m.getName().equals("computeSubstance")) throw new UnsupportedOperationException(m.toString());
                Reply reply;
                if (nativeFluid == null) reply = spec.replies.get(draws % spec.replies.size());
                else reply = new Reply(stateId(compute.invoke(nativeFluid, a)), (boolean) call(nativeFluid, "shouldScheduleFluidUpdate"));
                draws++; schedule = reply.schedule();
                events.add(List.of("aquifer", call(a[0], "blockX"), call(a[0], "blockY"), call(a[0], "blockZ"), bits((double) a[1]), reply.state(), schedule));
                return voxelState(reply.state());
            });
        }

        List<Integer> maskIndices() throws Exception {
            return BitSet.valueOf((long[]) call(mask, "toArray")).stream().boxed().toList();
        }

        Object skip(Map<String, Object> step) throws Exception {
            boolean canyon = step.get("kind").equals("canyon");
            float[] widths = canyon ? (float[]) nativeWidths.invoke(voxelCarvers[2], ctx, configs[2],
                call(type("util.RandomSource"), "createThreadLocalInstance", (long) step.get("width_seed"))) : null;
            double floor = Double.longBitsToDouble(Long.parseUnsignedLong((String) step.get("floor_bits"), 16));
            return Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type(CARVER + "WorldCarver$CarveSkipChecker")}, (p, m, a) -> {
                if (!m.getName().equals("shouldSkip")) throw new UnsupportedOperationException(m.toString());
                return canyon ? nativeCanyonSkip.invoke(voxelCarvers[2], a[0], widths, a[1], a[2], a[3], a[4])
                    : nativeCaveSkip.invoke(null, a[1], a[2], a[3], floor);
            });
        }

        Map<String, Object> capture() throws Exception {
            Object fluid = aquifer(), flag = Class.forName("org.apache.commons.lang3.mutable.MutableBoolean").getConstructor(boolean.class).newInstance(spec.initialSurface);
            List<Boolean> returns = new ArrayList<>();
            List<Object> masks = new ArrayList<>();
            List<Integer> initialMask = maskIndices();
            active = this;
            try {
                for (int i = 0; i < spec.steps.size(); i++) {
                    Map<String, Object> step = spec.steps.get(i);
                    events.add(List.of("pass", i));
                    boolean result;
                    if (step.get("kind").equals("block")) {
                        List<?> p = (List<?>) step.get("position");
                        Object cursor = make("core.BlockPos$MutableBlockPos", p.get(0), p.get(1), p.get(2));
                        result = (boolean) nativeBlock.invoke(voxelCarvers[0], ctx, configs[0], chunk, biomeGetter,
                            mask, cursor, make("core.BlockPos$MutableBlockPos"), fluid, flag);
                    } else {
                        int index = step.get("kind").equals("canyon") ? 2 : 0;
                        List<?> c = (List<?>) step.get("center_bits"), r = (List<?>) step.get("radius_bits");
                        result = (boolean) nativeEllipsoid.invoke(voxelCarvers[index], ctx, configs[index], chunk, biomeGetter,
                            fluid, fromBits(c.get(0)), fromBits(c.get(1)), fromBits(c.get(2)), fromBits(r.get(0)), fromBits(r.get(1)), mask, skip(step));
                    }
                    returns.add(result); events.add(List.of("return", i, result)); masks.add(maskIndices());
                }
            } finally { active = null; }
            Set<Pos> checked = new HashSet<>(written);
            for (List<Integer> p : spec.overrides) checked.add(new Pos(spec.cx * 16 + p.get(0), p.get(1), spec.cz * 16 + p.get(2)));
            for (Object p : ((java.util.stream.Stream<?>) call(mask, "stream", call(chunk, "getPos"))).toList()) checked.add(Pos.from(p));
            List<Object> finalStates = new ArrayList<>();
            for (Pos p : checked.stream().sorted(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z)).toList())
                finalStates.add(List.of(p.x(), p.y(), p.z(), stateId(call(chunk, "getBlockState", make("core.BlockPos", p.x(), p.y(), p.z())))));
            List<Object> posts = new ArrayList<>();
            Method unpack = type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates", short.class, int.class, type("world.level.ChunkPos"));
            Object[] sections = (Object[]) call(chunk, "getPostProcessing");
            for (int section = 0; section < sections.length; section++) if (sections[section] != null)
                for (Object packed : (Iterable<?>) sections[section]) {
                    Pos p = Pos.from(unpack.invoke(null, packed, section - 4, call(chunk, "getPos")));
                    posts.add(List.of(p.x(), p.y(), p.z()));
                }
            Map<String, Object> sample = new LinkedHashMap<>();
            sample.put("op", surfaceColumn == null ? "voxel" : "surface_voxel"); sample.put("id", spec.id); sample.put("target", List.of(spec.cx, spec.cz));
            sample.put("fill", spec.fill); sample.put("overrides", spec.overrides); sample.put("premarked", spec.premarked); sample.put("additional_mask", spec.additional);
            sample.put("aquifer", Map.of("mode", "scripted_cycle", "initial_schedule", spec.initialSchedule, "replies", spec.replies.stream().map(Reply::json).toList()));
            sample.put("surface", Map.of("mode", "scripted_constant", "state", spec.top)); sample.put("initial_surface", spec.initialSurface);
            if (surfaceColumn != null) sample.put("surface", Map.of("mode", "native_overworld", "seed", spec.surfaceSeed, "contexts", surfaceColumn.contexts));
            if (spec.nativeAquifer) sample.put("aquifer", Map.of("mode", "native_noise_chunk", "initial_schedule", false));
            sample.put("steps", spec.steps); sample.put("returns", returns); sample.put("events", events);
            sample.put("initial_mask", initialMask); sample.put("masks", masks); sample.put("final_states", finalStates); sample.put("postprocessing", posts);
            sample.put("aquifer_calls", draws); sample.put("final_schedule", schedule);
            return sample;
        }
    }

    static double fromBits(Object value) { return Double.longBitsToDouble(Long.parseUnsignedLong((String) value, 16)); }

    static void voxelSamples(List<Object> samples) throws Exception {
        bootstrapVoxels();
        int air = stateId("AIR"), water = stateId("WATER"), lava = stateId("LAVA");
        Reply dry = new Reply(air, false), wet = new Reply(water, true), barrier = new Reply(-1, true);
        List<VoxelCase> cases = new ArrayList<>();
        Reply[] individual = {dry, new Reply(air, true), new Reply(water, false), wet, new Reply(lava, false), new Reply(lava, true), barrier};
        for (int i = 0; i < individual.length; i++)
            cases.add(new VoxelCase("voxel/fluid/" + i).fluids(individual[i]).ellipse(8.5, 32.5, 8.5, 2.25, 2.75));
        cases.add(new VoxelCase("voxel/fluid/mixed_order").fluids(new Reply(air, true), new Reply(water, false), barrier, new Reply(lava, true), wet)
            .ellipse(8.5, 32.5, 8.5, 2.25, 2.75));
        for (String block : new String[]{"BEDROCK", "AIR", "LAVA", "CAVE_AIR", "OAK_LOG"}) {
            VoxelCase c = block.equals("CAVE_AIR")
                ? new VoxelCase("voxel/rejected/" + block).put(8, 33, 8, block).fluids(wet).ellipse(8.5, 32.5, 8.5, 0.6, 0.6)
                : new VoxelCase("voxel/rejected/" + block).fill(block).fluids(wet).ellipse(8.5, 32.5, 8.5, 1.25, 1.75);
            c.initialSchedule = true; cases.add(c);
        }
        for (String block : new String[]{"TERRACOTTA", "RED_SAND", "MUD", "POWDER_SNOW"})
            cases.add(new VoxelCase("voxel/replaceable/" + block).fill(block).fluids(dry, wet, barrier).ellipse(8.5, 32.5, 8.5, 1.25, 1.75));
        for (int id : new int[]{27923, 93}) {
            VoxelCase c = new VoxelCase("voxel/replaceable/state_" + id).fluids(dry, wet).ellipse(8.5, 32.5, 8.5, 1.25, 1.75);
            c.fill = id; cases.add(c);
        }
        cases.add(new VoxelCase("voxel/clip/min_x").fluids(wet).ellipse(-0.25, 32.5, 0.5, 1.25, 1.75));
        cases.add(new VoxelCase("voxel/clip/negative_chunk").at(-1, -1).fluids(wet).ellipse(15.75, 32.5, 15.75, 1.25, 1.75));
        cases.add(new VoxelCase("voxel/clip/mixed_sign_chunk").at(-17, 29).fluids(wet).ellipse(0.5, 32.5, 15.5, 1.25, 1.75));
        cases.add(new VoxelCase("voxel/clip/world_border").at(1874999, -1875000).fluids(wet).ellipse(15.5, 32.5, 0.5, 1.25, 1.75));
        cases.add(new VoxelCase("voxel/clip/out_of_reach").ellipse(28.5, 32.5, 8.5, 1.5, 1.5));
        cases.add(new VoxelCase("voxel/clip/no_overlap").ellipse(18.5, 32.5, 8.5, 1.5, 1.5));
        cases.add(new VoxelCase("voxel/clip/tangent").ellipse(8, 32.5, 8, 0.5, 0.5));
        for (int y : new int[]{-65, -64, -63, -62, -57, -56, -55, -54, 311, 312, 313, 319, 320})
            cases.add(new VoxelCase("voxel/y/" + y).fluids(wet).ellipse(8.5, y + 0.5, 8.5, 0.6, 1.75));
        VoxelCase inherited = new VoxelCase("voxel/lava/inherited_true_flag").ellipse(8.5, -61.5, 8.5, 0.6, 1.75);
        inherited.initialSchedule = true; cases.add(inherited);
        for (double floor : new double[]{-1, (double) -0.4f, 0, 1})
            cases.add(new VoxelCase("voxel/floor/" + bits(floor)).ellipse(8.5, 32.5, 8.5, 1.25, 2, floor, null));
        cases.add(new VoxelCase("voxel/mask/repeat").fluids(wet).ellipse(8.5, 32.5, 8.5, 1.25, 1.75).ellipse(8.5, 32.5, 8.5, 1.25, 1.75));
        cases.add(new VoxelCase("voxel/mask/overlap").fluids(dry, wet, barrier)
            .ellipse(7.5, 32.5, 8.5, 1.5, 2).ellipse(9.5, 32.5, 8.5, 1.5, 2).ellipse(8.5, 32.5, 8.5, 1.5, 2));
        cases.add(new VoxelCase("voxel/mask/barrier_is_not_retried").fluids(barrier, dry)
            .ellipse(8.5, 32.5, 8.5, 0.6, 0.6).ellipse(8.5, 32.5, 8.5, 0.6, 0.6).ellipse(9.5, 32.5, 8.5, 0.6, 0.6));
        VoxelCase premarked = new VoxelCase("voxel/mask/wrapped_premark").at(-1, 1).fluids(wet).ellipse(15.5, 32.5, 0.5, 0.6, 2);
        premarked.premarked.addAll(List.of(List.of(-1, 33, 16), List.of(15, 34, 0))); cases.add(premarked);
        VoxelCase additional = new VoxelCase("voxel/mask/additional_callback").fluids(wet).ellipse(8.5, 32.5, 8.5, 1.5, 2);
        additional.additional.addAll(List.of(List.of(8, 33, 8), List.of(9, 33, 8))); cases.add(additional);
        for (long seed : new long[]{0, 17}) for (double y : new double[]{32.5, -61.5})
            cases.add(new VoxelCase("voxel/canyon/" + seed + "/" + y).fluids(dry, wet).ellipse(8.5, y, 8.5, 1.5, 2.25, -1, seed));
        for (String mode : new String[]{"none", "barrier", "wet_callback", "mycelium", "podzol", "column_reset"}) {
            VoxelCase c = new VoxelCase("voxel/surface/" + mode).fill("DIRT")
                .put(8, 34, 8, mode.equals("mycelium") ? "MYCELIUM" : mode.equals("podzol") ? "PODZOL" : "GRASS_BLOCK")
                .ellipse(8.5, 32.5, 8.5, mode.equals("column_reset") ? 1.6 : 0.6, 2);
            if (mode.equals("barrier")) c.fluids(barrier, dry);
            if (mode.equals("wet_callback") || mode.equals("podzol")) c.top = water;
            if (mode.equals("mycelium")) { c.fluids(wet); c.top = stateId("STONE"); }
            cases.add(c);
        }
        cases.add(new VoxelCase("voxel/block/rejected").fill("BEDROCK").fluids(wet).block(8, 32, 8));
        cases.add(new VoxelCase("voxel/block/grass_barrier_flag").fill("DIRT").put(8, 34, 8, "GRASS_BLOCK").fluids(barrier, dry)
            .block(8, 34, 8).block(8, 33, 8));
        VoxelCase carried = new VoxelCase("voxel/block/lava_carried_flag").fluids(new Reply(water, false))
            .block(8, -57, 8).block(8, -56, 8).block(8, -55, 8).block(9, -57, 8).block(8, -57, 8);
        carried.initialSchedule = true; cases.add(carried);
        cases.add(new VoxelCase("voxel/block/lava_after_aquifer").fluids(wet).block(8, -55, 8).block(8, -56, 8));
        VoxelCase same = new VoxelCase("voxel/block/same_state_and_premark").put(8, 32, 8, "WATER").fluids(wet).block(8, 32, 8).block(8, 32, 8);
        same.premarked.add(List.of(8, 32, 8)); cases.add(same);
        VoxelCase flagged = new VoxelCase("voxel/block/caller_surface_flag").fill("DIRT").block(8, 32, 8);
        flagged.initialSurface = true; flagged.top = water; cases.add(flagged);
        cases.add(new VoxelCase("voxel/block/build_limits").block(8, -65, 8).block(8, -64, 8).block(8, 319, 8).block(8, 320, 8));
        for (VoxelCase spec : cases) samples.add(new VoxelRun(spec).capture());
        voxelState(lava); voxelState(stateId("VOID_AIR"));
    }

    static final Map<Long, SurfaceSeed> surfaceSeeds = new HashMap<>();
    static final String SURFACE_CONTEXT = "world.level.levelgen.SurfaceRules$Context";

    static Object member(Object target, String owner, String name) throws Exception {
        Field field = type(owner).getDeclaredField(name);
        field.setAccessible(true);
        return field.get(target);
    }

    static SurfaceSeed surfaceSeed(long seed) throws Exception {
        SurfaceSeed result = surfaceSeeds.get(seed);
        if (result == null) { result = new SurfaceSeed(seed); surfaceSeeds.put(seed, result); }
        return result;
    }

    static final class SurfaceSeed {
        final long seed;
        final Object settings, randomState, system, source, generator, rule, picker, biomeManager;

        SurfaceSeed(long seed) throws Exception {
            this.seed = seed;
            Object holder = registryValue("NOISE_SETTINGS", "minecraft:overworld");
            settings = call(holder, "value");
            randomState = call(type("world.level.levelgen.RandomState"), "create", settings,
                call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE")), seed);
            system = call(randomState, "surfaceSystem");
            rule = call(settings, "surfaceRule");
            Object parameters = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
                field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"),
                call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME")));
            source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", parameters));
            generator = make("world.level.levelgen.NoiseBasedChunkGenerator", source, holder);
            Object sampler = call(randomState, "sampler");
            Object rawBiomes = Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type("world.level.biome.BiomeManager$NoiseBiomeSource")}, (p, m, a) -> {
                if (!m.getName().equals("getNoiseBiome")) throw new UnsupportedOperationException(m.toString());
                return call(source, "getNoiseBiome", a[0], a[1], a[2], sampler);
            });
            biomeManager = make("world.level.biome.BiomeManager", rawBiomes, call(type("world.level.biome.BiomeManager"), "obfuscateSeed", seed));
            Object lava = make("world.level.levelgen.Aquifer$FluidStatus", -54, state("LAVA"));
            Object water = make("world.level.levelgen.Aquifer$FluidStatus", 63, state("WATER"));
            picker = Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type("world.level.levelgen.Aquifer$FluidPicker")}, (p, m, a) -> {
                if (!m.getName().equals("computeFluid")) throw new UnsupportedOperationException(m.toString());
                return (int) a[1] < -54 ? lava : water;
            });
        }
    }

    static final class SurfaceColumn {
        final SurfaceSeed env;
        final Object chunk, noiseChunk, ctx;
        final Function<Object, Object> biomeGetter;
        final List<Object> contexts = new ArrayList<>();
        Object observedContext;
        List<Integer> quart;

        SurfaceColumn(SurfaceSeed env, Object chunk, String fixedBiome) throws Exception {
            this.env = env; this.chunk = chunk;
            noiseChunk = call(type("world.level.levelgen.NoiseChunk"), "forChunk", chunk, env.randomState,
                field("world.level.levelgen.DensityFunctions$BeardifierMarker", "INSTANCE"), env.settings, env.picker,
                call(type("world.level.levelgen.blending.Blender"), "empty"));
            Object fixed = fixedBiome == null ? null : registryValue("BIOME", fixedBiome);
            Object sampler = call(env.randomState, "sampler");
            Object source = Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type("world.level.biome.BiomeManager$NoiseBiomeSource")}, (p, m, a) -> {
                if (!m.getName().equals("getNoiseBiome")) throw new UnsupportedOperationException(m.toString());
                quart = List.of((int) a[0], (int) a[1], (int) a[2]);
                return fixed != null ? fixed : call(env.source, "getNoiseBiome", a[0], a[1], a[2], sampler);
            });
            Object manager = make("world.level.biome.BiomeManager", source, call(type("world.level.biome.BiomeManager"), "obfuscateSeed", env.seed));
            biomeGetter = p -> {
                try { return call(manager, "getBiome", p); }
                catch (Exception e) { throw new RuntimeException(e); }
            };
            Object rule = Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type("world.level.levelgen.SurfaceRules$RuleSource")}, (p, m, a) -> {
                if (!m.getName().equals("apply")) throw new UnsupportedOperationException(m.toString());
                observedContext = a[0];
                Object nativeRule = call(env.rule, "apply", a[0]);
                Method apply = type("world.level.levelgen.SurfaceRules$SurfaceRule").getMethod("tryApply", int.class, int.class, int.class);
                return Proxy.newProxyInstance(CarverReference.class.getClassLoader(), new Class<?>[]{type("world.level.levelgen.SurfaceRules$SurfaceRule")}, (q, n, b) -> {
                    if (!n.getName().equals("tryApply")) throw new UnsupportedOperationException(n.toString());
                    Object result = apply.invoke(nativeRule, b);
                    if (active != null) {
                        boolean wet = (int) member(observedContext, SURFACE_CONTEXT, "waterHeight") != Integer.MIN_VALUE;
                        active.events.add(List.of("top_material", b[0], b[1], b[2], wet, stateId(result)));
                        contexts.add(snapshot());
                    }
                    return result;
                });
            });
            ctx = make(CARVER + "CarvingContext", env.generator, registries, height, noiseChunk, env.randomState, rule);
        }

        Map<String, Object> snapshot() throws Exception {
            Map<String, Object> result = new LinkedHashMap<>();
            for (String name : List.of("blockX", "blockY", "blockZ", "stoneDepthAbove", "stoneDepthBelow", "waterHeight", "surfaceDepth"))
                result.put(name, member(observedContext, SURFACE_CONTEXT, name));
            result.put("surface_secondary_bits", bits((double) call(observedContext, "getSurfaceSecondary")));
            result.put("min_surface_level", call(observedContext, "getMinSurfaceLevel"));
            result.put("preliminary_corners", member(observedContext, SURFACE_CONTEXT, "preliminarySurfaceCache"));
            Object holder = ((java.util.function.Supplier<?>) member(observedContext, SURFACE_CONTEXT, "biome")).get();
            result.put("biome", call(call(holder, "key"), "identifier").toString());
            result.put("quart", quart);
            Object biome = call(holder, "value"), position = member(observedContext, SURFACE_CONTEXT, "pos");
            result.put("temperature_bits", bits((float) call(biome, "getHeightAdjustedTemperature", position, 63)));
            result.put("steep", call(member(observedContext, SURFACE_CONTEXT, "steep"), "test"));
            return result;
        }

        Map<String, Object> capture(String id, int x, int y, int z, boolean wet, String fixedBiome, String profile) throws Exception {
            Object position = make("core.BlockPos", x, y, z);
            Optional<?> result = (Optional<?>) call(ctx, "topMaterial", biomeGetter, chunk, position, wet);
            Object state = result.orElse(null);
            Map<String, Object> sample = new LinkedHashMap<>();
            sample.put("op", "top_material"); sample.put("id", id); sample.put("seed", env.seed);
            sample.put("position", List.of(x, y, z)); sample.put("under_fluid", wet);
            sample.put("source", fixedBiome == null ? "overworld" : fixedBiome); sample.put("height_profile", profile);
            sample.put("context", snapshot()); sample.put("result", stateId(state));
            sample.put("has_fluid", state != null && !(boolean) call(call(state, "getFluidState"), "isEmpty"));
            return sample;
        }
    }

    static Object surfaceChunk(int x, int z, String profile) throws Exception {
        Object chunk = voxelChunkConstructor.newInstance(make("world.level.ChunkPos", x >> 4, z >> 4),
            field("world.level.chunk.UpgradeData", "EMPTY"), height, airFactory, null);
        for (int lx = 0; lx < 16; lx++) for (int lz = 0; lz < 16; lz++) {
            int y = switch (profile) {
                case "south_up" -> 64 + lz * 2;
                case "north_up" -> 94 - lz * 2;
                case "west_up" -> 94 - lx * 2;
                case "east_up" -> 64 + lx * 2;
                case "flat" -> 80;
                default -> throw new IllegalArgumentException(profile);
            };
            call(chunk, "setBlockState", make("core.BlockPos", (x & ~15) + lx, y, (z & ~15) + lz), state("STONE"), 3);
        }
        return chunk;
    }

    static Map<String, Object> surfaceSamples(List<Object> samples) throws Exception {
        SurfaceSeed env = surfaceSeed(0);
        Object jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        Object ruleJson = call(call(field("world.level.levelgen.SurfaceRules$RuleSource", "CODEC"), "encodeStart", jsonOps, env.rule), "getOrThrow");
        List<String> biomes = new ArrayList<>();
        for (Object holder : (Set<?>) call(env.source, "possibleBiomes")) biomes.add(call(call(holder, "key"), "identifier").toString());
        Collections.sort(biomes);
        for (String biome : biomes) {
            SurfaceColumn column = new SurfaceColumn(env, surfaceChunk(8, 8, "flat"), biome);
            for (int y : new int[]{62, 64, 100, 180}) for (boolean wet : new boolean[]{false, true})
                samples.add(column.capture("surface/fixed/" + biome + "/" + y + "/" + wet, 8, y, 8, wet, biome, "flat"));
        }
        for (long seed : new long[]{0, -1, 846692123413862008L}) {
            env = surfaceSeed(seed);
            for (int[] p : new int[][]{{0, 0}, {15, 15}, {-1, -1}, {16, -17}, {-2001, 3007}, {29999983, -29999983}}) {
                SurfaceColumn column = new SurfaceColumn(env, surfaceChunk(p[0], p[1], "flat"), null);
                Map<String, Object> first = column.capture("surface/overworld/" + seed + "/" + p[0] + "/" + p[1] + "/probe", p[0], 64, p[1], false, null, "flat");
                int minSurface = (int) ((Map<?, ?>) first.get("context")).get("min_surface_level");
                samples.add(first);
                Set<Integer> ys = new TreeSet<>(List.of(-64, -63, -60, -59, 61, 62, 63, minSurface - 1, minSurface, minSurface + 1, 180, 319));
                for (int y : ys) for (boolean wet : new boolean[]{false, true})
                    samples.add(column.capture("surface/overworld/" + seed + "/" + p[0] + "/" + p[1] + "/" + y + "/" + wet, p[0], y, p[1], wet, null, "flat"));
            }
        }
        for (String profile : List.of("south_up", "north_up", "west_up", "east_up")) for (int[] p : new int[][]{{8, 8}, {0, 0}, {15, 15}}) {
            String biome = "minecraft:snowy_slopes";
            SurfaceColumn column = new SurfaceColumn(surfaceSeed(0), surfaceChunk(p[0], p[1], profile), biome);
            samples.add(column.capture("surface/steep/" + profile + "/" + p[0], p[0], 180, p[1], false, biome, profile));
        }
        for (long seed : new long[]{0, -1, 846692123413862008L}) for (boolean nativeFluid : new boolean[]{false, true}) for (int[] p : new int[][]{{8, 65, 8}, {-1, 63, -1}, {-2001, 103, 3007}, {16, 34, -17}}) {
            VoxelCase c = new VoxelCase("surface/voxel/" + seed + "/" + p[0] + "/" + nativeFluid).fill("DIRT").at(p[0] >> 4, p[2] >> 4);
            c.surfaceSeed = seed; c.nativeAquifer = nativeFluid;
            int x = p[0] & 15, z = p[2] & 15;
            c.put(x, p[1], z, "GRASS_BLOCK").fluids(new Reply(stateId("AIR"), false), new Reply(stateId("WATER"), true));
            c.ellipse(x + 0.5, p[1] - 1.5, z + 0.5, 0.6, 2).block(x, p[1] - 3, z);
            samples.add(new VoxelRun(c).capture());
        }
        surfaceHoleWitnesses(samples);
        surfaceWaterWitness(samples);
        for (long seed : SEEDS) {
            List<Integer> bands = new ArrayList<>();
            for (Object state : (Object[]) member(surfaceSeed(seed).system, "world.level.levelgen.SurfaceSystem", "clayBands")) bands.add(stateId(state));
            samples.add(Map.of("op", "surface_bands", "id", "surface/bands/" + seed, "seed", seed, "states", bands));
        }
        Map<String, Object> palette = new TreeMap<>();
        surfacePalette(ruleJson, palette, jsonOps);
        Map<String, Object> climates = new TreeMap<>();
        Object biomeRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        for (Object biome : (Iterable<?>) biomeRegistry) {
            Object settings = member(biome, "world.level.biome.Biome", "climateSettings");
            boolean frozen = ((Enum<?>) call(settings, "temperatureModifier")).name().equals("FROZEN");
            climates.put(call(biomeRegistry, "getKey", biome).toString(), List.of(bits((float) call(biome, "getBaseTemperature")), frozen));
        }
        return Map.of("surface_rule", ruleJson, "biomes", biomes, "palette", palette.values(), "biome_climates", climates,
            "environment", "Native CarvingContext.topMaterial, RandomState, NoiseChunk.forChunk and SurfaceSystem; empty Blender; sparse prescribed height columns or voxel fill; no terrain generation or ticks",
            "biome_source", "BiomeManager(obfuscateSeed(worldSeed)) over RandomState.sampler and MultiNoiseBiomeSource; fixed-biome cases are explicitly labeled");
    }

    static void surfacePalette(Object value, Map<String, Object> palette, Object ops) throws Exception {
        if ((boolean) call(value, "isJsonArray")) {
            for (Object child : (Iterable<?>) call(value, "getAsJsonArray")) surfacePalette(child, palette, ops);
        } else if ((boolean) call(value, "isJsonObject")) {
            Object object = call(value, "getAsJsonObject");
            Object spec = call(object, "get", "result_state");
            if (spec != null) {
                Object state = call(call(field("world.level.block.state.BlockState", "CODEC"), "parse", ops, spec), "getOrThrow");
                palette.put(spec.toString(), Map.of("spec", spec, "state", stateId(state), "has_fluid", !(boolean) call(call(state, "getFluidState"), "isEmpty")));
            }
            for (Object entry : (Set<?>) call(object, "entrySet")) surfacePalette(((Map.Entry<?, ?>) entry).getValue(), palette, ops);
        }
    }

    static void surfaceHoleWitnesses(List<Object> samples) throws Exception {
        SurfaceSeed env = surfaceSeed(0);
        Random random = new Random(261);
        Set<Integer> results = new HashSet<>();
        for (int i = 0; i < 16384 && results.size() < 2; i++) {
            int x = random.nextInt(2048) - 1024, z = random.nextInt(2048) - 1024;
            if ((int) call(env.system, "getSurfaceDepth", x, z) > 0) continue;
            String biome = "minecraft:frozen_ocean";
            SurfaceColumn column = new SurfaceColumn(env, surfaceChunk(x, z, "flat"), biome);
            Map<String, Object> probe = column.capture("probe", x, 64, z, true, biome, "flat");
            int y = Math.max(64, (int) ((Map<?, ?>) probe.get("context")).get("min_surface_level"));
            Map<String, Object> wet = column.capture("surface/hole/" + i + "/wet", x, y, z, true, biome, "flat");
            int result = (int) wet.get("result");
            if ((result == stateId("ICE") || result == stateId("WATER")) && results.add(result)) {
                samples.add(wet);
                samples.add(column.capture("surface/hole/" + i + "/dry", x, y, z, false, biome, "flat"));
            }
        }
        if (results.size() != 2) throw new IllegalStateException("missing native cold/warm surface hole witnesses");
    }

    static void surfaceWaterWitness(List<Object> samples) throws Exception {
        SurfaceSeed env = surfaceSeed(0);
        Random random = new Random(775);
        for (int i = 0; i < 16384; i++) {
            int x = random.nextInt(8192) - 4096, z = random.nextInt(8192) - 4096;
            Object holder = call(env.biomeManager, "getBiome", make("core.BlockPos", x, 62, z));
            String biome = call(call(holder, "key"), "identifier").toString();
            if (!biome.endsWith("swamp")) continue;
            SurfaceColumn column = new SurfaceColumn(env, surfaceChunk(x, z, "flat"), null);
            Map<String, Object> sample = column.capture("surface/water_witness", x, 62, z, false, null, "flat");
            if ((int) sample.get("result") != stateId("WATER")) continue;
            samples.add(sample);
            for (boolean nativeFluid : new boolean[]{false, true}) {
                VoxelCase c = new VoxelCase("surface/voxel/water_witness/" + nativeFluid).fill("DIRT").at(x >> 4, z >> 4);
                c.surfaceSeed = 0L; c.nativeAquifer = nativeFluid;
                c.put(x & 15, 63, z & 15, "GRASS_BLOCK").ellipse((x & 15) + 0.5, 62.5, (z & 15) + 0.5, 0.6, 0.6);
                samples.add(new VoxelRun(c).capture());
            }
            return;
        }
        throw new IllegalStateException("missing native overworld water top-material witness");
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 0) throw new IllegalArgumentException("usage: CarverReference");
        bootstrap();
        List<Object> samples = new ArrayList<>();
        reachSamples(samples);
        rngSamples(samples);
        for (int index = 0; index < KINDS.length; index++) for (long seed : new long[]{0, 1, 17, 42})
            samples.add(carveSample("carve/" + KINDS[index] + "/" + seed, seed, index, 0, 0, 0, 0, true));
        samples.add(carveSample("carve/neighbor_cave", 42, 0, -1, 1, 0, 0, true));
        samples.add(carveSample("carve/neighbor_canyon", 17, 2, -1, 1, 0, 0, true));
        samples.add(carveSample("carve/border_canyon", 42, 2, 1874999, -1875000, 1874999, -1875000, true));
        samples.add(carveSample("carve/canyon_length_witness", CANYON_WITNESS, 2, 0, 0, 0, 0, false));
        for (long seed : new long[]{0, 1, 17, 42}) for (float thickness : new float[]{0.75f, 2.3f})
            samples.add(tunnelSample(seed, thickness));
        voxelSamples(samples);
        Map<String, Object> surfaceMetadata = surfaceSamples(samples);
        Object jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        Object builtins = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", field("core.registries.BuiltInRegistries", "REGISTRY"));
        Object ops = call(type("resources.RegistryOps"), "create", jsonOps, builtins);
        Map<String, Object> configurations = new LinkedHashMap<>();
        for (int i = 0; i < KINDS.length; i++) configurations.put(KINDS[i],
            call(call(field(CARVER + "ConfiguredWorldCarver", "DIRECT_CODEC"), "encodeStart", ops, configured[i]), "getOrThrow"));
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("CARVER_REFERENCE=" + call(gson, "toJson", Map.of("samples", samples,
            "replaceability", predicateTables(), "configurations", configurations,
            "geometry_fields", List.of("x", "y", "z", "horizontal_radius", "vertical_radius"), "surface_metadata", surfaceMetadata,
            "voxel_metadata", Map.of("min_y", -64, "height", 384, "null_state", -1, "void_air_state", stateId("VOID_AIR"),
                "lava_state", stateId("LAVA"), "fluid_states", fluidStates,
                "environment", "Native ProtoChunk sections/setters at EMPTY; scripted aquifer cycle and surface callback; no ticks",
                "mask_coordinates", "local X/Z, absolute Y; additional_mask is a scripted callback, not retrogen geometry",
                "postprocessing_order", "native section order, preserving insertion order and duplicates inside each section"))));
    }
}
