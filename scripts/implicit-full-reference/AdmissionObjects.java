import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Queue;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicReferenceArray;

/** Read-only object descriptions; all identities use == and an independent ID space. */
final class AdmissionObjects {
    private record Identity(Object value, long id) { }
    private static final Map<String, List<Identity>> IDENTITIES = new HashMap<>();
    private static long nextIdentity;

    static long id(Object value) {
        if (value == null) return 0;
        synchronized (HistoryProbe.class) {
            List<Identity> entries = IDENTITIES.computeIfAbsent(value.getClass().getName(), key -> new ArrayList<>());
            for (Identity entry : entries) if (entry.value == value) return entry.id;
            long id = ++nextIdentity;
            entries.add(new Identity(value, id));
            return id;
        }
    }

    static Map<String, Object> object(Object value) {
        return value == null ? null : NativeAccess.map("object_id", id(value), "class", value.getClass().getName());
    }

    static String status(Object value) throws Exception {
        return value == null ? null : (String) NativeAccess.call(value, "getName");
    }

    static List<Integer> position(Object value) throws Exception {
        Object pos = NativeAccess.call(value, "getPos");
        return List.of((int) NativeAccess.call(pos, "x"), (int) NativeAccess.call(pos, "z"));
    }

    static Map<String, Object> future(Object value) {
        if (value == null) return null;
        CompletableFuture<?> future = (CompletableFuture<?>) value;
        Map<String, Object> row = object(future);
        row.put("done", future.isDone());
        return row;
    }

    static Map<String, Object> chunk(Object value) throws Exception {
        if (value == null) return null;
        Map<String, Object> row = object(value);
        row.put("pos", position(value));
        row.put("status", status(NativeAccess.call(value, "getPersistedStatus")));
        if (value.getClass().getName().equals("net.minecraft.world.level.chunk.ImposterProtoChunk"))
            row.put("wrapped", object(NativeAccess.call(value, "getWrapped")));
        return row;
    }

    static Map<String, Object> holder(Object value) throws Exception {
        Map<String, Object> row = object(value);
        row.put("pos", position(value));
        row.put("ticket_level", NativeAccess.call(value, "getTicketLevel"));
        row.put("queue_level", NativeAccess.call(value, "getQueueLevel"));
        row.put("old_ticket_level", NativeAccess.field(value, "oldTicketLevel"));
        row.put("allowed_status", status(NativeAccess.field(value, "highestAllowedStatus")));
        row.put("was_accessible_since_last_save", NativeAccess.field(value, "wasAccessibleSinceLastSave"));
        row.put("full_chunk_future", future(NativeAccess.field(value, "fullChunkFuture")));
        row.put("full_state_confirmation", future(NativeAccess.field(value, "pendingFullStateConfirmation")));
        AtomicReferenceArray<?> futures = (AtomicReferenceArray<?>) NativeAccess.field(value, "futures");
        List<Object> statuses = new ArrayList<>();
        for (int i = 0; i < futures.length(); i++) statuses.add(NativeAccess.map("status_index", i, "future", future(futures.get(i))));
        row.put("status_futures", statuses);
        return row;
    }

    static List<Object> cache(Object cache) throws Exception {
        long[] positions = (long[]) NativeAccess.field(cache, "lastChunkPos");
        Object[] statuses = (Object[]) NativeAccess.field(cache, "lastChunkStatus");
        Object[] chunks = (Object[]) NativeAccess.field(cache, "lastChunk");
        List<Object> rows = new ArrayList<>();
        for (int i = 0; i < positions.length; i++) rows.add(NativeAccess.map(
            "slot", i, "packed_pos", Long.toString(positions[i]),
            "pos", List.of((int) positions[i], (int) (positions[i] >> 32)),
            "requested_status", status(statuses[i]), "chunk", chunk(chunks[i])));
        return rows;
    }

    static Map<String, Object> pending(Object executor) throws Exception {
        Object[] values = ((Queue<?>) NativeAccess.field(executor, "pendingRunnables")).toArray();
        List<Object> rows = new ArrayList<>();
        for (int i = 0; i < Math.min(values.length, 256); i++) rows.add(object(values[i]));
        return NativeAccess.map("runnables", rows, "observed_count", values.length, "truncated", values.length > 256,
            "consistency", "read-only weakly-consistent queue observation");
    }
}
