import java.lang.reflect.*;
import java.nio.*;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ForkJoinPool;

/** Times the real 26.1 fillFromNoise kernel; no instrumentation inside its task. */
public final class NoiseFillBenchmark extends DensityMaterialsReference {
    static final int SCHEMA = 2;
    static final String SCOPE = "noise-fill-wg-v2";

    static String heightmapFingerprint(Object chunk, String name) throws Exception {
        Object kind = field("world.level.levelgen.Heightmap$Types", name);
        // Never let output validation prime a missing map outside the timer.
        if (!(boolean) call(chunk, "hasPrimedHeightmap", kind))
            throw new AssertionError("fill did not create " + name);
        Object heights = call(chunk, "getOrCreateHeightmapUnprimed", kind);
        Method get = type("world.level.levelgen.Heightmap").getMethod("getFirstAvailable", int.class, int.class);
        ByteBuffer bytes = ByteBuffer.allocate(256 * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++)
            bytes.putInt((int) get.invoke(heights, x, z));
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes.array()));
    }

    static Map<String, Object> fingerprint(Object chunk, int cx, int cz) throws Exception {
        MessageDigest blocks = MessageDigest.getInstance("SHA-256");
        ByteBuffer states = ByteBuffer.allocate(384 * 256 * 4).order(ByteOrder.LITTLE_ENDIAN);
        Method get = type("world.level.chunk.LevelChunkSection").getMethod("getBlockState", int.class, int.class, int.class);
        Map<Object, Integer> ids = new IdentityHashMap<>();
        Object[] sections = (Object[]) call(chunk, "getSections");
        for (int y = -64; y < 320; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
            Object state = get.invoke(sections[(y + 64) >> 4], x, y & 15, z);
            Integer id = ids.get(state);
            if (id == null) { id = stateId(state); ids.put(state, id); }
            states.putInt(id);
        }
        MessageDigest marks = MessageDigest.getInstance("SHA-256");
        Object[] lists = (Object[]) call(chunk, "getPostProcessing");
        ByteBuffer mark = ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN);
        int count = 0;
        for (int section = 0; section < lists.length; section++) {
            if (lists[section] == null) continue;
            for (Object value : (Iterable<?>) lists[section]) {
                int packed = ((Number) value).intValue() & 0xffff;
                mark.clear();
                mark.putInt(packed & 15).putInt(-64 + section * 16 + ((packed >> 4) & 15)).putInt((packed >> 8) & 15);
                marks.update(mark.array()); count++;
            }
        }
        return Map.of("chunk", List.of(cx, cz),
            "blocks_sha256", HexFormat.of().formatHex(blocks.digest(states.array())),
            "marks_sha256", HexFormat.of().formatHex(marks.digest()), "marks", count,
            "world_surface_wg_sha256", heightmapFingerprint(chunk, "WORLD_SURFACE_WG"),
            "ocean_floor_wg_sha256", heightmapFingerprint(chunk, "OCEAN_FLOOR_WG"));
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        @SuppressWarnings("unchecked") Map<String, Object> request = (Map<String, Object>)
            call(gson, "fromJson", Files.readString(Path.of(args[0])), Map.class);
        if (((Number) request.get("schema")).intValue() != SCHEMA || !SCOPE.equals(request.get("scope")))
            throw new IllegalArgumentException("unsupported noise benchmark contract");
        settings = call(call(registry("NOISE_SETTINGS"), "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD")), "value");
        noises = registry("NOISE");
        long seed = Long.parseLong((String) request.get("seed"));
        int warmup = ((Number) request.get("warmup_batches")).intValue();
        int measured = ((Number) request.get("measured_batches")).intValue();
        int workers = ((Number) request.get("workers")).intValue();
        if (!Set.of(1, 2, 4).contains(workers) || warmup < 1 || measured < 1)
            throw new IllegalArgumentException("expected workers 1/2/4 and positive batch counts");
        Object service = call(call(type("util.Util"), "backgroundExecutor"), "service");
        if (!(service instanceof ForkJoinPool pool) || pool.getParallelism() != workers)
            throw new AssertionError("native background executor does not match requested workers: " + service);
        int actualWorkers = pool.getParallelism();
        int processors = Runtime.getRuntime().availableProcessors();
        if (processors != workers + 1) throw new AssertionError("ActiveProcessorCount mismatch: " + processors);
        @SuppressWarnings("unchecked") List<List<Number>> coordinates = (List<List<Number>>) request.get("chunks");
        Object random = randomState(seed);
        Object blender = call(type("world.level.levelgen.blending.Blender"), "empty");
        List<Double> seconds = new ArrayList<>();
        List<Double> warmupSeconds = new ArrayList<>();
        List<Object> reference = null;
        for (int batch = 0; batch < warmup + measured; batch++) {
            List<Selected> inputs = new ArrayList<>();
            for (List<Number> p : coordinates) inputs.add(select(List.of(), p.get(0).intValue(), p.get(1).intValue()));
            // Initial chunk allocation and world setup are untimed. Reflection
            // lookup/invocation, task dispatch/join, fresh NoiseChunk/aquifer
            // setup, palette/scratch work and WG updates remain inside the timer.
            long start = System.nanoTime();
            List<CompletableFuture<?>> tasks = new ArrayList<>();
            for (Selected input : inputs) tasks.add((CompletableFuture<?>)
                call(generator, "fillFromNoise", blender, random, input.manager(), input.chunk()));
            CompletableFuture.allOf(tasks.toArray(CompletableFuture[]::new)).join();
            double elapsed = (System.nanoTime() - start) / 1e9;
            List<Object> hashes = new ArrayList<>();
            for (int i = 0; i < inputs.size(); i++) hashes.add(fingerprint(inputs.get(i).chunk(),
                coordinates.get(i).get(0).intValue(), coordinates.get(i).get(1).intValue()));
            if (reference != null && !reference.equals(hashes)) throw new AssertionError("NOISE changed in batch " + batch);
            reference = hashes;
            if (batch >= warmup) seconds.add(elapsed);
            else warmupSeconds.add(elapsed);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("schema", SCHEMA); result.put("scope", SCOPE); result.put("engine", "Vanilla 26.1");
        result.put("seed", Long.toString(seed)); result.put("workers", actualWorkers);
        result.put("native_executor", "ForkJoinPool"); result.put("available_processors", processors);
        result.put("warmup_batches", warmup); result.put("chunks_per_batch", coordinates.size());
        result.put("seconds", seconds); result.put("warmup_seconds", warmupSeconds);
        result.put("fingerprints", reference);
        output("NOISE_BENCHMARK", result);
        call(resources, "close");
    }
}
