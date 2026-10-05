import java.lang.constant.*;
import java.lang.invoke.MethodHandles;
import java.lang.reflect.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Consumer;
import java.util.function.Function;

/** Actual WorldGenRegion writes into constructed ProtoChunks and typed hive entities.
 * The region's server bootstrap is bypassed; its setBlock/getBlockEntity/ensureCanWrite
 * methods are unmodified. Only the server's unrelated POI callback is record-only.
 * Compile with TreeReference, NativeEntityLevel and NativeWorldgenRegistries on JDK 21;
 * execute on the pinned JAR's Java 25 runtime.
 */
public class TreeEffectReference extends TreeReference {
    static final String REGION = "server.level.WorldGenRegion";
    static final String STATUS = "world.level.chunk.status.ChunkStatus";
    static final String HIVE = "world.level.block.entity.BeehiveBlockEntity";
    static Object registries, factory, height, hiveType, level, jsonOps;
    static int poiCallbacks;

    record Action(String op, Object value) {}
    record Scenario(String name, Object initial, boolean resolveInitial, List<Action> actions) {}

    static Object allocate(Class<?> type) throws Exception {
        Class<?> unsafe = Class.forName("sun.misc.Unsafe");
        Field singleton = unsafe.getDeclaredField("theUnsafe");
        singleton.setAccessible(true);
        return unsafe.getMethod("allocateInstance", Class.class).invoke(singleton.get(null), type);
    }

    static void setField(Object object, String owner, String name, Object value) throws Exception {
        Field field = type(owner).getDeclaredField(name);
        field.setAccessible(true);
        field.set(object, value);
    }

    public static void recordPoi(Object pos, Object before, Object after) {
        poiCallbacks++;
    }

    static Object probeLevel() throws Exception {
        Class<?> parent = NativeEntityLevel.create().getClass();
        Consumer<Object> build = builder -> {
            try {
                NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withFlags", Modifier.PUBLIC);
                NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withSuperclass", ClassDesc.of(parent.getName()));
                Consumer<Object> body = code -> {
                    try {
                        for (int slot = 1; slot <= 3; slot++)
                            NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "aload", slot);
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "invokestatic",
                            ClassDesc.of("TreeEffectReference"), "recordPoi",
                            MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object,
                                ConstantDescs.CD_Object, ConstantDescs.CD_Object));
                        NativeEntityLevel.api("java.lang.classfile.CodeBuilder", code, "return_");
                    } catch (Exception e) { throw new RuntimeException(e); }
                };
                NativeEntityLevel.api("java.lang.classfile.ClassBuilder", builder, "withMethodBody",
                    "updatePOIOnBlockStateChange", MethodTypeDesc.of(ConstantDescs.CD_void,
                        ClassDesc.of(MC + "core.BlockPos"), ClassDesc.of(MC + "world.level.block.state.BlockState"),
                        ClassDesc.of(MC + "world.level.block.state.BlockState")), Modifier.PUBLIC, body);
            } catch (Exception e) { throw new RuntimeException(e); }
        };
        Object api = NativeEntityLevel.api("java.lang.classfile.ClassFile", null, "of");
        byte[] bytes = (byte[]) NativeEntityLevel.api("java.lang.classfile.ClassFile", api, "build",
            ClassDesc.of("TreeEffectProbeLevel"), build);
        return allocate(MethodHandles.lookup().defineClass(bytes));
    }

    static int stateId(Object state) throws Exception {
        return (int) call(type("world.level.block.Block"), "getId", state);
    }

    static Object nbtJson(Object tag) throws Exception {
        return tag == null ? null : call(field("nbt.NbtOps", "INSTANCE"), "convertTo", jsonOps, tag);
    }

    static class World {
        final Object region, chunk, pos;
        final Pos source;
        final Map<Pos, Object> chunks = new HashMap<>();
        final IdentityHashMap<Object, Integer> identities = new IdentityHashMap<>();
        final List<int[]> requests = new ArrayList<>();

        World(Pos target) throws Exception {
            source = new Pos((target.x() >> 4) + 1, 0, (target.z() >> 4) - 1);
            pos = make("core.BlockPos", target.x(), target.y(), target.z());
            Object carvers = field(STATUS, "CARVERS");
            Object initializer = Proxy.newProxyInstance(TreeEffectReference.class.getClassLoader(),
                new Class<?>[]{type("util.StaticCache2D$Initializer")}, (proxy, method, args) -> {
                    if (!method.getName().equals("get")) throw new UnsupportedOperationException(method.toString());
                    int x = (int) args[0], z = (int) args[1];
                    Object owner = make("world.level.ChunkPos", x, z);
                    Object chunk = make("world.level.chunk.ProtoChunk", owner,
                        field("world.level.chunk.UpgradeData", "EMPTY"), height, factory, null);
                    call(chunk, "setPersistedStatus", carvers);
                    Object holder = make("server.level.ChunkHolder", owner, 0, height, null, null, null);
                    call(holder, "completeFuture", carvers, chunk);
                    chunks.put(new Pos(x, 0, z), chunk);
                    return holder;
                });
            Object cache = call(type("util.StaticCache2D"), "create", source.x(), source.z(), 1, initializer);
            chunk = Objects.requireNonNull(chunks.get(new Pos(target.x() >> 4, 0, target.z() >> 4)));
            region = allocate(type(REGION));
            setField(region, REGION, "cache", cache);
            setField(region, REGION, "center", chunks.get(source));
            setField(region, REGION, "level", level);
            setField(region, REGION, "generatingStep", call(field("world.level.chunk.status.ChunkPyramid",
                "GENERATION_PYRAMID"), "getStepTo", field(STATUS, "FEATURES")));
            setField(region, REGION, "subTickCount", new AtomicLong());
            Object levelData = Proxy.newProxyInstance(TreeEffectReference.class.getClassLoader(),
                new Class<?>[]{type("world.level.storage.LevelData")}, (proxy, method, args) -> {
                    if (method.getName().equals("getGameTime")) return 100L;
                    throw new UnsupportedOperationException(method.toString());
                });
            setField(region, REGION, "levelData", levelData);
            for (String kind : List.of("Block", "Fluid")) {
                Function<Object, Object> getter = p -> {
                    try { return call(call(region, "getChunk", p), "get" + kind + "Ticks"); }
                    catch (Exception e) { throw new RuntimeException(e); }
                };
                setField(region, REGION, kind.toLowerCase(Locale.ROOT) + "Ticks",
                    make("world.ticks.WorldGenTickAccess", getter));
            }
            if (call(region, "getChunk", pos) != chunk) throw new IllegalStateException("wrong native owner");
        }

        void request(Object target, int delay) throws Exception {
            Pos p = Pos.from(pos);
            boolean fluid = type("world.level.material.Fluid").isInstance(target);
            int id = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", target)
                : stateId(call(target, "defaultBlockState"));
            requests.add(new int[]{p.x(), p.y(), p.z(), id, delay, fluid ? 1 : 0});
            call(region, "scheduleTick", pos, target, delay);
        }

        List<Object> ticks() throws Exception {
            List<Object> result = new ArrayList<>();
            for (String kind : List.of("Block", "Fluid")) {
                for (Object tick : (List<?>) call(call(chunk, "get" + kind + "Ticks"), "scheduledTicks")) {
                    Pos p = Pos.from(call(tick, "pos"));
                    Object target = call(tick, "type");
                    boolean fluid = kind.equals("Fluid");
                    int id = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", target)
                        : stateId(call(target, "defaultBlockState"));
                    result.add(List.of(p.x(), p.y(), p.z(), id, call(tick, "delay"), fluid ? 1 : 0,
                        call(call(tick, "priority"), "getValue")));
                }
            }
            return result;
        }

        Optional<?> resolve() throws Exception {
            Optional<?> typed = (Optional<?>) call(region, "getBlockEntity", pos, hiveType);
            Object direct = call(chunk, "getBlockEntity", pos);
            if (typed.orElse(null) != direct) throw new IllegalStateException("typed hive lookup differs");
            return typed;
        }

        Map<String, Object> snapshot() throws Exception {
            Object entity = call(chunk, "getBlockEntity", pos);
            Object pending = call(chunk, "getBlockEntityNbt", pos);
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("state", stateId(call(chunk, "getBlockState", pos)));
            result.put("entity_id", entity == null ? 0 : identities.computeIfAbsent(entity, ignored -> identities.size() + 1));
            result.put("entity_state", entity == null ? null : stateId(call(entity, "getBlockState")));
            result.put("pending_id", pending == null ? null : call(pending, "getStringOr", "id", ""));
            List<Object> ages = null;
            if (entity != null) {
                if (call(entity, "getType") != hiveType) throw new IllegalStateException("unexpected block entity type");
                ages = new ArrayList<>();
                for (Object occupant : (List<?>) call(entity, "getBees")) ages.add(call(occupant, "ticksInHive"));
                if (ages.size() != (int) call(entity, "getOccupantCount")) throw new IllegalStateException("occupant count differs");
            }
            result.put("ticks_in_hive", ages);
            return result;
        }
    }

    static Action write(Object state) { return new Action("write", state); }
    static Action resolve() { return new Action("resolve", null); }
    static Action store(int age) { return new Action("store", age); }

    static List<Scenario> scenarios() throws Exception {
        Object facing = field("world.level.block.BeehiveBlock", "FACING");
        Object honey = field("world.level.block.BeehiveBlock", "HONEY_LEVEL");
        Object nest = call(state("BEE_NEST"), "setValue", facing, field("core.Direction", "SOUTH"));
        Object hive = state("BEEHIVE");
        Object nestProperties = call(call(nest, "setValue", facing, field("core.Direction", "EAST")), "setValue", honey, 5);
        Object hiveProperties = call(call(hive, "setValue", facing, field("core.Direction", "WEST")), "setValue", honey, 3);
        return List.of(
            new Scenario("same_state", nest, true, List.of(write(nest), resolve(), store(-5), resolve())),
            new Scenario("nest_properties", nest, true, List.of(write(nestProperties), resolve(), store(-5), write(nest), resolve())),
            new Scenario("hive_properties", hive, true, List.of(write(hiveProperties), resolve(), store(-5), write(hive), resolve())),
            new Scenario("nest_to_hive", nest, true, List.of(write(hive), resolve(), store(-5))),
            new Scenario("hive_to_nest", hive, true, List.of(write(nest), resolve(), store(-5))),
            new Scenario("stone_recreate", nest, true, List.of(write(state("STONE")), resolve(), write(nest), resolve(), store(-5))),
            new Scenario("air_recreate", hive, true, List.of(write(state("AIR")), resolve(), write(hive), resolve(), store(-5))),
            new Scenario("repeated_stores", nest, true, List.of(store(-5), resolve(), store(Integer.MIN_VALUE), store(Integer.MAX_VALUE))),
            new Scenario("pending_rewrites", nest, false, List.of(write(nestProperties), write(hive), resolve(), store(-5), write(nest), resolve(), store(599)))
        );
    }

    static Map<String, Object> sample(Scenario scenario, Pos target, int flags) throws Exception {
        World world = new World(target);
        poiCallbacks = 0;
        Object block = call(scenario.initial(), "getBlock");
        Object water = field("world.level.material.Fluids", "WATER");
        world.request(block, 1);
        world.request(water, 5);
        world.request(block, -5);
        world.request(water, Integer.MIN_VALUE);
        world.request(field("world.level.material.Fluids", "FLOWING_LAVA"), Integer.MAX_VALUE);
        List<Object> initialTicks = world.ticks();
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("name", scenario.name());
        result.put("pos", List.of(target.x(), target.y(), target.z()));
        result.put("source", List.of(world.source.x(), world.source.z()));
        result.put("flags", flags);
        result.put("requests", world.requests);
        result.put("native_ticks_before", initialTicks);
        List<Action> actions = new ArrayList<>();
        actions.add(write(scenario.initial()));
        if (scenario.resolveInitial()) actions.addAll(List.of(resolve(), store(9)));
        actions.addAll(scenario.actions());
        List<Object> steps = new ArrayList<>();
        for (Action action : actions) {
            Map<String, Object> step = new LinkedHashMap<>();
            step.put("op", action.op());
            switch (action.op()) {
                case "write" -> {
                    step.put("state", stateId(action.value()));
                    step.put("accepted", call(world.region, "setBlock", world.pos, action.value(), flags, 512));
                }
                case "resolve" -> step.put("present", world.resolve().isPresent());
                case "store" -> {
                    step.put("age", action.value());
                    Object hive = world.resolve().orElseThrow();
                    call(hive, "storeBee", call(type(HIVE + "$Occupant"), "create", action.value()));
                }
                default -> throw new IllegalArgumentException(action.op());
            }
            step.put("after", world.snapshot());
            step.put("native_ticks_unchanged", initialTicks.equals(world.ticks()));
            steps.add(step);
        }
        result.put("steps", steps);
        result.put("native_ticks_after", world.ticks());
        result.put("poi_callbacks", poiCallbacks);
        result.put("nbt", nbtJson(call(world.chunk, "getBlockEntityNbtForSaving", world.pos, registries)));
        return result;
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        registries = NativeWorldgenRegistries.load();
        factory = call(type("world.level.chunk.PalettedContainerFactory"), "create", registries);
        height = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        hiveType = field("world.level.block.entity.BlockEntityType", "BEEHIVE");
        level = probeLevel();
        jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        List<Object> samples = new ArrayList<>();
        List<Pos> positions = List.of(new Pos(-1, 65, 16), new Pos(16, 319, -1), new Pos(-17, -64, -1));
        int index = 0;
        for (Scenario scenario : scenarios()) for (int flags : new int[]{19, 3})
            samples.add(sample(scenario, positions.get(index++ % positions.size()), flags));
        Map<String, Object> root = new LinkedHashMap<>();
        root.put("version", 1);
        root.put("api", "WorldGenRegion.setBlock/getBlockEntity(BlockPos, BlockEntityType) -> ProtoChunk");
        root.put("setup", "Constructed ProtoChunks/ChunkHolders with completed CARVERS futures; native FEATURES step; WorldGenRegion constructor bypassed; fixed game time 100; server POI callback record-only");
        root.put("scope", "BeehiveBlockEntity-compatible states, non-block-entity removal/recreation, typed lookup/storeBee and native proto tick retention");
        root.put("excluded", List.of("transitions to other block-entity types", "LevelChunk writes", "POI execution", "global source scheduling", "tick execution"));
        root.put("request_encoding", "input scheduleTick calls: x,y,z,default block-state/fluid ID,delay,fluid flag; native ticks append priority and have proto delay zero");
        root.put("source_dependencies", List.of("TreeReference.java", "NativeEntityLevel.java", "NativeWorldgenRegistries.java", "TreeEffectReference.java"));
        root.put("samples", samples);
        Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        Object gson = call(call(builder, "serializeNulls"), "create");
        System.out.println("TREE_EFFECT_REFERENCE=" + call(gson, "toJson", root));
    }
}
