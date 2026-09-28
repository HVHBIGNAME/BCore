import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;

/** Native random-spread candidates and frequency gates; no biome or exclusion checks. */
public class StructureReference extends TreeReference {
    static final String PLACEMENT = "world.level.levelgen.structure.placement.";

    static Map<?, ?> placement(String name) throws Exception {
        String path = "/data/minecraft/worldgen/structure_set/" + name + ".json";
        try (var stream = StructureReference.class.getResourceAsStream(path)) {
            if (stream == null) throw new IllegalArgumentException("missing " + path);
            Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
            Map<?, ?> document = (Map<?, ?>) call(gson, "fromJson", new String(stream.readAllBytes(), StandardCharsets.UTF_8), Map.class);
            return (Map<?, ?>) document.get("placement");
        }
    }

    static Object nativePlacement(Map<?, ?> config) throws Exception {
        Class<?> reducer = type(PLACEMENT + "StructurePlacement$FrequencyReductionMethod");
        Class<?> spread = type(PLACEMENT + "RandomSpreadType");
        String method = config.containsKey("frequency_reduction_method") ? (String) config.get("frequency_reduction_method") : "default";
        String spreadName = config.containsKey("spread_type") ? (String) config.get("spread_type") : "linear";
        float frequency = config.containsKey("frequency") ? ((Number) config.get("frequency")).floatValue() : 1.0f;
        // Exclusion zones require a structure-state registry. This probe exercises
        // only getPotentialStructureChunk and applyAdditionalChunkRestrictions.
        return type(PLACEMENT + "RandomSpreadStructurePlacement").getConstructor(
            type("core.Vec3i"), reducer, float.class, int.class, Optional.class, int.class, int.class, spread
        ).newInstance(field("core.Vec3i", "ZERO"), reducer.getField(method.toUpperCase(Locale.ROOT)).get(null),
            frequency, ((Number) config.get("salt")).intValue(), Optional.empty(),
            ((Number) config.get("spacing")).intValue(), ((Number) config.get("separation")).intValue(),
            spread.getField(spreadName.toUpperCase(Locale.ROOT)).get(null));
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        List<Object> records = new ArrayList<>();
        Method candidate = type(PLACEMENT + "RandomSpreadStructurePlacement").getMethod("getPotentialStructureChunk", long.class, int.class, int.class);
        Method restrictions = type(PLACEMENT + "StructurePlacement").getMethod("applyAdditionalChunkRestrictions", int.class, int.class, long.class);
        Method getX = type("world.level.ChunkPos").getMethod("x");
        Method getZ = type("world.level.ChunkPos").getMethod("z");
        for (String name : List.of("villages", "mineshafts", "ancient_cities", "trial_chambers", "ocean_monuments", "woodland_mansions", "pillager_outposts", "buried_treasures", "test_default_frequency")) {
            Map<?, ?> config = name.equals("test_default_frequency")
                ? Map.of("spacing", 7, "separation", 2, "salt", 12345, "frequency", 0.37)
                : placement(name);
            Object nativeConfig = nativePlacement(config);
            List<Object> samples = new ArrayList<>();
            for (long seed : new long[]{0, 1, -1, 846692123413862008L, Long.MIN_VALUE, Long.MAX_VALUE}) {
                ByteBuffer bytes = ByteBuffer.allocate(65 * 65 * 9).order(ByteOrder.LITTLE_ENDIAN);
                int accepted = 0;
                for (int z = -32; z <= 32; z++) for (int x = -32; x <= 32; x++) {
                    Object pos = candidate.invoke(nativeConfig, seed, x, z);
                    int cx = (int) getX.invoke(pos), cz = (int) getZ.invoke(pos);
                    boolean allowed = (boolean) restrictions.invoke(nativeConfig, x, z, seed);
                    bytes.putInt(cx).putInt(cz).put((byte) (allowed ? 1 : 0));
                    if (allowed && cx == x && cz == z) accepted++;
                }
                samples.add(Map.of("seed", seed, "candidates_and_frequency_md5",
                    HexFormat.of().formatHex(MessageDigest.getInstance("MD5").digest(bytes.array())), "accepted_candidates", accepted));
            }
            records.add(Map.of("name", name, "placement", config, "samples", samples));
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("STRUCTURE_REFERENCE=" + call(gson, "toJson", Map.of("grid_min", -32, "grid_max", 32, "samples", records)));
    }
}
