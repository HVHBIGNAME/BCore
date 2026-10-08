import java.lang.reflect.*;
import java.util.*;
import java.util.function.Consumer;

/** Native saved proto-mob -> LOAD factory -> actual ServerEntity pairing packets. */
final class GenerationSpawnHandoff {
    static boolean observing;
    static Object codec;

    static Object call(Object target, String name, Object... args) throws Exception {
        return GenerationSpawnProbe.call(target, name, args);
    }
    static Class<?> type(String name) throws Exception { return GenerationSpawnProbe.type(name); }
    static Object constant(String owner, String name) throws Exception { return GenerationSpawnProbe.constant(owner, name); }
    static Object make(String owner, Object... args) throws Exception { return GenerationSpawnProbe.make(owner, args); }
    static Map<String, Object> map(Object... args) { return GenerationSpawnProbe.map(args); }

    static Object rawBuffer() throws Exception {
        return call(Class.forName("io.netty.buffer.Unpooled"), "buffer");
    }
    static String hex(Object buffer) throws Exception {
        byte[] bytes = new byte[(int) call(buffer, "readableBytes")];
        call(buffer, "getBytes", (int) call(buffer, "readerIndex"), bytes);
        return HexFormat.of().formatHex(bytes);
    }
    static String valueHex(Object serializer, Object value) throws Exception {
        Object raw = rawBuffer();
        try {
            Object buffer = make("network.RegistryFriendlyByteBuf", raw, GenerationSpawnProbe.registries);
            call(call(serializer, "codec"), "encode", buffer, value);
            return hex(raw);
        } finally { call(raw, "release"); }
    }
    static Map<String, Object> metadataValue(Object data) throws Exception {
        Object serializer = call(data, "serializer");
        return map("index", call(data, "id"), "serializer",
            call(type("network.syncher.EntityDataSerializers"), "getSerializedId", serializer),
            "hex", valueHex(serializer, call(data, "value")));
    }

    static Map<String, Object> packet(Object packet) throws Exception {
        if (codec == null) {
            Object protocol = call(constant("network.protocol.game.GameProtocols", "CLIENTBOUND_TEMPLATE"),
                "bind", call(type("network.RegistryFriendlyByteBuf"), "decorator", GenerationSpawnProbe.registries));
            codec = call(protocol, "codec");
        }
        Object raw = rawBuffer();
        try {
            call(codec, "encode", raw, packet);
            Object duplicate = call(raw, "duplicate");
            int id = (int) call(type("network.VarInt"), "read", duplicate);
            Object decoded = call(codec, "decode", call(raw, "duplicate"));
            if (!call(packet, "type").equals(call(decoded, "type"))) throw new AssertionError("native packet type changed");
            String name = packet.getClass().getSimpleName();
            Map<String, Object> result = map("class", name, "packet_id", id, "wire_hex", hex(raw));
            if (name.equals("ClientboundSetEntityDataPacket")) {
                List<Object> entries = new ArrayList<>();
                for (Object data : (List<?>) call(packet, "packedItems")) entries.add(metadataValue(data));
                result.put("entries", entries);
            }
            if (name.equals("ClientboundUpdateAttributesPacket")) {
                List<Object> attributes = new ArrayList<>();
                Object attributeCodec = constant("network.protocol.game.ClientboundUpdateAttributesPacket$AttributeSnapshot", "STREAM_CODEC");
                for (Object value : (List<?>) call(packet, "getValues")) {
                    Object buffer = rawBuffer();
                    try {
                        call(attributeCodec, "encode", make("network.RegistryFriendlyByteBuf", buffer, GenerationSpawnProbe.registries), value);
                        attributes.add(map("name", GenerationSpawnProbe.holderName(call(value, "attribute")), "hex", hex(buffer)));
                    } finally { call(buffer, "release"); }
                }
                result.put("attributes", attributes);
            }
            return result;
        } finally { call(raw, "release"); }
    }

    static Map<String, Object> observe(Object original, Object tag) throws Exception {
        GenerationSpawnProbe.Case previous = GenerationSpawnProbe.ACTIVE.get();
        observing = true;
        Map<String, Object> input = GenerationSpawnProbe.input("load-entropy", "plains", 0, 0, 0, null);
        input.put("entity_seed", "9000");
        GenerationSpawnProbe.Case independent = new GenerationSpawnProbe.Case(input);
        GenerationSpawnProbe.ACTIVE.set(independent);
        try {
            Object problems = make("util.ProblemReporter$Collector");
            Object data = call(type("world.level.storage.TagValueInput"), "create", problems, GenerationSpawnProbe.registries, tag);
            Object loaded = call(type("world.entity.EntityType"), "loadEntityRecursive", data,
                GenerationSpawnProbe.level, constant("world.entity.EntitySpawnReason", "LOAD"), constant("world.entity.EntityProcessor", "NOP"));
            if (loaded == null || !call(problems, "getReport").equals("")) throw new AssertionError("native mob load failed");
            Map<String, Object> saved = GenerationSpawnProbe.saveEntity(loaded);
            call(loaded, "setId", 913); // Runtime packet identity is an explicit fixture input.
            Object synchronizer = Proxy.newProxyInstance(GenerationSpawnHandoff.class.getClassLoader(),
                new Class<?>[]{type("server.level.ServerEntity$Synchronizer")}, (p, m, a) -> {
                    throw new UnsupportedOperationException("Unexpected synchronizer call " + m);
                });
            Object tracker = make("server.level.ServerEntity", GenerationSpawnProbe.level, loaded, 3, true, synchronizer);
            List<Object> packets = new ArrayList<>();
            Consumer<Object> sink = value -> {
                try { packets.add(packet(value)); }
                catch (Exception failure) { throw new RuntimeException(failure); }
            };
            call(tracker, "sendPairingData", null, sink);
            return map("loaded", saved, "runtime_id", 913, "load_entropy_seed", "9000", "packets", packets,
                "load_rng", independent.entityRngs.get(0).finish());
        } finally {
            GenerationSpawnProbe.ACTIVE.set(previous);
            observing = false;
        }
    }

    static Object describeValue(Object value) throws Exception {
        if (value == null || value instanceof Number || value instanceof Boolean || value instanceof String) return value;
        if (value instanceof Enum<?> e) return e.name();
        if (value instanceof Optional<?> o) return o.isEmpty() ? "empty" : describeValue(o.get());
        if (type("core.Holder").isInstance(value)) return GenerationSpawnProbe.holderName(value);
        if (value instanceof List<?> list && list.isEmpty()) return List.of();
        return value.getClass().getName();
    }

    static Object catalog(List<?> types) throws Exception {
        Map<String, Object> metadata = new TreeMap<>(), defaults = new TreeMap<>(), registries = new TreeMap<>();
        observing = true;
        try {
            for (Object row : types) {
                String name = (String) ((Map<?, ?>) row).get("name");
                if (name.equals("minecraft:strider")) continue;
                GenerationSpawnProbe.Case c = new GenerationSpawnProbe.Case(GenerationSpawnProbe.input("metadata", "plains", 0, 0, 0, null));
                GenerationSpawnProbe.ACTIVE.set(c);
                Object entity = call(GenerationSpawnProbe.entity(name), "create", GenerationSpawnProbe.level, constant("world.entity.EntitySpawnReason", "NATURAL"));
                Map<Integer, String> fields = new TreeMap<>();
                for (Class<?> owner = entity.getClass(); owner != null; owner = owner.getSuperclass()) {
                    for (Field field : owner.getDeclaredFields()) {
                        if (Modifier.isStatic(field.getModifiers()) && field.getType() == type("network.syncher.EntityDataAccessor")) {
                            field.setAccessible(true);
                            Object accessor = field.get(null);
                            fields.put((int) call(accessor, "id"), field.getName());
                        }
                    }
                }
                Object[] items = (Object[]) GenerationSpawnProbe.field(call(entity, "getEntityData"), "itemsById");
                List<Object> entries = new ArrayList<>();
                for (Object item : items) {
                    Object data = call(item, "value");
                    Map<String, Object> entry = metadataValue(data);
                    entry.put("name", fields.get((int) call(data, "id")));
                    entry.put("default_hex", valueHex(call(data, "serializer"), GenerationSpawnProbe.field(item, "initialValue")));
                    entry.put("value", describeValue(call(item, "getValue")));
                    entries.add(entry);
                }
                metadata.put(name, entries);
                Object supplier = call(type("world.entity.ai.attributes.DefaultAttributes"), "getSupplier", GenerationSpawnProbe.entity(name));
                Map<String, Object> attributes = new TreeMap<>();
                for (var attribute : ((Map<?, ?>) GenerationSpawnProbe.field(supplier, "instances")).entrySet()) {
                    attributes.put(GenerationSpawnProbe.holderName(attribute.getKey()), map("base", call(attribute.getValue(), "getBaseValue"),
                        "syncable", call(call(attribute.getKey(), "value"), "isClientSyncable")));
                }
                defaults.put(name, attributes);
            }
            for (String key : List.of("COW_VARIANT", "PIG_VARIANT", "CHICKEN_VARIANT", "WOLF_VARIANT", "FROG_VARIANT",
                    "COW_SOUND_VARIANT", "PIG_SOUND_VARIANT", "CHICKEN_SOUND_VARIANT", "WOLF_SOUND_VARIANT", "ATTRIBUTE", "ITEM")) {
                Object registry = GenerationSpawnProbe.registry(key);
                Map<String, Object> ids = new TreeMap<>();
                for (Object value : (Iterable<?>) registry) ids.put(call(registry, "getKey", value).toString(), call(registry, "getId", value));
                registries.put(key.toLowerCase(Locale.ROOT), ids);
            }
        } finally { GenerationSpawnProbe.ACTIVE.remove(); observing = false; }
        return map("metadata", metadata, "default_attributes", defaults, "registries", registries);
    }
}
