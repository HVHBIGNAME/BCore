// Native StairBlock.updateShape: corner selection, ordered reads and water ticks.
import java.lang.reflect.*;
import java.util.*;

public final class StairShapeReference extends TreeReference {
    static int id(Object state) throws Exception {
        return (int) call(type("world.level.block.Block"), "getId", state);
    }

    static Map<Pos, Object> profile(Object state, Object origin, String name) throws Exception {
        Map<Pos, Object> blocks = new HashMap<>();
        Object facing = call(state, "getValue", field("world.level.block.StairBlock", "FACING"));
        Object half = call(state, "getValue", field("world.level.block.StairBlock", "HALF"));
        Object left = call(facing, "getCounterClockWise"), right = call(facing, "getClockWise");
        Object back = call(facing, "getOpposite");
        Object other = call(state("OAK_STAIRS"), "setValue", field("world.level.block.StairBlock", "HALF"), half);
        if (name.equals("empty")) return blocks;
        boolean front = name.startsWith("outer") || name.startsWith("both") || name.startsWith("front");
        Object neighborFacing = name.contains("right") ? right : name.contains("parallel") ? facing : name.contains("opposite") ? back : left;
        Object neighbor = call(other, "setValue", field("world.level.block.StairBlock", "FACING"), neighborFacing);
        if (name.contains("other_half")) neighbor = call(neighbor, "cycle", field("world.level.block.StairBlock", "HALF"));
        blocks.put(Pos.from(call(origin, "relative", front ? facing : back)), neighbor);
        if (name.endsWith("blocked")) {
            Object side = front ? call(neighborFacing, "getOpposite") : neighborFacing;
            blocks.put(Pos.from(call(origin, "relative", side)), call(other, "setValue", field("world.level.block.StairBlock", "FACING"), facing));
        }
        if (name.startsWith("both")) {
            blocks.put(Pos.from(call(origin, "relative", back)), call(other, "setValue", field("world.level.block.StairBlock", "FACING"), right));
        }
        return blocks;
    }

    static Map<String, Object> observe(Object state, Object direction, int directionId, String name) throws Exception {
        Object origin = make("core.BlockPos", -1, 72, 16);
        Map<Pos, Object> blocks = profile(state, origin, name);
        List<Object> events = new ArrayList<>();
        Object reader = Proxy.newProxyInstance(StairShapeReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.LevelReader")}, (proxy, method, args) -> {
                if (!method.getName().equals("getBlockState")) throw new UnsupportedOperationException("unexpected stair read " + method);
                Pos p = Pos.from(args[0]);
                Object value = blocks.getOrDefault(p, state("AIR"));
                events.add(List.of("get", p.x(), p.y(), p.z(), id(value)));
                return value;
            });
        Object scheduler = Proxy.newProxyInstance(StairShapeReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.ScheduledTickAccess")}, (proxy, method, args) -> {
                if (!method.getName().equals("scheduleTick") || args.length != 3 || !type("world.level.material.Fluid").isInstance(args[1]))
                    throw new UnsupportedOperationException("unexpected stair tick " + method);
                Pos p = Pos.from(args[0]);
                int fluid = (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", args[1]);
                events.add(List.of("tick", p.x(), p.y(), p.z(), fluid, args[2]));
                return null;
            });
        Object random = make("world.level.levelgen.XoroshiroRandomSource", 42L);
        Object untouched = make("world.level.levelgen.XoroshiroRandomSource", 42L);
        // Deliberately differs from the live block. Horizontal stair updates
        // query the world, and must not use this supplied neighbor as a shortcut.
        Object neighbor = state("STONE");
        Object result = call(state, "updateShape", reader, scheduler, origin, direction,
            call(origin, "relative", direction), neighbor, random);
        if (!call(random, "nextLong").equals(call(untouched, "nextLong"))) throw new AssertionError("stair RNG consumed");
        List<Object> input = new ArrayList<>();
        for (var entry : blocks.entrySet()) {
            Pos p = entry.getKey();
            input.add(List.of(p.x(), p.y(), p.z(), id(entry.getValue())));
        }
        input.sort(Comparator.comparing(Object::toString));
        return Map.of("state", id(state), "pos", List.of(-1, 72, 16), "direction", directionId,
            "neighbor", id(neighbor), "profile", name, "blocks", input,
            "result", id(result), "events", events);
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        List<Object> states = new ArrayList<>(), blocks = new ArrayList<>(), samples = new ArrayList<>();
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) states.add(state);
        Object registry = field("core.registries.BuiltInRegistries", "BLOCK");
        Object[] directions = type("core.Direction").getEnumConstants();
        Set<Object> seen = Collections.newSetFromMap(new IdentityHashMap<>());
        for (Object state : states) {
            Object block = call(state, "getBlock");
            if (!type("world.level.block.StairBlock").isInstance(block)) continue;
            String owner = FallingShapeReference.owner(block.getClass(), "updateShape", 8).getSimpleName();
            if (!owner.equals("StairBlock")) throw new UnsupportedOperationException("new stair override " + owner);
            Collection<?> all = (Collection<?>) call(call(block, "getStateDefinition"), "getPossibleStates");
            int first = id(all.iterator().next());
            if (seen.add(block)) blocks.add(Map.of("name", call(registry, "getKey", block).toString(),
                "first", first, "count", all.size(), "default", id(call(block, "defaultBlockState")), "owner", owner));
            boolean exhaustive = block == field("world.level.block.Blocks", "COBBLESTONE_STAIRS");
            if (!exhaustive && id(state) != first && id(state) != first + all.size() - 1) continue;
            List<String> profiles = exhaustive ? List.of("empty", "outer_left", "outer_right", "outer_left_blocked", "outer_right_blocked",
                "inner_left", "inner_right", "inner_left_blocked", "inner_right_blocked", "front_parallel", "front_opposite",
                "rear_parallel", "rear_opposite", "front_other_half", "rear_other_half", "both", "both_blocked")
                : List.of("empty", "outer_left", "inner_right");
            for (String profile : profiles) {
                for (int d = 0; d < directions.length; d++) samples.add(observe(state, directions[d], d, profile));
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("STAIR_SHAPE_REFERENCE=" + call(gson, "toJson", Map.of(
            "blocks", blocks, "samples", samples, "state_count", states.size(), "random_untouched", true)));
    }
}
