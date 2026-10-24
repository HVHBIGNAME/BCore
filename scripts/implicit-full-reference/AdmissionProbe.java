import java.io.BufferedWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.atomic.AtomicLong;

/** Versioned sidecar. No native queue, future, supplier, or returned object is replaced. */
public final class AdmissionProbe {
    private static final String CACHE = "server/level/ServerChunkCache.";
    private static final String MAIN = "server/level/ServerChunkCache$MainThreadExecutor.";
    private static final String LOOP = "util/thread/BlockableEventLoop.";
    private static final String HOLDER = "server/level/ChunkHolder.";
    private static final String GENERATION = "server/level/GenerationChunkHolder.";
    private static final String MAP = "server/level/ChunkMap.";
    private static final String TASKS = "world/level/chunk/status/ChunkStatusTasks.";
    private static final String LEVEL = "world/level/chunk/LevelChunk.";
    private static final String EXECUTOR = "net.minecraft.server.level.ServerChunkCache$MainThreadExecutor";
    private static final ThreadLocal<Deque<Frame>> FRAMES = ThreadLocal.withInitial(ArrayDeque::new);
    private static final Set<Long> WATCH = new HashSet<>();
    private static BufferedWriter writer;
    private static Object gson;
    private static AtomicLong baselineSequence;
    private static Field phaseField, requestField, stoppedField;
    private static ThreadLocal<?> observing, activeStage;
    private static long sequence, nextCall;

    private static final class Frame {
        final String method;
        final Object self;
        final Object[] args;
        boolean active;
        long id, entryBaseline, baselineStage;
        Object future, supplier;
        Frame parentApi;
        int futureCalls, pumpCalls, polls, workedPolls, mainRuns, distanceTrue, distanceFalse;
        Frame(String method, Object self, Object[] args) {
            this.method = method;
            this.self = self;
            this.args = args;
        }
    }

    private static Field historyField(String name) throws Exception {
        Field field = HistoryProbe.class.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    static void initialize() throws Exception {
        synchronized (HistoryProbe.class) {
            gson = NativeAccess.field(HistoryProbe.class, "gson");
            baselineSequence = (AtomicLong) NativeAccess.field(HistoryProbe.class, "SEQUENCE");
            phaseField = historyField("phase");
            requestField = historyField("request");
            stoppedField = historyField("stopped");
            observing = (ThreadLocal<?>) NativeAccess.field(HistoryProbe.class, "OBSERVING");
            activeStage = (ThreadLocal<?>) NativeAccess.field(HistoryProbe.class, "ACTIVE");
            Map<?, ?> config = (Map<?, ?>) NativeAccess.field(HistoryProbe.class, "config");
            for (Object value : (List<?>) config.get("watch")) {
                List<?> pos = (List<?>) value;
                int x = ((Number) pos.get(0)).intValue(), z = ((Number) pos.get(1)).intValue();
                for (int dx = -1; dx <= 1; dx++) for (int dz = -1; dz <= 1; dz++) WATCH.add(pack(x + dx, z + dz));
            }
            Path output = (Path) NativeAccess.field(HistoryProbe.class, "output");
            writer = Files.newBufferedWriter(output.resolve("admission-events.jsonl"), StandardCharsets.UTF_8,
                StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE);
            emit("admission_start", NativeAccess.map("contract", "private-companion-v1", "identity_space", "companion-only; == comparison",
                "baseline_sources_unchanged", true, "queue_and_future_wrapping", false,
                "baseline_observer_calls", "included and labeled baseline_observing",
                "terminal_contract", "prefix sealed by baseline oracle_complete and successful native process halt"));
        }
    }

    private static long pack(int x, int z) { return (x & 0xffffffffL) | ((z & 0xffffffffL) << 32); }

    private static boolean watched(Object value) throws Exception {
        List<Integer> pos = AdmissionObjects.position(value);
        return WATCH.contains(pack(pos.get(0), pos.get(1)));
    }

    private static boolean enabled() throws Exception {
        return writer != null && !stoppedField.getBoolean(null) && phaseField.get(null).equals("requests");
    }

    private static Frame nearest(String method) {
        for (Frame frame : FRAMES.get()) if (frame.active && frame.method.equals(method)) return frame;
        return null;
    }

    private static void context(Map<String, Object> row, String key, String method) {
        Frame frame = nearest(method);
        if (frame != null) row.put(key, frame.id);
    }

    // All callers hold exactly the same monitor used by HistoryProbe.emit.
    private static void emit(String event, Map<String, Object> fields) throws Exception {
        Map<String, Object> row = NativeAccess.map("schema", 1, "seq", ++sequence, "baseline_seq", baselineSequence.get(),
            "event", event, "phase", phaseField.get(null), "request", requestField.getInt(null),
            "baseline_observing", Boolean.TRUE.equals(observing.get()),
            "thread_id", Thread.currentThread().threadId(), "thread", Thread.currentThread().getName());
        context(row, "api_call_id", CACHE + "getChunk");
        context(row, "pump_call_id", LOOP + "managedBlock");
        context(row, "execution_call_id", MAIN + "doRunTask");
        context(row, "full_call_id", TASKS + "full");
        row.putAll(fields);
        writer.write((String) NativeAccess.call(gson, "toJson", row));
        writer.newLine();
        writer.flush();
    }

    static void transformed(String owner, List<String> methods, String before, String after) throws Exception {
        synchronized (HistoryProbe.class) {
            if (!stoppedField.getBoolean(null)) emit("transformed_class", NativeAccess.map("owner", owner, "methods", methods,
                "input_after_baseline_transform_sha256", before, "output_sha256", after));
        }
    }

    private static boolean eligible(Frame frame) throws Exception {
        if (!enabled()) return false;
        String method = frame.method;
        if (method.startsWith(LOOP) || method.startsWith(MAIN)) return frame.self.getClass().getName().equals(EXECUTOR);
        if (method.startsWith(HOLDER) || method.startsWith(GENERATION) || method.startsWith(LEVEL)) return watched(frame.self);
        if (method.startsWith(MAP)) return watched(frame.args[0]);
        if (method.equals(TASKS + "full")) return watched(frame.args[3]);
        if (method.equals(TASKS + "lambda$full$0")) return watched(frame.args[0]);
        return true;
    }

    private static Map<String, Object> row(Frame frame) {
        return NativeAccess.map("call_id", frame.id, "method", frame.method, "entry_baseline_seq", frame.entryBaseline);
    }

    private static Map<String, Object> requestRow(Frame frame) throws Exception {
        Map<String, Object> row = row(frame);
        row.put("cache", AdmissionObjects.object(frame.self));
        row.put("pos", List.of(frame.args[0], frame.args[1]));
        row.put("requested_status", AdmissionObjects.status(frame.args[2]));
        row.put("create", frame.args[3]);
        return row;
    }

    private static Map<String, Object> counters(Frame frame) {
        return NativeAccess.map("future_calls", frame.futureCalls, "pump_calls", frame.pumpCalls,
            "polls", frame.polls, "worked_polls", frame.workedPolls, "main_runs", frame.mainRuns,
            "distance_updates_true", frame.distanceTrue, "distance_updates_false", frame.distanceFalse);
    }

    private static List<String> callers() {
        List<String> result = new ArrayList<>();
        for (StackTraceElement entry : Thread.currentThread().getStackTrace()) {
            if (entry.getClassName().startsWith("net.minecraft.") || entry.getClassName().startsWith("java.util.concurrent.CompletableFuture"))
                result.add(entry.getClassName() + "." + entry.getMethodName());
            if (result.size() == 12) break;
        }
        return result;
    }

    public static void enter(String method, Object self, Object[] args) {
        try {
            synchronized (HistoryProbe.class) {
                Frame frame = new Frame(method, self, args);
                frame.active = eligible(frame);
                frame.entryBaseline = baselineSequence.get();
                frame.parentApi = nearest(CACHE + "getChunk");
                if (frame.active) frame.id = ++nextCall;
                FRAMES.get().push(frame);
                if (frame.active) entered(frame);
            }
        } catch (Throwable failure) { HistoryProbe.fatal(failure); }
    }

    private static void entered(Frame frame) throws Exception {
        String method = frame.method;
        Map<String, Object> row = row(frame);
        if (method.equals(CACHE + "getChunk")) {
            row = requestRow(frame);
            if (frame.parentApi != null) row.put("parent_api_call_id", frame.parentApi.id);
            row.put("cache_slots", AdmissionObjects.cache(frame.self));
            emit("api_enter", row);
        } else if (method.equals(CACHE + "getChunkFutureMainThread")) {
            Frame api = nearest(CACHE + "getChunk");
            if (api != null) api.futureCalls++;
            emit("api_future_enter", requestRow(frame));
        } else if (method.equals(CACHE + "storeInCache") || method.equals(CACHE + "clearCache")) {
            row.put("cache", AdmissionObjects.object(frame.self));
            row.put("cache_slots", AdmissionObjects.cache(frame.self));
            if (frame.args.length != 0) {
                row.put("packed_pos", frame.args[0].toString());
                row.put("stored_chunk", AdmissionObjects.chunk(frame.args[1]));
                row.put("stored_status", AdmissionObjects.status(frame.args[2]));
            }
            emit(method.endsWith("storeInCache") ? "cache_store_enter" : "cache_clear_enter", row);
        } else if (method.equals(LOOP + "managedBlock")) {
            Frame api = nearest(CACHE + "getChunk");
            if (api != null) { api.pumpCalls++; frame.future = api.future; }
            row.put("requested_future", AdmissionObjects.future(frame.future));
            row.put("future_known", frame.future != null);
            row.put("executor", AdmissionObjects.object(frame.self));
            row.put("pending", AdmissionObjects.pending(frame.self));
            emit("pump_enter", row);
        } else if (method.equals(LOOP + "schedule") || method.equals(MAIN + "doRunTask")) {
            row.put("executor", AdmissionObjects.object(frame.self));
            row.put("runnable", AdmissionObjects.object(frame.args[0]));
            if (method.equals(LOOP + "schedule")) row.put("callers", callers());
            else {
                Frame api = nearest(CACHE + "getChunk"), pump = nearest(LOOP + "managedBlock");
                if (api != null) api.mainRuns++;
                if (pump != null) pump.mainRuns++;
            }
            emit(method.equals(LOOP + "schedule") ? "main_schedule_enter" : "main_execute_enter", row);
        } else if (method.equals(HOLDER + "updateFutures") || method.equals(GENERATION + "scheduleChunkGenerationTask")) {
            row.put("holder", AdmissionObjects.holder(frame.self));
            if (method.startsWith(GENERATION)) row.put("requested_status", AdmissionObjects.status(frame.args[0]));
            emit(method.startsWith(HOLDER) ? "holder_update_enter" : "holder_future_enter", row);
        } else if (method.equals(MAP + "prepareAccessibleChunk")) {
            row.put("holder", AdmissionObjects.holder(frame.args[0]));
            context(row, "holder_update_call_id", HOLDER + "updateFutures");
            emit("accessible_enter", row);
        } else if (method.equals(TASKS + "full")) {
            Object stage = activeStage.get();
            if (stage != null) frame.baselineStage = ((Number) NativeAccess.field(stage, "id")).longValue();
            row.put("baseline_stage_id", frame.baselineStage);
            row.put("input_chunk", AdmissionObjects.chunk(frame.args[3]));
            emit("full_enter", row);
        } else if (method.equals(TASKS + "lambda$full$0")) {
            row.put("input_chunk", AdmissionObjects.chunk(frame.args[0]));
            row.put("holder", AdmissionObjects.holder(frame.args[2]));
            emit("full_body_enter", row);
        } else if (method.equals(LEVEL + "setFullStatus")) {
            row.put("chunk", AdmissionObjects.chunk(frame.self));
            row.put("supplier", AdmissionObjects.object(frame.args[0]));
            context(row, "full_body_call_id", TASKS + "lambda$full$0");
            emit("full_status_supplier_enter", row);
        }
    }

    public static void fullSubmission(Object supplier, Object executor) {
        try {
            synchronized (HistoryProbe.class) {
                Frame frame = nearest(TASKS + "full");
                if (frame == null || !enabled()) return;
                frame.supplier = supplier;
                emit("full_supplier", NativeAccess.map("call_id", frame.id, "baseline_stage_id", frame.baselineStage,
                    "supplier", AdmissionObjects.object(supplier), "executor", AdmissionObjects.object(executor)));
            }
        } catch (Throwable failure) { HistoryProbe.fatal(failure); }
    }

    public static void exit(Object result, String method, Object self, Object[] args) {
        try {
            synchronized (HistoryProbe.class) {
                Frame frame = FRAMES.get().peek();
                if (frame == null || !frame.method.equals(method) || frame.self != self)
                    throw new IllegalStateException("unpaired companion return: " + method);
                if (frame.active && !stoppedField.getBoolean(null)) returned(frame, result);
                FRAMES.get().pop();
            }
        } catch (Throwable failure) { HistoryProbe.fatal(failure); }
    }

    private static void returned(Frame frame, Object result) throws Exception {
        String method = frame.method;
        Map<String, Object> row = row(frame);
        if (method.equals(CACHE + "getChunk")) {
            row = requestRow(frame);
            row.put("returned", AdmissionObjects.chunk(result));
            row.put("counters", counters(frame));
            row.put("cache_slots", AdmissionObjects.cache(frame.self));
            emit("api_return", row);
        } else if (method.equals(CACHE + "getChunkFutureMainThread")) {
            Frame api = nearest(CACHE + "getChunk");
            if (api != null) api.future = result;
            row = requestRow(frame);
            row.put("future", AdmissionObjects.future(result));
            emit("api_future_return", row);
        } else if (method.equals(CACHE + "storeInCache") || method.equals(CACHE + "clearCache")) {
            row.put("cache", AdmissionObjects.object(frame.self));
            row.put("cache_slots", AdmissionObjects.cache(frame.self));
            emit(method.endsWith("storeInCache") ? "cache_store_return" : "cache_clear_return", row);
        } else if (method.equals(LOOP + "managedBlock")) {
            row.put("requested_future", AdmissionObjects.future(frame.future));
            row.put("executor", AdmissionObjects.object(frame.self));
            row.put("pending", AdmissionObjects.pending(frame.self));
            row.put("counters", counters(frame));
            emit("pump_return", row);
        } else if (method.equals(MAIN + "pollTask") || method.equals(CACHE + "runDistanceManagerUpdates")) {
            Frame api = nearest(CACHE + "getChunk"), pump = nearest(LOOP + "managedBlock");
            boolean worked = Boolean.TRUE.equals(result);
            for (Frame target : new Frame[]{api, pump}) {
                if (target == null) continue;
                if (method.equals(MAIN + "pollTask")) { target.polls++; if (worked) target.workedPolls++; }
                else if (worked) target.distanceTrue++;
                else target.distanceFalse++;
            }
            if (method.equals(CACHE + "runDistanceManagerUpdates") && worked) emit("distance_update_true", row);
        } else if (method.equals(LOOP + "schedule") || method.equals(MAIN + "doRunTask")) {
            row.put("executor", AdmissionObjects.object(frame.self));
            row.put("runnable", AdmissionObjects.object(frame.args[0]));
            emit(method.equals(LOOP + "schedule") ? "main_schedule_return" : "main_execute_return", row);
        } else if (method.equals(HOLDER + "updateFutures") || method.equals(GENERATION + "scheduleChunkGenerationTask")) {
            row.put("holder", AdmissionObjects.holder(frame.self));
            if (method.startsWith(GENERATION)) {
                row.put("requested_status", AdmissionObjects.status(frame.args[0]));
                row.put("future", AdmissionObjects.future(result));
            }
            emit(method.startsWith(HOLDER) ? "holder_update_return" : "holder_future_return", row);
        } else if (method.equals(MAP + "prepareAccessibleChunk")) {
            row.put("holder", AdmissionObjects.holder(frame.args[0]));
            row.put("future", AdmissionObjects.future(result));
            context(row, "holder_update_call_id", HOLDER + "updateFutures");
            emit("accessible_return", row);
        } else if (method.equals(TASKS + "full")) {
            row.put("baseline_stage_id", frame.baselineStage);
            row.put("supplier", AdmissionObjects.object(frame.supplier));
            row.put("future", AdmissionObjects.future(result));
            emit("full_return", row);
        } else if (method.equals(TASKS + "lambda$full$0")) {
            row.put("returned", AdmissionObjects.chunk(result));
            row.put("holder", AdmissionObjects.holder(frame.args[2]));
            emit("full_body_return", row);
        } else if (method.equals(LEVEL + "setFullStatus")) {
            row.put("chunk", AdmissionObjects.chunk(frame.self));
            row.put("supplier", AdmissionObjects.object(frame.args[0]));
            emit("full_status_supplier_return", row);
        }
    }
}
