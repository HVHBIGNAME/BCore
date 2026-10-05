import java.lang.reflect.*;
import java.nio.*;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.CompletableFuture;

/** Times the real 26.1 fillFromNoise kernel; no instrumentation inside its task. */
public final class NoiseFillBenchmark extends DensityMaterialsReference {
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
            "marks_sha256", HexFormat.of().formatHex(marks.digest()), "marks", count);
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        @SuppressWarnings("unchecked") Map<String, Object> request = (Map<String, Object>)
            call(gson, "fromJson", Files.readString(Path.of(args[0])), Map.class);
        settings = call(call(registry("NOISE_SETTINGS"), "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD")), "value");
        noises = registry("NOISE");
        long seed = Long.parseLong((String) request.get("seed"));
        int warmup = ((Number) request.get("warmup_batches")).intValue();
        int measured = ((Number) request.get("measured_batches")).intValue();
        int workers = ((Number) request.get("workers")).intValue();
        @SuppressWarnings("unchecked") List<List<Number>> coordinates = (List<List<Number>>) request.get("chunks");
        Object random = randomState(seed);
        Object blender = call(type("world.level.levelgen.blending.Blender"), "empty");
        List<Double> seconds = new ArrayList<>();
        List<Object> reference = null;
        for (int batch = 0; batch < warmup + measured; batch++) {
            List<Selected> inputs = new ArrayList<>();
            for (List<Number> p : coordinates) inputs.add(select(List.of(), p.get(0).intValue(), p.get(1).intValue()));
            // Paletted-container allocation, registry setup and fingerprints are untimed.
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
        }
        output("NOISE_BENCHMARK", Map.of("engine", "Vanilla 26.1", "scope", "noise-fill-kernel",
            "seed", Long.toString(seed), "workers", workers, "warmup_batches", warmup,
            "chunks_per_batch", coordinates.size(), "seconds", seconds, "fingerprints", reference));
        call(resources, "close");
    }
}
