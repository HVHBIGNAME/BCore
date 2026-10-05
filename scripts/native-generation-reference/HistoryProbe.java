import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import java.util.function.*;
import java.util.zip.GZIPOutputStream;

/** Runs real ServerChunkCache requests and records native stages before any gameplay tick. */
public final class HistoryProbe {
    private static final AtomicLong SEQUENCE = new AtomicLong();
    // Recording must not call identityHashCode: materializing an extra JVM hash can
    // perturb later identity-keyed native collection iteration. Equality here is ==.
    private record Identity(Object value, long id) { }
    private static final Map<String, List<Identity>> IDENTITIES = new HashMap<>();
    private static long identityCounter;
    private static final Set<String> BLOBS = ConcurrentHashMap.newKeySet();
    private static final Map<Long, Stage> STAGES = new ConcurrentHashMap<>();
    // Protected by emit's monitor: describe the observed prefix at its atomic
    // end boundary, including native ticket work still in flight after a request.
    private static final Map<Long, Map<String,Object>> OPEN_STAGES = new LinkedHashMap<>();
    private static volatile boolean stopped;
    private static final ThreadLocal<Stage> ACTIVE = new ThreadLocal<>();
    private static final ThreadLocal<Boolean> OBSERVING = ThreadLocal.withInitial(() -> false);
    private static final List<String> INSTRUMENTED = new CopyOnWriteArrayList<>();
    private static final AtomicInteger TICKS = new AtomicInteger();
    private static Map<?, ?> config;
    private static Object gson;
    private static Path output;
    private static BufferedWriter trace;
    private static volatile String phase = "bootstrap";
    private static volatile int request = -1;
    private static volatile boolean running;
    private static long sources;

    private record Stage(long id, Object level, Object step, Object cache, Object chunk,
                         String status, List<Integer> pos, long source) { }

    static void initialize(String configPath) throws Exception {
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        config = (Map<?, ?>) NativeAccess.call(gson, "fromJson", Files.readString(Path.of(configPath)), Map.class);
        output = Path.of((String) config.get("output"));
        Files.createDirectories(output.resolve("blobs"));
        trace = Files.newBufferedWriter(output.resolve("events.jsonl"), StandardCharsets.UTF_8,
            StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE);
        emit("oracle_start", NativeAccess.map("config", config, "java", System.getProperty("java.runtime.version"),
            "processors", Runtime.getRuntime().availableProcessors(), "gameplay_ticks", 0));
    }

    static void instrumented(String method) { INSTRUMENTED.add(method); }

    private static long identity(Object value) {
        if (value == null) return 0;
        synchronized (IDENTITIES) {
            List<Identity> instances = IDENTITIES.computeIfAbsent(value.getClass().getName(), key -> new ArrayList<>());
            for (Identity entry : instances) if (entry.value == value) return entry.id;
            long id = ++identityCounter;
            instances.add(new Identity(value, id));
            return id;
        }
    }

    private static synchronized long emit(String event, Map<String, Object> fields) throws Exception {
        if (stopped) return -1;
        long seq = SEQUENCE.incrementAndGet();
        Map<String, Object> row = NativeAccess.map("seq", seq, "event", event, "phase", phase,
            "request", request, "thread", Thread.currentThread().getName());
        row.putAll(fields);
        if (event.equals("stage_enter")) OPEN_STAGES.put(seq, NativeAccess.map("stage_id",seq,
            "pos",row.get("pos"),"status",row.get("status"),"request",row.get("request"),"thread",row.get("thread")));
        if (event.equals("stage_exit")) {
            if (OPEN_STAGES.remove(((Number) fields.get("stage_id")).longValue()) == null)
                throw new IllegalStateException("unpaired observed stage exit");
        }
        if (event.equals("oracle_complete")) {
            row.put("inflight_stages", new ArrayList<>(OPEN_STAGES.values()));
            stopped = true;
        }
        trace.write((String) NativeAccess.call(gson, "toJson", row));
        trace.newLine();
        if (!event.equals("world_set_block") && !event.equals("world_write_guard")) trace.flush();
        return seq;
    }

    public static void fatal(Throwable failure) {
        failure.printStackTrace();
        try {
            emit("oracle_failure", NativeAccess.map("class", failure.getClass().getName(), "message", failure.toString()));
            trace.close();
        } catch (Throwable ignored) { }
        Runtime.getRuntime().halt(3);
    }

    private static String status(Object value) throws Exception {
        return value == null ? null : (String) NativeAccess.call(value, "getName");
    }

    private static List<Integer> chunkPos(Object value) throws Exception {
        return List.of((int) NativeAccess.call(value, "x"), (int) NativeAccess.call(value, "z"));
    }

    private static List<Integer> blockPos(Object value) throws Exception {
        return List.of((int) NativeAccess.call(value, "getX"), (int) NativeAccess.call(value, "getY"),
            (int) NativeAccess.call(value, "getZ"));
    }

    private static Map<String, Object> holder(Object value) throws Exception {
        return NativeAccess.map("holder", identity(value), "pos", chunkPos(NativeAccess.call(value, "getPos")),
            "ticket_level", NativeAccess.call(value, "getTicketLevel"), "queue_level", NativeAccess.call(value, "getQueueLevel"),
            "persisted", status(NativeAccess.call(value, "getPersistedStatus")),
            "latest", status(NativeAccess.call(value, "getLatestStatus")));
    }

    private static Map<String, Object> task(Object value) throws Exception {
        return NativeAccess.map("task", identity(value), "pos", chunkPos(NativeAccess.field(value, "pos")),
            "target", status(NativeAccess.field(value, "targetStatus")),
            "scheduled", status(NativeAccess.field(value, "scheduledStatus")),
            "needs_generation", NativeAccess.field(value, "needsGeneration"));
    }

    private static Map<String, Object> sourceFields() {
        Stage stage = ACTIVE.get();
        return stage == null ? NativeAccess.map() : NativeAccess.map("stage_id", stage.id,
            "source", stage.pos, "source_sequence", stage.source, "status", stage.status);
    }

    public static void enter(String method, Object self, Object[] args) {
        if (OBSERVING.get() || stopped) return;
        try {
            if (method.equals("server/MinecraftServer.setInitialSpawn")) {
                emit("bootstrap_spawn_boundary", NativeAccess.map("mode", config.get("bootstrap"),
                    "level_seed", NativeAccess.call(args[0], "getSeed")));
                if (config.get("bootstrap").equals("before_spawn")) run(args[0]);
            } else if (method.equals("server/MinecraftServer.tickServer") || method.equals("server/level/ServerLevel.tick")) {
                TICKS.incrementAndGet();
                throw new IllegalStateException("gameplay tick entered before oracle completed: " + method);
            } else if (method.equals("world/level/chunk/status/ChunkStep.apply")) {
                Object level = NativeAccess.call(args[0], "level");
                String target = status(NativeAccess.call(self, "targetStatus"));
                long source = target.equals("minecraft:features") ? ++sources : 0;
                Map<String, Object> row = NativeAccess.map("pos", chunkPos(NativeAccess.call(args[2], "getPos")),
                    "status", target, "chunk", identity(args[2]), "step", identity(self),
                    "task_impl", NativeAccess.call(self, "task").getClass().getName(),
                    "source_sequence", source, "game_time", NativeAccess.call(level, "getGameTime"));
                long id = emit("stage_enter", row);
                Stage stage = new Stage(id, level, self, args[1], args[2], target,
                    chunkPos(NativeAccess.call(args[2], "getPos")), source);
                STAGES.put(identity(args[2]), stage);
                ACTIVE.set(stage);
                snapshots(stage, "before", args[2]);
            } else if (method.equals("server/level/ChunkMap.applyStep")) {
                Map<String, Object> row = holder(args[0]);
                row.put("status", status(NativeAccess.call(args[1], "targetStatus")));
                row.put("map", identity(self));
                emit("map_apply_step", row);
            } else if (method.equals("server/level/ChunkGenerationTask.scheduleLayer")
                       || method.equals("server/level/ChunkGenerationTask.runUntilWait")
                       || method.equals("server/level/ChunkGenerationTask.markForCancellation")) {
                Map<String, Object> row = task(self);
                if (args.length > 0) {
                    row.put("layer", status(args[0]));
                    row.put("generation", args[1]);
                }
                emit(method.substring(method.lastIndexOf('.') + 1), row);
            } else if (method.equals("server/level/ChunkGenerationTask.scheduleChunkInLayer")) {
                Map<String, Object> row = task(self);
                row.put("layer", status(args[0]));
                row.put("generation", args[1]);
                row.put("destination", holder(args[2]));
                emit("schedule_chunk", row);
            } else if (method.equals("server/level/GenerationChunkHolder.scheduleChunkGenerationTask")
                       || method.equals("server/level/GenerationChunkHolder.completeFuture")) {
                Map<String, Object> row = holder(self);
                row.put("status", status(args[0]));
                emit(method.endsWith("completeFuture") ? "holder_complete" : "holder_request", row);
            } else if (method.equals("server/level/ServerChunkCache.getChunk")
                       || method.equals("server/level/ServerChunkCache.getChunkFutureMainThread")) {
                emit("native_request", NativeAccess.map("api", method, "cache", identity(self),
                    "pos", List.of(args[0], args[1]), "status", status(args[2]), "create", args[3]));
            } else if (method.equals("server/level/ChunkTaskDispatcher.submit")) {
                long packed = (long) args[1];
                emit("dispatcher_submit", NativeAccess.map("dispatcher", identity(self), "runnable", identity(args[0]),
                    "pos", List.of((int) packed, (int) (packed >> 32)), "priority", ((IntSupplier) args[2]).getAsInt()));
            } else if (method.equals("server/level/ChunkTaskDispatcher.scheduleForExecution")) {
                long packed = (long) NativeAccess.call(args[0], "chunkPos");
                List<Long> tasks = new ArrayList<>();
                for (Object task : (List<?>) NativeAccess.call(args[0], "tasks")) tasks.add(identity(task));
                emit("dispatcher_execute", NativeAccess.map("dispatcher", identity(self),
                    "pos", List.of((int) packed, (int) (packed >> 32)), "tasks", tasks));
            } else if (method.equals("world/level/levelgen/placement/PlacedFeature.placeWithBiomeCheck")) {
                feature("feature_enter", self, args, null);
            } else if (method.equals("server/level/WorldGenRegion.setCurrentlyGenerating")) {
                Map<String, Object> row = sourceFields();
                row.put("label", args[0] == null ? null : ((Supplier<?>) args[0]).get());
                emit("decoration_label", row);
            } else if (method.equals("world/ticks/WorldGenTickAccess.schedule") && ACTIVE.get() != null) {
                Map<String, Object> row = sourceFields();
                row.put("pos", blockPos(NativeAccess.call(args[0], "pos")));
                row.put("type", NativeAccess.call(args[0], "type").toString());
                row.put("trigger_tick", NativeAccess.call(args[0], "triggerTick"));
                row.put("priority", NativeAccess.call(args[0], "priority").toString());
                row.put("sub_tick_order", NativeAccess.call(args[0], "subTickOrder"));
                emit("world_tick_request", row);
            } else if (method.equals("server/level/WorldGenRegion.markPosForPostprocessing")) {
                Map<String, Object> row = sourceFields();
                row.put("pos", blockPos(args[0]));
                emit("world_postprocess", row);
            } else if (method.equals("world/level/chunk/ProtoChunk.markPosForPostprocessing")) {
                Stage stage = STAGES.get(identity(self));
                emit("chunk_postprocess", NativeAccess.map("pos", blockPos(args[0]),
                    "chunk", chunkPos(NativeAccess.call(self, "getPos")),
                    "stage_id", stage == null ? null : stage.id,
                    "source", ACTIVE.get() == null ? null : ACTIVE.get().pos));
            } else if (method.equals("server/level/ChunkMap.prepareTickingChunk")
                       || method.equals("world/level/chunk/LevelChunk.postProcessGeneration")) {
                emit("ticking_preparation", NativeAccess.map("api", method,
                    "pos", chunkPos(NativeAccess.call(args.length == 0 ? self : args[0], "getPos"))));
            }
        } catch (Throwable failure) { fatal(failure); }
    }

    public static void exit(Object result, String method, Object self, Object[] args) {
        if (OBSERVING.get() || stopped) return;
        try {
            if (method.equals("server/MinecraftServer.loadLevel") && config.get("bootstrap").equals("native")) {
                run(NativeAccess.call(self, "overworld"));
            } else if (method.equals("server/level/ChunkGenerationTask.create")) {
                emit("task_create", task(result));
            } else if (method.equals("server/level/GenerationChunkHolder.acquireStatusBump")) {
                Map<String, Object> row = holder(self);
                row.put("status", status(args[0]));
                row.put("acquired", result);
                emit("holder_claim", row);
            } else if (method.equals("server/level/ChunkGenerationTask.runUntilWait")) {
                Map<String, Object> row = task(self);
                row.put("wait_future", identity(result));
                row.put("done", result == null || ((CompletableFuture<?>) result).isDone());
                emit("task_wait", row);
            } else if (method.equals("world/level/levelgen/placement/PlacedFeature.placeWithBiomeCheck")) {
                feature("feature_exit", self, args, result);
            } else if (method.equals("world/level/levelgen/WorldgenRandom.setDecorationSeed")
                       || method.equals("world/level/levelgen/WorldgenRandom.setFeatureSeed")) {
                if (ACTIVE.get() == null) return;
                Map<String, Object> row = sourceFields();
                row.put("args", args);
                row.put("result", result);
                row.put("rng", rng(self));
                emit(method.endsWith("setFeatureSeed") ? "feature_seed" : "decoration_seed", row);
            } else if (method.equals("server/level/WorldGenRegion.setBlock")
                       || method.equals("server/level/WorldGenRegion.ensureCanWrite")) {
                Map<String, Object> row = sourceFields();
                row.put("pos", blockPos(args[0]));
                row.put("accepted", result);
                if (args.length > 1) {
                    row.put("state", NativeAccess.call(NativeAccess.type("world.level.block.Block"), "getId", args[1]));
                    row.put("flags", args[2]);
                    row.put("recursion", args[3]);
                }
                if (ACTIVE.get() != null) emit(args.length > 1 ? "world_set_block" : "world_write_guard", row);
            } else if (method.equals("server/level/WorldGenRegion.addFreshEntity")) {
                Map<String, Object> row = sourceFields();
                row.put("entity", entityNbt(NativeAccess.call(self, "getLevel"), args[0]));
                row.put("accepted", result);
                emit("world_entity", row);
            } else if (method.equals("server/level/WorldGenRegion.getHeight") && args.length == 3 && ACTIVE.get() != null) {
                Map<String, Object> row = sourceFields();
                String kind = ((Enum<?>) args[0]).name();
                row.put("heightmap", kind);
                row.put("xz", List.of(args[1], args[2]));
                row.put("returned", result);
                Object current = NativeAccess.field(self, "currentlyGenerating");
                String label = current == null ? "" : ((Supplier<?>) current).get().toString();
                row.put("decoration_label", label);
                if (kind.endsWith("_WG") && (label.contains("patch_grass_forest") || label.contains("seagrass"))) {
                    // Observe native states with the native heightmap predicate; do NOT
                    // call getHeight on a different type (that could prime a live map).
                    @SuppressWarnings("unchecked")
                    Predicate<Object> opaque = (Predicate<Object>) NativeAccess.call(args[0], "isOpaque");
                    int min = (int) NativeAccess.call(self, "getMinY");
                    int top = min;
                    for (int y = min + (int) NativeAccess.call(self, "getHeight") - 1; y >= min; y--) {
                        Object state = NativeAccess.call(self, "getBlockState", NativeAccess.make("core.BlockPos", args[1], y, args[2]));
                        if (opaque.test(state)) { top = y + 1; break; }
                    }
                    row.put("live_predicate_first_free", top);
                }
                emit("world_height_query", row);
            }
        } catch (Throwable failure) { fatal(failure); }
    }

    public static CompletableFuture<?> future(CompletableFuture<?> future, String method, Object self, Object[] args) {
        if (OBSERVING.get() || stopped) return future;
        try {
            if (method.equals("server/level/ChunkMap.applyStep")) {
                if (!status(NativeAccess.call(args[1], "targetStatus")).equals("minecraft:empty")) return future;
                Object level = NativeAccess.field(self, "level");
                return future.whenComplete((chunk, error) -> {
                    try {
                        if (error != null) throw new IllegalStateException("native EMPTY failed", error);
                        emit("empty_complete", NativeAccess.map("holder", holder(args[0]), "snapshot", snapshot(level, chunk)));
                    } catch (Throwable failure) { fatal(failure); }
                });
            }
            Stage stage = STAGES.get(identity(args[2]));
            if (stage == null) throw new IllegalStateException("stage exit without entry");
            ACTIVE.remove();
            return future.whenComplete((chunk, error) -> {
                try {
                    if (error != null) throw new IllegalStateException("native stage failed " + stage.status + stage.pos, error);
                    snapshots(stage, "after", chunk);
                    emit("stage_exit", NativeAccess.map("stage_id", stage.id, "pos", stage.pos, "status", stage.status,
                        "source_sequence", stage.source, "persisted", status(NativeAccess.call(chunk, "getPersistedStatus"))));
                    STAGES.remove(identity(args[2]));
                } catch (Throwable failure) { fatal(failure); }
            });
        } catch (Throwable failure) { fatal(failure); return future; }
    }

    private static void feature(String event, Object feature, Object[] args, Object result) throws Exception {
        if (ACTIVE.get() == null) return;
        Object registry = NativeAccess.call(NativeAccess.call(args[0], "registryAccess"), "lookupOrThrow",
            NativeAccess.constant("core.registries.Registries", "PLACED_FEATURE"));
        Object key = NativeAccess.call(registry, "getKey", feature);
        Map<String, Object> row = sourceFields();
        row.put("feature", key == null ? null : key.toString());
        row.put("pos", blockPos(args[3]));
        row.put("rng", rng(args[2]));
        row.put("result", result);
        long eventId = emit(event, row);
        String name = key == null ? "" : key.toString().replace("minecraft:", "");
        if (((List<?>) config.get("feature_snapshots")).contains(name)) {
            Stage stage = ACTIVE.get();
            List<Object> chunks = new ArrayList<>();
            for (int x = stage.pos.get(0) - 1; x <= stage.pos.get(0) + 1; x++) {
                for (int z = stage.pos.get(1) - 1; z <= stage.pos.get(1) + 1; z++) {
                    Object holder = NativeAccess.call(stage.cache, "get", x, z);
                    chunks.add(snapshot(stage.level, NativeAccess.call(holder, "getLatestChunk")));
                }
            }
            emit("feature_snapshot", NativeAccess.map("stage_id", stage.id, "feature_event", eventId,
                "feature", key.toString(), "source", stage.pos, "source_sequence", stage.source,
                "boundary", event.equals("feature_enter") ? "before" : "after", "chunks", chunks));
        }
    }

    private static Map<String, Object> gaussian(Object source) throws Exception {
        Object gaussian = NativeAccess.field(source, "gaussianSource");
        return NativeAccess.map("has_next", NativeAccess.field(gaussian, "haveNextNextGaussian"),
            "next_bits", Long.toUnsignedString(Double.doubleToRawLongBits((double) NativeAccess.field(gaussian, "nextNextGaussian"))));
    }

    private static Map<String, Object> rng(Object random) throws Exception {
        Map<String, Object> result = NativeAccess.map("class", random.getClass().getName(), "identity", identity(random),
            "count", NativeAccess.call(random, "getCount"), "gaussian", gaussian(random));
        Object source = NativeAccess.field(random, "randomSource");
        result.put("source_class", source.getClass().getName());
        result.put("source_gaussian", gaussian(source));
        if (source.getClass().getSimpleName().equals("XoroshiroRandomSource")) {
            Object bits = NativeAccess.field(source, "randomNumberGenerator");
            result.put("seed_lo", NativeAccess.field(bits, "seedLo"));
            result.put("seed_hi", NativeAccess.field(bits, "seedHi"));
            Object cloneSource = NativeAccess.make("world.level.levelgen.XoroshiroRandomSource",
                NativeAccess.field(bits, "seedLo"), NativeAccess.field(bits, "seedHi"));
            Object clone = NativeAccess.make("world.level.levelgen.WorldgenRandom", cloneSource);
            result.put("next_i64_from_copy", NativeAccess.call(clone, "nextLong"));
        } else {
            result.put("seed", ((AtomicLong) NativeAccess.field(source, "seed")).get());
        }
        return result;
    }

    private static void snapshots(Stage stage, String boundary, Object center) throws Exception {
        OBSERVING.set(true);
        try {
            List<Object> chunks = new ArrayList<>();
            // FEATURES may mutate structure pieces throughout the direct cache, not just its block-write square.
            int radius = stage.source == 0 ? 0 : (int) NativeAccess.call(NativeAccess.call(stage.step, "directDependencies"), "getRadius");
            for (int x = stage.pos.get(0) - radius; x <= stage.pos.get(0) + radius; x++) {
                for (int z = stage.pos.get(1) - radius; z <= stage.pos.get(1) + radius; z++) {
                    Object chunk = center;
                    Object holder = NativeAccess.call(stage.cache, "get", x, z);
                    if (x != stage.pos.get(0) || z != stage.pos.get(1)) chunk = NativeAccess.call(holder, "getLatestChunk");
                    if (chunk == null) throw new IllegalStateException("missing stage dependency " + x + "," + z);
                    chunks.add(NativeAccess.map("holder", identity(holder), "snapshot", snapshot(stage.level, chunk)));
                }
            }
            emit("stage_snapshot", NativeAccess.map("stage_id", stage.id, "pos", stage.pos, "status", stage.status,
                "source_sequence", stage.source, "boundary", boundary, "radius", radius, "chunks", chunks));
        } finally { OBSERVING.set(false); }
    }

    private static String blob(byte[] raw, String kind) throws Exception {
        String hash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(raw));
        String name = hash + "." + kind + ".gz";
        if (BLOBS.add(name)) {
            try (OutputStream out = new GZIPOutputStream(Files.newOutputStream(output.resolve("blobs").resolve(name),
                    StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE))) { out.write(raw); }
        }
        return name;
    }

    private static String nbtBlob(Object nbt) throws Exception {
        ByteArrayOutputStream raw = new ByteArrayOutputStream();
        NativeAccess.call(NativeAccess.type("nbt.NbtIo"), "write", nbt, new DataOutputStream(raw));
        return blob(raw.toByteArray(), "nbt");
    }

    private static Map<String, Object> entityNbt(Object level, Object entity) throws Exception {
        Object value = NativeAccess.call(NativeAccess.type("world.level.storage.TagValueOutput"), "createWithContext",
            NativeAccess.constant("util.ProblemReporter", "DISCARDING"), NativeAccess.call(level, "registryAccess"));
        boolean saved = (boolean) NativeAccess.call(entity, "save", value);
        return NativeAccess.map("saved", saved, "nbt", nbtBlob(NativeAccess.call(value, "buildResult")),
            "position_bits", List.of(Long.toUnsignedString(Double.doubleToRawLongBits((double) NativeAccess.call(entity, "getX"))),
                Long.toUnsignedString(Double.doubleToRawLongBits((double) NativeAccess.call(entity, "getY"))),
                Long.toUnsignedString(Double.doubleToRawLongBits((double) NativeAccess.call(entity, "getZ")))));
    }

    private static Map<String, Object> snapshot(Object level, Object chunk) throws Exception {
        boolean observing = OBSERVING.get();
        OBSERVING.set(true);
        try {
            if (chunk.getClass().getSimpleName().equals("ImposterProtoChunk")) chunk = NativeAccess.call(chunk, "getWrapped");
            Object copy = NativeAccess.call(NativeAccess.type("world.level.chunk.storage.SerializableChunkData"), "copyOf", level, chunk);
            Object nbt = NativeAccess.call(copy, "write");
            Map<String, Object> heightmaps = new LinkedHashMap<>();
            for (Object value : (Collection<?>) NativeAccess.call(chunk, "getHeightmaps")) {
                Map.Entry<?, ?> entry = (Map.Entry<?, ?>) value;
                heightmaps.put(((Enum<?>) entry.getKey()).name(), ((long[]) NativeAccess.call(entry.getValue(), "getRawData")).clone());
            }
            List<Object> starts = new ArrayList<>();
            Object registry = NativeAccess.call(NativeAccess.call(level, "registryAccess"), "lookupOrThrow",
                NativeAccess.constant("core.registries.Registries", "STRUCTURE"));
            for (Map.Entry<?, ?> entry : ((Map<?, ?>) NativeAccess.call(chunk, "getAllStarts")).entrySet()) {
                starts.add(NativeAccess.map("key", NativeAccess.call(registry, "getKey", entry.getKey()).toString(),
                    "identity", identity(entry.getValue()), "valid", NativeAccess.call(entry.getValue(), "isValid")));
            }
            Map<String, Object> extra = NativeAccess.map("heightmaps_live", heightmaps,
                "is_unsaved", NativeAccess.call(chunk, "isUnsaved"), "chunk_class", chunk.getClass().getName());
            List<Object> samples = new ArrayList<>();
            List<Object> sectionCounts = new ArrayList<>();
            Object[] sections = (Object[]) NativeAccess.call(chunk, "getSections");
            int minY = (int) NativeAccess.call(chunk, "getMinY");
            for (int section = 0; section < sections.length; section++) {
                Object data = sections[section];
                sectionCounts.add(NativeAccess.map("non_empty", NativeAccess.field(data, "nonEmptyBlockCount"),
                    "fluid", NativeAccess.field(data, "fluidCount"), "ticking_blocks", NativeAccess.field(data, "tickingBlockCount"),
                    "ticking_fluids", NativeAccess.field(data, "tickingFluidCount")));
                for (int i : new int[]{0, 51, 2047, 4095}) {
                    Object state = NativeAccess.call(data, "getBlockState", i & 15, i >> 8, (i >> 4) & 15);
                    samples.add(List.of(section * 4096 + i, NativeAccess.call(NativeAccess.type("world.level.block.Block"), "getId", state)));
                }
            }
            extra.put("sample_state_ids", samples);
            extra.put("section_counts", sectionCounts);
            extra.put("min_y", minY);
            List<Object> levelEntities = new ArrayList<>();
            if (status(NativeAccess.call(chunk, "getPersistedStatus")).equals("minecraft:full")) {
                List<Integer> chunkPosition = chunkPos(NativeAccess.call(chunk, "getPos"));
                for (Object entity : (Iterable<?>) NativeAccess.call(level, "getAllEntities")) {
                    if (chunkPos(NativeAccess.call(entity, "chunkPosition")).equals(chunkPosition)) levelEntities.add(entityNbt(level, entity));
                }
            }
            extra.put("level_entities", levelEntities);
            String extraJson = (String) NativeAccess.call(gson, "toJson", extra);
            return NativeAccess.map("pos", chunkPos(NativeAccess.call(chunk, "getPos")),
                "status", status(NativeAccess.call(chunk, "getPersistedStatus")), "chunk", identity(chunk),
                "nbt", nbtBlob(nbt), "extra", blob(extraJson.getBytes(StandardCharsets.UTF_8), "json"),
                "starts_iteration", starts);
        } finally { OBSERVING.set(observing); }
    }

    private static void stateRegistry() throws Exception {
        List<Object> states = new ArrayList<>();
        Object registry = NativeAccess.constant("world.level.block.Block", "BLOCK_STATE_REGISTRY");
        for (Object state : (Iterable<?>) registry) {
            Map<String, Object> properties = new TreeMap<>();
            for (Object property : (Collection<?>) NativeAccess.call(state, "getProperties")) {
                properties.put((String) NativeAccess.call(property, "getName"),
                    NativeAccess.call(property, "getName", NativeAccess.call(state, "getValue", property)));
            }
            Object block = NativeAccess.call(state, "getBlock");
            String key = NativeAccess.call(NativeAccess.constant("core.registries.BuiltInRegistries", "BLOCK"), "getKey", block).toString();
            states.add(NativeAccess.map("id", NativeAccess.call(registry, "getId", state), "Name", key, "Properties", properties));
        }
        Files.writeString(output.resolve("block-states.json"), (String) NativeAccess.call(gson, "toJson", states),
            StandardCharsets.UTF_8, StandardOpenOption.CREATE_NEW);
    }

    private static void run(Object level) throws Exception {
        if (running) throw new IllegalStateException("duplicate oracle runner");
        running = true;
        Object source = NativeAccess.call(level, "getChunkSource");
        Object map = NativeAccess.field(source, "chunkMap");
        emit("bootstrap_complete", NativeAccess.map("mode", config.get("bootstrap"),
            "holders", NativeAccess.call(map, "size"), "game_time", NativeAccess.call(level, "getGameTime"),
            "native_seed", NativeAccess.call(level, "getSeed")));
        stateRegistry();
        phase = "requests";
        for (Object value : (List<?>) config.get("requests")) {
            request++;
            Map<?, ?> spec = (Map<?, ?>) value;
            List<?> pos = (List<?>) spec.get("pos");
            int x = ((Number) pos.get(0)).intValue(), z = ((Number) pos.get(1)).intValue();
            Object target = NativeAccess.constant("world.level.chunk.status.ChunkStatus", ((String) spec.get("status")).toUpperCase(Locale.ROOT));
            emit("request_enter", NativeAccess.map("spec", spec));
            Object chunk = NativeAccess.call(source, "getChunk", x, z, target, true);
            if (chunk == null) throw new IllegalStateException("native getChunk returned null");
            List<Object> watches = new ArrayList<>();
            for (Object watchValue : (List<?>) config.get("watch")) {
                List<?> watch = (List<?>) watchValue;
                int wx = ((Number) watch.get(0)).intValue(), wz = ((Number) watch.get(1)).intValue();
                long packed = (wx & 0xffffffffL) | ((wz & 0xffffffffL) << 32);
                Object holder = NativeAccess.call(map, "getUpdatingChunkIfPresent", packed);
                Object watched = holder == null ? null : NativeAccess.call(holder, "getLatestChunk");
                watches.add(watched == null ? NativeAccess.map("pos", List.of(wx, wz), "absent", true) : snapshot(level, watched));
            }
            emit("request_exit", NativeAccess.map("spec", spec, "snapshot", snapshot(level, chunk), "watch", watches,
                "game_time", NativeAccess.call(level, "getGameTime")));
        }
        phase = "complete";
        emit("oracle_complete", NativeAccess.map("gameplay_ticks", TICKS.get(), "feature_sources", sources,
            "game_time", NativeAccess.call(level, "getGameTime"), "instrumented", INSTRUMENTED));
        trace.close();
        // This is an ephemeral observation process. Server shutdown/save/ticks are outside this history.
        Runtime.getRuntime().halt(0);
    }
}
