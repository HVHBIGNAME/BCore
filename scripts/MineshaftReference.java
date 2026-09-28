import java.util.*;

/** Native mineshaft piece graph and normal-mine vertical adjustment. */
public class MineshaftReference extends TreeReference {
    static Object generator, biomeSource, settings, noiseRegistry, height;
    static Map<String, Object> probe(long seed, int cx, int cz, String kind) throws Exception {
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.LegacyRandomSource", seed));
        call(random, "setLargeFeatureSeed", seed, cx, cz);
        Object type = field("world.level.levelgen.structure.structures.MineshaftStructure$Type", kind);
        Object randomState = call(type("world.level.levelgen.RandomState"),"create",settings,noiseRegistry,seed);
        // This operation only queries generator/random/position/height;
        // structure admission, registry access and templates are outside this probe.
        Object context = make("world.level.levelgen.structure.Structure$GenerationContext", null,generator,biomeSource,randomState,null,random,seed,
            make("world.level.ChunkPos",cx,cz),height,null);
        Object structure = make("world.level.levelgen.structure.structures.MineshaftStructure",null,type);
        Object stub = ((Optional<?>) call(structure,"findGenerationPoint",context)).orElseThrow();
        Object position = call(stub,"position");
        Object builder = call(stub,"getPiecesBuilder");
        int dy = (int) call(position,"getY")-50;
        Object center = call(call(builder,"getBoundingBox"),"getCenter");
        int baseHeight = (int) call(generator,"getBaseHeight",call(center,"getX"),call(center,"getZ"),field("world.level.levelgen.Heightmap$Types","WORLD_SURFACE_WG"),height,randomState);
        List<Object> pieces = new ArrayList<>();
        Object jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        for (Object piece : (List<?>) call(call(builder, "build"), "pieces")) {
            Object tag = call(piece, "createTag", (Object) null);
            pieces.add(call(field("nbt.NbtOps", "INSTANCE"), "convertTo", jsonOps, tag));
        }
        return Map.of("seed", seed, "chunk", List.of(cx,cz), "type", kind, "vertical_offset", dy, "base_height",baseHeight,"pieces", pieces,
            "generation_point",List.of(call(position,"getX"),call(position,"getY"),call(position,"getZ")),
            "base_height_position",List.of(call(center,"getX"),call(center,"getZ")),
            "next_i64", call(random, "nextLong"));
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion"); call(type("server.Bootstrap"), "bootStrap");
        Object lookup = call(type("data.registries.VanillaRegistries"),"createLookup");
        Object holder = call(call(lookup,"lookupOrThrow",field("core.registries.Registries","NOISE_SETTINGS")),"getOrThrow",field("world.level.levelgen.NoiseGeneratorSettings","OVERWORLD"));
        settings = call(holder,"value");
        noiseRegistry = call(lookup,"lookupOrThrow",field("core.registries.Registries","NOISE"));
        biomeSource = make("world.level.biome.FixedBiomeSource",call(type("core.Holder"),"direct","unused by layout/base-height probe"));
        generator = make("world.level.levelgen.NoiseBasedChunkGenerator",biomeSource,holder);
        height = call(type("world.level.LevelHeightAccessor"),"create",-64,384);
        List<Object> samples = new ArrayList<>();
        for (long seed : new long[]{0,1,-1,846692123413862008L,Long.MIN_VALUE,Long.MAX_VALUE})
            for (int[] pos : new int[][]{{0,0},{-34,3},{-125,187}})
                for (String kind : List.of("NORMAL","MESA")) samples.add(probe(seed,pos[0],pos[1],kind));
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("MINESHAFT_REFERENCE=" + call(gson,"toJson",Map.of("samples",samples)));
    }
}
