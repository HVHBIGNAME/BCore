import java.lang.reflect.*;
import java.util.*;
import java.util.stream.Stream;

/** Native codec bytes, player bounds and stateless full-cube block-item bindings. */
public final class BlockActionReference {
    private static Object schema(String name, Class<?>[] signature, Object... args) throws Exception {
        Method method = ProtocolSchema.class.getDeclaredMethod(name, signature);
        method.setAccessible(true);
        try { return method.invoke(null, args); }
        catch (InvocationTargetException error) {
            if (error.getCause() instanceof Exception cause) throw cause;
            throw error;
        }
    }

    private static Object call(Object target, String name, Object... args) throws Exception {
        return schema("call", new Class<?>[]{Object.class, String.class, Object[].class}, target, name, args);
    }

    private static Object field(String owner, String name) throws Exception {
        return schema("field", new Class<?>[]{String.class, String.class}, owner, name);
    }

    private static Object make(String owner, Object... args) throws Exception {
        return schema("make", new Class<?>[]{String.class, Object[].class}, owner, args);
    }

    private static Object registries() throws Exception {
        Object root = call(TreeReference.type("core.RegistryAccess"), "fromRegistryOfRegistries",
            field("core.registries.BuiltInRegistries", "REGISTRY"));
        Object loaded = NativeWorldgenRegistries.load();
        List<Object> rows = new ArrayList<>();
        try (Stream<?> stream = (Stream<?>) call(root, "registries")) { rows.addAll(stream.toList()); }
        try (Stream<?> stream = (Stream<?>) call(loaded, "registries")) { rows.addAll(stream.toList()); }
        Object result = call(make("core.RegistryAccess$ImmutableRegistryAccess", rows.stream()), "freeze");
        for (Object pending : (List<?>) call(field("core.registries.BuiltInRegistries", "DATA_COMPONENT_INITIALIZERS"), "build", result)) {
            call(pending, "apply");
        }
        return result;
    }

    private static Map<String, Object> encode(String name, Object packet, Object registries,
            Map<?, ?> ids, Map<String, Object> input) throws Exception {
        Object codec = field(packet.getClass().getName().replace("net.minecraft.", ""), "STREAM_CODEC");
        Object raw = call(Class.forName("io.netty.buffer.Unpooled"), "buffer");
        byte[] bytes;
        try {
            Object buffer = make("network.RegistryFriendlyByteBuf", raw, registries);
            call(codec, "encode", buffer, packet);
            bytes = new byte[(int) call(raw, "readableBytes")];
            call(raw, "readBytes", bytes);
        } finally { call(raw, "release"); }
        Object incoming = call(Class.forName("io.netty.buffer.Unpooled"), "wrappedBuffer", bytes);
        Object decoded;
        try {
            decoded = call(codec, "decode", make("network.RegistryFriendlyByteBuf", incoming, registries));
            if ((int) call(incoming, "readableBytes") != 0) throw new IllegalStateException("trailing native payload");
        } finally { call(incoming, "release"); }
        String packetName = call(call(decoded, "type"), "id").toString();
        Map<String, Object> row = new TreeMap<>();
        row.put("name", name);
        row.put("packet", packetName);
        row.put("protocol_id", ((Map<?, ?>) ids.get(packetName)).get("protocol_id"));
        row.put("payload_hex", HexFormat.of().formatHex(bytes));
        row.put("input", input);
        row.put("native_decode_consumed_all", true);
        return row;
    }

    private static List<Map<String, Object>> blocks() throws Exception {
        Object items = field("core.registries.BuiltInRegistries", "ITEM");
        Object registry = field("core.registries.BuiltInRegistries", "BLOCK");
        Object empty = field("world.level.EmptyBlockGetter", "INSTANCE");
        Object origin = make("core.BlockPos", 0, 0, 0);
        List<Map<String, Object>> result = new ArrayList<>();
        for (Object item : (Iterable<?>) items) {
            if (!item.getClass().getName().equals("net.minecraft.world.item.BlockItem")) continue;
            Object block = call(item, "getBlock");
            String type = block.getClass().getName();
            if (!Set.of("net.minecraft.world.level.block.Block", "net.minecraft.world.level.block.DropExperienceBlock").contains(type)) continue;
            if (!((Collection<?>) call(call(block, "getStateDefinition"), "getProperties")).isEmpty()) continue;
            Object state = call(block, "defaultBlockState");
            if ((boolean) call(state, "hasBlockEntity") || !(boolean) call(call(state, "getFluidState"), "isEmpty")) continue;
            Object shape = call(state, "getCollisionShape", empty, origin);
            if (!(boolean) call(TreeReference.type("world.level.block.Block"), "isShapeFullBlock", shape)) continue;
            Object stack = make("world.item.ItemStack", item, 1);
            result.add(Map.of("item_id", call(items, "getId", item), "item", call(items, "getKey", item).toString(),
                "block", call(registry, "getKey", block).toString(), "state_id", call(TreeReference.type("world.level.block.Block"), "getId", state),
                "block_class", type, "max_stack_size", call(stack, "getMaxStackSize"),
                "stateless", true, "full_collision_cube", true, "has_block_entity", false));
        }
        return result;
    }

    private static Map<String, Object> playerBounds() throws Exception {
        Object attribute = call(field("world.entity.ai.attributes.Attributes", "BLOCK_INTERACTION_RANGE"), "value");
        Object modifier = field("server.level.ServerPlayer", "CREATIVE_BLOCK_INTERACTION_RANGE_MODIFIER");
        Object dimensions = call(field("world.entity.EntityTypes", "PLAYER"), "getDimensions");
        return Map.of("block_range_default", call(attribute, "getDefaultValue"),
            "creative_block_range_addition", call(modifier, "amount"),
            "width", ((Number) call(dimensions, "width")).doubleValue(),
            "height", ((Number) call(dimensions, "height")).doubleValue(),
            "eye_height", ((Number) call(dimensions, "eyeHeight")).doubleValue());
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        call(TreeReference.type("SharedConstants"), "tryDetectVersion");
        call(TreeReference.type("server.Bootstrap"), "bootStrap");
        Object registries = registries();
        Map<String, Object> serverbound = (Map<String, Object>) schema("packetIds", new Class<?>[]{String.class, String.class}, "play", "serverbound");
        Map<String, Object> clientbound = (Map<String, Object>) schema("packetIds", new Class<?>[]{String.class, String.class}, "play", "clientbound");
        List<Map<String, Object>> cases = new ArrayList<>();
        List<Map<String, Object>> actions = new ArrayList<>();
        Object[] values = TreeReference.type("network.protocol.game.ServerboundPlayerActionPacket$Action").getEnumConstants();
        for (Object action : values) {
            Enum<?> value = (Enum<?>) action;
            actions.add(Map.of("ordinal", value.ordinal(), "name", value.name()));
            Object pos = make("core.BlockPos", -17, 319, 32);
            Object direction = field("core.Direction", "UP");
            cases.add(encode("action-" + value.name(), make("network.protocol.game.ServerboundPlayerActionPacket", action, pos, direction, 257),
                registries, serverbound, Map.of("action", value.ordinal(), "pos", List.of(-17, 319, 32), "face", 1, "sequence", 257)));
        }
        for (Object direction : TreeReference.type("core.Direction").getEnumConstants()) {
            Enum<?> value = (Enum<?>) direction;
            Object pos = make("core.BlockPos", -17, -64, 32);
            Object location = make("world.phys.Vec3", -16.75, -63.5, 33.0);
            Object hit = make("world.phys.BlockHitResult", location, direction, pos, false, false);
            cases.add(encode("use-" + value.name(), make("network.protocol.game.ServerboundUseItemOnPacket", field("world.InteractionHand", "MAIN_HAND"), hit, 128),
                registries, serverbound, Map.of("hand", 0, "pos", List.of(-17, -64, 32), "face", value.ordinal(),
                    "offsets", List.of(0.25, 0.5, 1.0), "inside", false, "world_border", false, "sequence", 128)));
        }
        for (int slot : new int[]{0, 8}) {
            cases.add(encode("carried-" + slot, make("network.protocol.game.ServerboundSetCarriedItemPacket", slot),
                registries, serverbound, Map.of("slot", slot)));
        }
        for (String name : List.of("STONE", "DIRT", "DIAMOND_BLOCK")) {
            Object item = field("world.item.Items", name);
            Object stack = make("world.item.ItemStack", item, 64);
            Object itemRegistry = field("core.registries.BuiltInRegistries", "ITEM");
            Map<String, Object> content = Map.of("slot", 36, "count", 64, "item_id", call(itemRegistry, "getId", item), "components_added", 0, "components_removed", 0);
            cases.add(encode("creative-" + name, make("network.protocol.game.ServerboundSetCreativeModeSlotPacket", 36, stack), registries, serverbound, content));
            if (!name.equals("DIRT")) {
                cases.add(encode("inventory-" + name, make("network.protocol.game.ClientboundSetPlayerInventoryPacket", 0, stack),
                    registries, clientbound, Map.of("slot", 0, "count", 64, "item_id", call(itemRegistry, "getId", item))));
            }
        }
        Object empty = field("world.item.ItemStack", "EMPTY");
        cases.add(encode("creative-empty", make("network.protocol.game.ServerboundSetCreativeModeSlotPacket", 44, empty),
            registries, serverbound, Map.of("slot", 44, "count", 0)));
        cases.add(encode("inventory-empty", make("network.protocol.game.ClientboundSetPlayerInventoryPacket", 8, empty),
            registries, clientbound, Map.of("slot", 8, "count", 0)));
        cases.add(encode("ack", make("network.protocol.game.ClientboundBlockChangedAckPacket", 257),
            registries, clientbound, Map.of("sequence", 257)));
        Object pos = make("core.BlockPos", -17, -64, 32);
        Object state = call(field("world.level.block.Blocks", "DIAMOND_BLOCK"), "defaultBlockState");
        cases.add(encode("block-update", make("network.protocol.game.ClientboundBlockUpdatePacket", pos, state),
            registries, clientbound, Map.of("pos", List.of(-17, -64, 32), "state_id", call(TreeReference.type("world.level.block.Block"), "getId", state))));
        Map<String, Object> result = new TreeMap<>();
        result.put("schema", "bcore.native-block-actions.v1");
        result.put("actions", actions);
        result.put("cases", cases);
        result.put("standing_player", playerBounds());
        result.put("stateless_full_cube_block_items", blocks());
        result.put("scope", "Native codecs, standing-player bounds and stateless full-cube item bindings; no server gameplay or GUI execution.");
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("MIGRATION_REFERENCE=" + call(gson, "toJson", result));
    }
}
