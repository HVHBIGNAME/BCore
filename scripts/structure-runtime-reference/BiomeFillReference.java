import java.lang.reflect.*;
import java.util.*;
import java.util.concurrent.*;

/** Records actual ChunkAccess/LevelChunkSection biome-fill order and outcomes. */
public class BiomeFillReference extends StructureRuntimeReference {
    static Map<String, Object> fill(int x, int z) throws Exception {
        Object chunk = markChunk(x,z);
        Object noise = call(generator,"createNoiseChunk",chunk,structureManager,
            call(type("world.level.levelgen.blending.Blender"),"empty"),randomState);
        Object settings = call(holder("NOISE_SETTINGS","overworld"),"value");
        Object sampler = call(noise,"cachedClimateSampler",call(randomState,"router"),call(settings,"spawnTarget"));
        Object source = call(generator,"getBiomeSource");
        List<List<Integer>> calls = new ArrayList<>();
        Object resolver = Proxy.newProxyInstance(BiomeFillReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.biome.BiomeResolver")},(p,m,a)->{
                if (!m.getName().equals("getNoiseBiome")) throw new UnsupportedOperationException(m.toString());
                calls.add(List.of((int)a[0],(int)a[1],(int)a[2]));
                return call(source,"getNoiseBiome",a);
            });
        call(chunk,"fillBiomesFromNoise",resolver,sampler);
        List<String> values = new ArrayList<>();
        Object[] sections = (Object[])call(chunk,"getSections");
        for (int y=-16;y<80;y++) for (int qz=0;qz<4;qz++) for (int qx=0;qx<4;qx++) {
            // ChunkStatusTasks sets BIOMES after this operation. Inspect the
            // filled section directly rather than bypassing ProtoChunk's guard.
            Object biome = call(call(sections[(y+16)>>2],"getNoiseBiome",qx,y&3,qz),"value");
            values.add(call(registry("BIOME"),"getKey",biome).toString());
        }
        return Map.of("chunk",List.of(x,z),"queries",calls,"biomes_yzx",values);
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        List<Object> histories = new ArrayList<>();
        for (long seed : new long[]{846692123413862008L,0,42}) {
            setup(seed,"overworld","trial_chambers");
            FutureTask<List<Object>> task = new FutureTask<>(()->{
                List<Object> rows = new ArrayList<>();
                for (int[] p : new int[][]{{14,8},{13,7},{14,8},{-3,-1}}) rows.add(fill(p[0],p[1]));
                return rows;
            });
            // A native RTree owns a ThreadLocal last-result. Each history starts
            // cold on its own thread and keeps that same thread for all chunks.
            Thread thread = new Thread(task,"native-biome-fill");
            thread.start(); histories.add(Map.of("seed",seed,"chunks",task.get())); thread.join();
        }
        output("STRUCTURERUNTIMEREFERENCE",Map.of("histories",histories,"scope",
            "Actual native ChunkAccess.fillBiomesFromNoise with its real cached NoiseChunk climate sampler and MultiNoiseBiomeSource. Resolver observation records call order; reading the filled palette adds no biome-source queries. Cold native threads retained per multi-chunk history."));
        call(resources,"close");
    }
}
