import java.lang.reflect.*;
import java.util.*;

/** Native game-protocol codec, including packet IDs and default minecart data. */
public class EntityPacketReference extends TreeReference {
    static Object registries, gameCodec;

    static Object buffer() throws Exception {
        return call(Class.forName("io.netty.buffer.Unpooled"), "buffer");
    }
    static byte[] bytes(Object buffer) throws Exception {
        byte[] result = new byte[(int) call(buffer, "readableBytes")];
        call(buffer, "getBytes", (int) call(buffer, "readerIndex"), result);
        return result;
    }
    static Map<String, Object> wire(Object packet) throws Exception {
        Object buffer = buffer();
        try {
            call(gameCodec, "encode", buffer, packet);
            String hex = HexFormat.of().formatHex(bytes(buffer));
            int id = (int) call(type("network.VarInt"), "read", call(buffer, "duplicate"));
            Object decoded = call(gameCodec, "decode", call(buffer, "duplicate"));
            if (!call(packet, "type").equals(call(decoded, "type")))
                throw new IllegalStateException("native packet round trip changed type");
            return Map.of("packet_id", id, "wire_hex", hex);
        } finally {
            call(buffer, "release");
        }
    }
    static String valueBytes(Object serializer, Object value) throws Exception {
        Object raw = buffer();
        try {
            Object buffer = make("network.RegistryFriendlyByteBuf", raw, registries);
            call(call(serializer, "codec"), "encode", buffer, value);
            return HexFormat.of().formatHex(bytes(raw));
        } finally {
            call(raw, "release");
        }
    }
    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        registries = call(type("core.RegistryAccess"), "fromRegistryOfRegistries",
            field("core.registries.BuiltInRegistries", "REGISTRY"));
        Object protocol = call(field("network.protocol.game.GameProtocols", "CLIENTBOUND_TEMPLATE"),
            "bind", call(type("network.RegistryFriendlyByteBuf"), "decorator", registries));
        gameCodec = call(protocol, "codec");
        Map<String, Integer> entityTypes = new LinkedHashMap<>();
        for (String name : List.of("CHEST_MINECART", "ITEM", "COW", "ZOMBIE")) {
            entityTypes.put(name.toLowerCase(Locale.ROOT), (int) call(field("core.registries.BuiltInRegistries", "ENTITY_TYPE"),
                "getId", field("world.entity.EntityType", name)));
        }
        Object level = NativeEntityLevel.create();
        Object minecart = make("world.entity.vehicle.minecart.MinecartChest",
            field("world.entity.EntityType", "CHEST_MINECART"), level);
        Object defaults = call(call(minecart, "getEntityData"), "getNonDefaultValues");
        if (defaults != null && !((List<?>) defaults).isEmpty())
            throw new IllegalStateException("minecart needs initial metadata: " + defaults);

        List<Object> samples = new ArrayList<>();
        Constructor<?> spawn = type("network.protocol.game.ClientboundAddEntityPacket").getConstructor(
            int.class, UUID.class, double.class, double.class, double.class, float.class, float.class,
            type("world.entity.EntityType"), int.class, type("world.phys.Vec3"), double.class);
        UUID uuid = UUID.fromString("00112233-4455-4677-8899-aabbccddeeff");
        call(minecart, "setId", 393);
        call(minecart, "setUUID", uuid);
        double[] pairedPosition = {-16.5, 32.5, 1.5};
        minecart.getClass().getMethod("setPos", double.class, double.class, double.class)
            .invoke(minecart, pairedPosition[0], pairedPosition[1], pairedPosition[2]);
        Object synchronizer = Proxy.newProxyInstance(EntityPacketReference.class.getClassLoader(),
            new Class<?>[]{type("server.level.ServerEntity$Synchronizer")}, (p, m, a) -> {
                throw new UnsupportedOperationException(m.toString());
            });
        Object serverEntity = make("server.level.ServerEntity", level, minecart, 3, true, synchronizer);
        Map<String, Object> paired = new LinkedHashMap<>(wire(call(minecart, "getAddEntityPacket", serverEntity)));
        paired.put("kind", "spawn"); paired.put("source", "native_minecart_pairing");
        paired.put("id", 393); paired.put("uuid", uuid.toString());
        paired.put("entity_type", entityTypes.get("chest_minecart")); paired.put("position", pairedPosition);
        samples.add(paired);
        for (int id : new int[]{1, 127, 128, 393, Integer.MAX_VALUE}) {
            for (double[] pos : new double[][]{{0.5,-63.5,0.5},{-16.5,319.5,29999983.5},{1.25,42.0,-0.0}}) {
                Object packet = spawn.newInstance(id, uuid, pos[0], pos[1], pos[2], 0.0f, 0.0f,
                    field("world.entity.EntityType", "CHEST_MINECART"), 0, field("world.phys.Vec3", "ZERO"), 0.0);
                Map<String, Object> sample = new LinkedHashMap<>(wire(packet));
                sample.put("kind", "spawn"); sample.put("id", id); sample.put("uuid", uuid.toString());
                sample.put("entity_type", entityTypes.get("chest_minecart")); sample.put("position", pos);
                samples.add(sample);
            }
        }
        for (int[] ids : new int[][]{{},{1},{127,128,393},{1,16384,Integer.MAX_VALUE}}) {
            Object packet = type("network.protocol.game.ClientboundRemoveEntitiesPacket")
                .getConstructor(int[].class).newInstance((Object) ids);
            Map<String, Object> sample = new LinkedHashMap<>(wire(packet));
            sample.put("kind", "remove"); sample.put("ids", ids); samples.add(sample);
        }
        call(minecart, "setInvisible", true);
        call(minecart, "setAirSupply", 260);
        call(minecart, "setNoGravity", true);
        minecart.getClass().getMethod("setDamage", float.class).invoke(minecart, 1.25f);
        List<?> data = (List<?>) call(call(minecart, "getEntityData"), "getNonDefaultValues");
        List<Object> entries = new ArrayList<>();
        for (Object value : data) {
            Object serializer = call(value, "serializer");
            entries.add(Map.of("index", call(value, "id"), "type_id", call(type("network.syncher.EntityDataSerializers"), "getSerializedId", serializer),
                "value_hex", valueBytes(serializer, call(value, "value"))));
        }
        Map<String, Object> metadata = new LinkedHashMap<>(wire(make("network.protocol.game.ClientboundSetEntityDataPacket", 393, data)));
        metadata.put("kind", "metadata"); metadata.put("id", 393); metadata.put("entries", entries); samples.add(metadata);
        for (float[] rotation : new float[][]{{0,0},{-12.5f,90.0f},{360.0f,-45.5f}}) {
            for (boolean ground : new boolean[]{false, true}) {
                double[] pos = {-16.5, 32.5, 29999983.5};
                Object vector = type("world.phys.Vec3").getConstructor(double.class, double.class, double.class)
                    .newInstance(pos[0], pos[1], pos[2]);
                Object change = type("world.entity.PositionMoveRotation")
                    .getConstructor(type("world.phys.Vec3"), type("world.phys.Vec3"), float.class, float.class)
                    .newInstance(vector, field("world.phys.Vec3", "ZERO"), rotation[0], rotation[1]);
                Map<String, Object> sample = new LinkedHashMap<>(wire(make("network.protocol.game.ClientboundTeleportEntityPacket",
                    393, change, Set.of(), ground)));
                sample.put("kind", "teleport"); sample.put("id", 393); sample.put("position", pos);
                sample.put("yaw", rotation[0]); sample.put("pitch", rotation[1]); sample.put("on_ground", ground);
                samples.add(sample);
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("ENTITY_PACKET_REFERENCE=" + call(gson, "toJson", Map.of("samples", samples,
            "entity_types", entityTypes, "minecart_default_metadata_empty", true)));
    }
}
