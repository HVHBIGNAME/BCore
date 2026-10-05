// Reflection permits javac 21; run against the pinned 26.1 JAR on Java 25.
// Native LevelLightEngine storage, DataLayer and packet constructor/writer only.
import java.lang.reflect.*;
import java.util.*;

public final class ChunkLightPacketReference extends TreeReference {
    static Map<String, Object> record(Object... fields) {
        Map<String, Object> out = new LinkedHashMap<>();
        for (int i = 0; i < fields.length; i += 2) out.put((String) fields[i], fields[i + 1]);
        return out;
    }

    static Object engine(boolean sky) throws Exception {
        Object world = Proxy.newProxyInstance(ChunkLightPacketReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.BlockGetter")}, (proxy, method, args) -> {
                return switch (method.getName()) {
                    case "getMinY" -> -64;
                    case "getHeight" -> 384;
                    default -> {
                        if (method.isDefault()) yield InvocationHandler.invokeDefault(proxy, method, args);
                        throw new UnsupportedOperationException("packet probe must not read blocks: " + method);
                    }
                };
            });
        Object getter = Proxy.newProxyInstance(ChunkLightPacketReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.chunk.LightChunkGetter")}, (proxy, method, args) -> {
                if (method.getName().equals("getLevel")) return world;
                throw new UnsupportedOperationException("packet probe must not generate chunks: " + method);
            });
        return make("world.level.lighting.LevelLightEngine", getter, true, sky);
    }

    static Object layer(String kind, int salt) throws Exception {
        if (kind.equals("null")) return null;
        if (kind.equals("lazy_zero")) return make("world.level.chunk.DataLayer", 0);
        if (kind.equals("lazy_full")) return make("world.level.chunk.DataLayer", 15);
        byte[] bytes = new byte[2048];
        if (kind.equals("pattern")) {
            for (int i = 0; i < bytes.length; i++) bytes[i] = (byte) (i * 37 + (i >>> 4) + salt);
        } else if (!kind.equals("materialized_zero")) {
            throw new IllegalArgumentException(kind);
        }
        return make("world.level.chunk.DataLayer", (Object) bytes);
    }

    static byte[] packetBytes(Object packet) throws Exception {
        Object raw = call(Class.forName("io.netty.buffer.Unpooled"), "buffer");
        try {
            Object buffer = make("network.FriendlyByteBuf", raw);
            call(packet, "write", buffer);
            byte[] result = new byte[(int) call(raw, "readableBytes")];
            call(raw, "getBytes", 0, result);
            return result;
        } finally {
            call(raw, "release");
        }
    }

    static Object capture(String name, String[] skyKinds, String[] blockKinds, boolean hasSky, int salt) throws Exception {
        Object engine = engine(hasSky);
        Object chunk = make("world.level.ChunkPos", -17, 29);
        Object sky = field("world.level.LightLayer", "SKY");
        Object block = field("world.level.LightLayer", "BLOCK");
        Object[][] layers = new Object[26][2];
        for (int i = 0; i < 26; i++) {
            layers[i][0] = layer(skyKinds[i % skyKinds.length], salt + i);
            layers[i][1] = layer(blockKinds[i % blockKinds.length], salt - i);
            Object section = call(type("core.SectionPos"), "of", chunk, -5 + i);
            if (layers[i][0] != null) call(engine, "queueSectionData", sky, section, layers[i][0]);
            if (layers[i][1] != null) call(engine, "queueSectionData", block, section, layers[i][1]);
        }
        // The constructor itself classifies lazy-zero versus materialized-zero
        // and null layers. Nothing in this probe implements that classification.
        Object packet = make("network.protocol.game.ClientboundLightUpdatePacketData", chunk, engine, null, null);
        List<Object> input = new ArrayList<>();
        for (Object[] pair : layers) {
            Object[] bytes = new Object[2];
            boolean[] empty = new boolean[2];
            for (int kind = 0; kind < 2; kind++) {
                if (pair[kind] == null) continue;
                empty[kind] = (boolean) call(pair[kind], "isEmpty");
                bytes[kind] = HexFormat.of().formatHex((byte[]) call(call(pair[kind], "copy"), "getData"));
                if ((boolean) call(pair[kind], "isEmpty") != empty[kind]) {
                    throw new AssertionError("observing the input materialized a native layer");
                }
            }
            input.add(record("sky", bytes[0], "block", bytes[1], "sky_empty", empty[0], "block_empty", empty[1]));
        }
        return record("name", name, "has_sky", hasSky, "min_section_y", -5, "sections", input,
            "sky_mask", ((BitSet) call(packet, "getSkyYMask")).stream().toArray(),
            "block_mask", ((BitSet) call(packet, "getBlockYMask")).stream().toArray(),
            "empty_sky_mask", ((BitSet) call(packet, "getEmptySkyYMask")).stream().toArray(),
            "empty_block_mask", ((BitSet) call(packet, "getEmptyBlockYMask")).stream().toArray(),
            "hex", HexFormat.of().formatHex(packetBytes(packet)));
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        List<Object> cases = new ArrayList<>();
        String[] kinds = {"null", "lazy_zero", "materialized_zero", "lazy_full", "pattern"};
        for (String kind : kinds) {
            cases.add(capture(kind + "_both", new String[]{kind}, new String[]{kind}, true, 17));
            cases.add(capture(kind + "_sky_only", new String[]{kind}, new String[]{"null"}, true, 42));
            cases.add(capture(kind + "_block_only", new String[]{"null"}, new String[]{kind}, false, -31));
        }
        for (int salt : new int[]{0, 1, 17, 255}) {
            cases.add(capture("mixed_" + salt, kinds,
                new String[]{"pattern", "null", "lazy_full", "materialized_zero", "lazy_zero"}, true, salt));
        }
        Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        Object gson = call(call(builder, "serializeNulls"), "create");
        System.out.println("CHUNK_LIGHT_PACKET_REFERENCE=" + call(gson, "toJson", cases));
    }
}
