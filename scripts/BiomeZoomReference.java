import java.lang.reflect.Proxy;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;

/** Capture the quart coordinates selected by the native BiomeManager. */
public class BiomeZoomReference extends TreeReference {
    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Pos[] selected = new Pos[1];
        Object holder = call(type("core.Holder"), "direct", "selection probe");
        Object source = Proxy.newProxyInstance(BiomeZoomReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.biome.BiomeManager$NoiseBiomeSource")}, (p, m, a) -> {
                if (!m.getName().equals("getNoiseBiome")) throw new UnsupportedOperationException(m.toString());
                selected[0] = new Pos((int) a[0], (int) a[1], (int) a[2]);
                return holder;
            });
        List<Object> samples = new ArrayList<>();
        for (long seed : new long[]{0, 1, -1, 846692123413862008L, Long.MIN_VALUE, Long.MAX_VALUE}) {
            long zoomSeed = (long) call(type("world.level.biome.BiomeManager"), "obfuscateSeed", seed);
            Object manager = make("world.level.biome.BiomeManager", source, zoomSeed);
            for (Pos center : List.of(new Pos(0, 0, 0), new Pos(-32, -64, 16), new Pos(29999984, 304, -29999984))) {
                MessageDigest digest = MessageDigest.getInstance("MD5");
                int radius = 8;
                for (int x = center.x() - radius; x <= center.x() + radius; x++)
                    for (int y = center.y() - radius; y <= center.y() + radius; y++)
                        for (int z = center.z() - radius; z <= center.z() + radius; z++) {
                            call(manager, "getBiome", make("core.BlockPos", x, y, z));
                            Pos q = selected[0];
                            digest.update(ByteBuffer.allocate(12).order(ByteOrder.LITTLE_ENDIAN).putInt(q.x()).putInt(q.y()).putInt(q.z()).array());
                        }
                samples.add(Map.of("seed", seed, "zoom_seed", zoomSeed, "center", List.of(center.x(), center.y(), center.z()),
                    "radius", radius, "quart_coordinates_md5", HexFormat.of().formatHex(digest.digest())));
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("BIOME_ZOOM_REFERENCE=" + call(gson, "toJson", Map.of("samples", samples)));
    }
}
