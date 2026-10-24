import java.io.*;
import java.lang.reflect.Proxy;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import java.util.function.BooleanSupplier;

/** Original FULL task on an isolated, explicitly constructed native SPAWN holder.
 * This is a transition witness, not a replacement for the native ticket history.
 */
public final class LifecycleProbe {
    private static Path output;
    private static Object level, registries;

    static void initialize(String path) { output = Path.of(path); }
    public static void fatal(Throwable error) { error.printStackTrace(); Runtime.getRuntime().halt(3); }
    public static void unexpectedTick() { fatal(new IllegalStateException("unexpected gameplay tick")); }
    private static Object call(Object target, String name, Object... args) throws Exception {
        return NativeAccess.call(target, name, args);
    }
    private static Object make(String name, Object... args) throws Exception { return NativeAccess.make(name, args); }
    private static Object constant(String owner, String name) throws Exception { return NativeAccess.constant(owner, name); }
    private static Object status(String name) throws Exception { return constant("world.level.chunk.status.ChunkStatus", name); }
    private static Object pos(int x, int y, int z) throws Exception { return make("core.BlockPos", x, y, z); }
    private static Object state(String block) throws Exception { return call(constant("world.level.block.Blocks", block), "defaultBlockState"); }
    private static Map<String, Object> map(Object... pairs) { return NativeAccess.map(pairs); }

    private static Object encoded(Object tag) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        call(NativeAccess.type("nbt.NbtIo"), "write", tag, new DataOutputStream(bytes));
        return map("nbt", Base64.getEncoder().encodeToString(bytes.toByteArray()));
    }

    private static List<Object> blockEntities(Object chunk, String field) throws Exception {
        List<Object> result = new ArrayList<>();
        for (Map.Entry<?, ?> entry : ((Map<?, ?>)NativeAccess.field(chunk, field)).entrySet()) {
            Object p = entry.getKey();
            Object nbt = field.equals("pendingBlockEntities") ? entry.getValue()
                : call(entry.getValue(), "saveWithFullMetadata", registries);
            result.add(map("pos", List.of(call(p, "getX"), call(p, "getY"), call(p, "getZ")), "data", encoded(nbt)));
        }
        result.sort(Comparator.comparing(row -> ((Map<?, ?>)row).get("pos").toString()));
        return result;
    }

    /** Reads maps and saved tick data, never getBlockEntity/SerializableChunkData. */
    private static Object observe(Object chunk, String boundary) throws Exception {
        List<String> heightmaps = new ArrayList<>();
        for (Object value : (Collection<?>)call(chunk, "getHeightmaps")) {
            heightmaps.add(((Enum<?>)((Map.Entry<?, ?>)value).getKey()).name());
        }
        Collections.sort(heightmaps);
        List<Object> marks = new ArrayList<>();
        for (Object section : (Object[])call(chunk, "getPostProcessing"))
            marks.add(section == null ? List.of() : new ArrayList<>((Collection<?>)section));
        Object blockTicks = call(chunk, "getBlockTicks"), fluidTicks = call(chunk, "getFluidTicks");
        return map("boundary", boundary, "class", chunk.getClass().getSimpleName(),
            "persisted", call(call(chunk, "getPersistedStatus"), "getName"), "heightmaps", heightmaps,
            "pending", blockEntities(chunk, "pendingBlockEntities"), "loaded", blockEntities(chunk, "blockEntities"),
            "marks", marks, "block_ticks_class", blockTicks.getClass().getSimpleName(),
            "fluid_ticks_class", fluidTicks.getClass().getSimpleName(),
            "block_ticks_count", call(blockTicks, "count"), "fluid_ticks_count", call(fluidTicks, "count"),
            "block_tick_head", blockTicks.getClass().getSimpleName().equals("LevelChunkTicks") ? tick(call(blockTicks, "peek")) : null,
            "block_ticks_registered", registered(chunk, "getBlockTicks", blockTicks),
            "fluid_ticks_registered", registered(chunk, "getFluidTicks", fluidTicks));
    }

    private static boolean registered(Object chunk, String accessor, Object ticks) throws Exception {
        Map<?, ?> containers = (Map<?, ?>)NativeAccess.field(call(level, accessor), "allContainers");
        Object p = call(chunk, "getPos");
        long key = ((int)call(p, "x") & 0xffffffffL) | ((long)(int)call(p, "z") << 32);
        return containers.get(key) == ticks;
    }

    private static Object tick(Object tick) throws Exception {
        return tick == null ? null : map("type", call(tick, "type").toString(),
            "trigger_tick", call(tick, "triggerTick"), "priority", call(tick, "priority").toString(),
            "sub_tick_order", call(tick, "subTickOrder"));
    }

    private static Object serialize(Object chunk) throws Exception {
        Object data = call(NativeAccess.type("world.level.chunk.storage.SerializableChunkData"), "copyOf", level, chunk);
        return encoded(call(data, "write"));
    }

    private static Object applyFull(Object context, Object cache, Object proto) throws Exception {
        Object step = call(constant("world.level.chunk.status.ChunkPyramid", "GENERATION_PYRAMID"), "getStepTo", status("FULL"));
        CompletableFuture<?> future = (CompletableFuture<?>)call(step, "apply", context, cache, proto);
        // Pump the original main-thread executor; no executor or task body is substituted.
        call(call(level, "getServer"), "managedBlock", (BooleanSupplier)future::isDone);
        return future.join();
    }

    public static void run(Object serverLevel) {
        try {
            level = serverLevel;
            registries = call(level, "registryAccess");
            long gameTime = (long)call(level, "getGameTime");
            Object factory = call(NativeAccess.type("world.level.chunk.PalettedContainerFactory"), "create", registries);
            Object proto = make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", 0, 0),
                constant("world.level.chunk.UpgradeData", "EMPTY"), level, factory, null);
            call(proto, "setPersistedStatus", status("CARVERS"));
            for (int x = 1; x <= 3; x++) {
                Object p = pos(x, 80, 1), s = state(x == 2 ? "SCULK_SHRIEKER" : "SCULK_SENSOR");
                call(proto, "setBlockState", p, s, 18);
                if (x == 3) {
                    Object entity = call(call(s, "getBlock"), "newBlockEntity", p, s);
                    call(entity, "setLastVibrationFrequency", 9);
                    call(proto, "setBlockEntity", entity);
                } else {
                    Object tag = make("nbt.CompoundTag");
                    call(tag, "putString", "id", "DUMMY");
                    call(tag, "putInt", "x", x); call(tag, "putInt", "y", 80); call(tag, "putInt", "z", 1);
                    call(proto, "setBlockEntityNbt", tag);
                }
            }
            Set<Object> maps = new HashSet<>();
            for (String name : List.of("WORLD_SURFACE_WG", "OCEAN_FLOOR_WG", "WORLD_SURFACE", "OCEAN_FLOOR", "MOTION_BLOCKING", "MOTION_BLOCKING_NO_LEAVES"))
                maps.add(constant("world.level.levelgen.Heightmap$Types", name));
            call(NativeAccess.type("world.level.levelgen.Heightmap"), "primeHeightmaps", proto, maps);
            call(proto, "markPosForPostprocessing", pos(8, 80, 8));
            call(proto, "markPosForPostprocessing", pos(8, 80, 8));
            Object blockTicks = call(proto, "getBlockTicks");
            Object normal = constant("world.ticks.TickPriority", "NORMAL");
            Object high = constant("world.ticks.TickPriority", "HIGH");
            Object stone = constant("world.level.block.Blocks", "STONE");
            call(blockTicks, "schedule", make("world.ticks.ScheduledTick", stone, pos(4, 80, 4), -5L, normal, 17L));
            call(blockTicks, "schedule", make("world.ticks.ScheduledTick", stone, pos(4, 80, 4), -9L, high, 18L));
            call(blockTicks, "schedule", make("world.ticks.ScheduledTick", stone, pos(5, 80, 4), 7L, high, 19L));
            call(call(proto, "getFluidTicks"), "schedule", make("world.ticks.ScheduledTick",
                constant("world.level.material.Fluids", "WATER"), pos(6, 80, 4), 11L, normal, 20L));
            call(proto, "initializeLightSources");
            call(proto, "setLightEngine", call(level, "getLightEngine"));
            call(proto, "setPersistedStatus", status("SPAWN"));
            call(proto, "setLightCorrect", true);
            Object holder = make("server.level.ChunkHolder", call(proto, "getPos"), 33, level, call(level, "getLightEngine"), null, null);
            for (String name : List.of("EMPTY", "STRUCTURE_STARTS", "STRUCTURE_REFERENCES", "BIOMES", "NOISE", "SURFACE", "CARVERS", "FEATURES", "INITIALIZE_LIGHT", "LIGHT", "SPAWN"))
                call(holder, "completeFuture", status(name), proto);
            Class<?> initializer = NativeAccess.type("util.StaticCache2D$Initializer");
            Object init = Proxy.newProxyInstance(initializer.getClassLoader(), new Class<?>[]{initializer}, (p, m, a) -> {
                if (!m.getName().equals("get")) throw new UnsupportedOperationException(m.toString());
                return holder;
            });
            Object cache = call(NativeAccess.type("util.StaticCache2D"), "create", 0, 0, 0, init);
            Object context = NativeAccess.field(NativeAccess.field(call(level, "getChunkSource"), "chunkMap"), "worldGenContext");
            List<Object> observations = new ArrayList<>();
            observations.add(observe(proto, "before FULL"));
            observations.add(map("boundary", "proto serialization", "saved", serialize(proto)));
            Object full = applyFull(context, cache, proto);
            observations.add(observe(full, "after FULL before serialization"));
            Object retained = call(holder, "getChunkIfPresentUnchecked", status("LIGHT"));
            observations.add(map("boundary", "holder after FULL", "lower_status_class", retained.getClass().getSimpleName(),
                "wrapped_is_full", call(retained, "getWrapped") == full,
                "sections_shared", ((Object[])call(proto, "getSections"))[9] == ((Object[])call(full, "getSections"))[9],
                "loaded_entity_shared", ((Map<?, ?>)NativeAccess.field(proto, "blockEntities")).get(pos(3, 80, 1))
                    == ((Map<?, ?>)NativeAccess.field(full, "blockEntities")).get(pos(3, 80, 1))));
            observations.add(map("boundary", "lower-status access after FULL",
                "write_returned_null", call(retained, "setBlockState", pos(8, 80, 8), state("STONE"), 18) == null,
                "write_left_air", call(call(full, "getBlockState", pos(8, 80, 8)), "isAir"),
                "wg_height", call(retained, "getHeight", constant("world.level.levelgen.Heightmap$Types", "WORLD_SURFACE_WG"), 1, 1),
                "runtime_height", call(full, "getHeight", constant("world.level.levelgen.Heightmap$Types", "WORLD_SURFACE"), 1, 1),
                "tick_access_class", call(retained, "getBlockTicks").getClass().getSimpleName()));
            observations.add(observe(proto, "old proto retained after FULL"));
            observations.add(map("boundary", "level serialization", "saved", serialize(full)));
            observations.add(observe(full, "after level serialization"));
            Object repeated = applyFull(context, cache, retained);
            observations.add(map("boundary", "repeat FULL", "same_chunk", repeated == full));
            observations.add(observe(full, "after repeat FULL"));
            call(full, "unpackTicks", gameTime);
            observations.add(observe(full, "after unpackTicks"));
            observations.add(map("boundary", "unpacked serialization", "saved", serialize(full)));
            if ((long)call(level, "getGameTime") != gameTime) throw new IllegalStateException("game time advanced");
            Object gson = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
            gson = call(call(gson, "serializeNulls"), "create");
            Files.writeString(output.resolve("observations.raw.json"), (String)call(gson, "toJson",
                map("schema", 1, "minecraft", "26.1", "protocol", 775, "game_time", gameTime,
                    "gameplay_ticks", 0, "observations", observations)) + "\n", StandardOpenOption.CREATE_NEW);
            System.out.println("FULL_LIFECYCLE_COMPLETE observations=" + observations.size());
            Runtime.getRuntime().halt(0);
        } catch (Throwable error) { fatal(error); }
    }
}
