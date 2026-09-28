import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.security.MessageDigest;
import java.util.*;

/** Bitwise double streams from native RNGs, followed by a stream-position check. */
public class RandomReference extends TreeReference {
    public static void main(String[] args) throws Exception {
        List<Object> samples = new ArrayList<>();
        int draws = 2048;
        for (String kind : List.of("legacy", "xoroshiro", "worldgen_xoroshiro")) {
            for (long seed : new long[]{0, 1, -1, 846692123413862008L, Long.MIN_VALUE, Long.MAX_VALUE}) {
                Object random = make("world.level.levelgen." + (kind.equals("legacy") ? "LegacyRandomSource" : "XoroshiroRandomSource"), seed);
                if (kind.equals("worldgen_xoroshiro")) random = make("world.level.levelgen.WorldgenRandom", random);
                Method nextDouble = random.getClass().getMethod("nextDouble");
                ByteBuffer bytes = ByteBuffer.allocate(draws * 8).order(ByteOrder.LITTLE_ENDIAN);
                for (int i = 0; i < draws; i++) bytes.putDouble((double) nextDouble.invoke(random));
                samples.add(Map.of("kind", kind, "seed", seed, "next_i64", call(random, "nextLong"),
                    "doubles_md5", HexFormat.of().formatHex(MessageDigest.getInstance("MD5").digest(bytes.array()))));
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("RANDOM_REFERENCE=" + call(gson, "toJson", Map.of("draws", draws, "samples", samples)));
    }
}
