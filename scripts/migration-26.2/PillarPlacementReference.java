import java.lang.reflect.*;
import java.util.*;

/** Native axis-only placement states and their block-update codec payloads. */
public final class PillarPlacementReference {
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

    private static Object actions(String name, Class<?>[] signature, Object... args) throws Exception {
        Method method = BlockActionReference.class.getDeclaredMethod(name, signature);
        method.setAccessible(true);
        try { return method.invoke(null, args); }
        catch (InvocationTargetException error) {
            if (error.getCause() instanceof Exception cause) throw cause;
            throw error;
        }
    }

    private static Object context(Object hit) throws Exception {
        Class<?> unsafeType = Class.forName("sun.misc.Unsafe");
        Field instance = unsafeType.getDeclaredField("theUnsafe");
        instance.setAccessible(true);
        Object context = unsafeType.getMethod("allocateInstance", Class.class).invoke(instance.get(null),
            TreeReference.type("world.item.context.BlockPlaceContext"));
        Field hitField = TreeReference.type("world.item.context.UseOnContext").getDeclaredField("hitResult");
        hitField.setAccessible(true);
        hitField.set(context, hit);
        return context;
    }

    private static int stateId(Object state) throws Exception {
        return (int) call(TreeReference.type("world.level.block.Block"), "getId", state);
    }

    private static Map<String, Integer> states(Object block, Object axisProperty) throws Exception {
        Map<String, Integer> result = new TreeMap<>();
        Object empty = field("world.level.EmptyBlockGetter", "INSTANCE");
        Object origin = make("core.BlockPos", 0, 0, 0);
        for (Object axis : TreeReference.type("core.Direction$Axis").getEnumConstants()) {
            Object state = call(call(block, "defaultBlockState"), "setValue", axisProperty, axis);
            Object shape = call(state, "getCollisionShape", empty, origin);
            if ((boolean) call(state, "hasBlockEntity") || !(boolean) call(call(state, "getFluidState"), "isEmpty")
                    || !(boolean) call(TreeReference.type("world.level.block.Block"), "isShapeFullBlock", shape)) {
                throw new IllegalStateException("axis-only pillar has unsupported collision/fluid/BE state");
            }
            result.put(call(axis, "getName").toString(), stateId(state));
        }
        return result;
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        call(TreeReference.type("SharedConstants"), "tryDetectVersion");
        call(TreeReference.type("server.Bootstrap"), "bootStrap");
        Object registries = actions("registries", new Class<?>[]{});
        Object itemRegistry = field("core.registries.BuiltInRegistries", "ITEM");
        Object blockRegistry = field("core.registries.BuiltInRegistries", "BLOCK");
        Object property = field("world.level.block.RotatedPillarBlock", "AXIS");
        Map<String, Object> ids = (Map<String, Object>) schema("packetIds",
            new Class<?>[]{String.class, String.class}, "play", "clientbound");
        List<Map<String, Object>> items = new ArrayList<>();
        List<Object> cases = new ArrayList<>();
        for (Object item : (Iterable<?>) itemRegistry) {
            if (!item.getClass().getName().equals("net.minecraft.world.item.BlockItem")) continue;
            Object block = call(item, "getBlock");
            if (!block.getClass().getName().equals("net.minecraft.world.level.block.RotatedPillarBlock")) continue;
            Collection<?> properties = (Collection<?>) call(call(block, "getStateDefinition"), "getProperties");
            if (properties.size() != 1 || !properties.contains(property)) {
                throw new IllegalStateException("unexpected RotatedPillarBlock properties");
            }
            int itemId = (int) call(itemRegistry, "getId", item);
            String itemName = call(itemRegistry, "getKey", item).toString();
            Map<String, Integer> states = states(block, property);
            int maximum = (int) call(make("world.item.ItemStack", item, 1), "getMaxStackSize");
            items.add(Map.of("item_id", itemId, "item", itemName,
                "block", call(blockRegistry, "getKey", block).toString(), "block_class", block.getClass().getName(),
                "states", states, "default_state_id", stateId(call(block, "defaultBlockState")),
                "max_stack_size", maximum, "full_collision_cube", true, "has_block_entity", false,
                "empty_fluid", true));
            for (Object face : TreeReference.type("core.Direction").getEnumConstants()) {
                int ordinal = ((Enum<?>) face).ordinal();
                List<Integer> coordinates = ordinal < 2 ? List.of(-17, -64, 32) : List.of(-1, 319, -9);
                Object pos = make("core.BlockPos", coordinates.get(0), coordinates.get(1), coordinates.get(2));
                Object location = make("world.phys.Vec3", coordinates.get(0) + 0.5,
                    coordinates.get(1) + 0.5, coordinates.get(2) + 0.5);
                Object hit = make("world.phys.BlockHitResult", location, face, pos, false, false);
                Object placement = call(block, "getStateForPlacement", context(hit));
                String axis = call(call(placement, "getValue", property), "getName").toString();
                int state = stateId(placement);
                if (states.get(axis) != state) throw new IllegalStateException("placement state outside native axis inventory");
                Map<String, Object> input = Map.of("item_id", itemId, "item", itemName, "face", ordinal,
                    "face_name", ((Enum<?>) face).name(), "axis", axis, "pos", coordinates, "state_id", state);
                cases.add(actions("encode", new Class<?>[]{String.class, Object.class, Object.class, Map.class, Map.class},
                    itemName + "-" + ((Enum<?>) face).name(),
                    make("network.protocol.game.ClientboundBlockUpdatePacket", pos, placement), registries, ids, input));
            }
        }
        Map<String, Object> result = new TreeMap<>();
        result.put("schema", "bcore.native-pillar-placement.v1");
        result.put("axis_full_cube_block_items", items);
        result.put("cases", cases);
        result.put("context_constructor_executed", false);
        result.put("native_world_callbacks_executed", false);
        result.put("scope", "Native RotatedPillarBlock.getStateForPlacement reads a real hit-result face from an allocated BlockPlaceContext; constructor, useOn, world callbacks and GUI are not executed.");
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("MIGRATION_REFERENCE=" + call(gson, "toJson", result));
    }
}
