// Native bamboo edge callbacks, including live support and supplied-neighbor age.
import java.lang.reflect.*;
import java.util.*;

public final class BambooShapeReference extends TreeReference {
    static int id(Object state) throws Exception {
        return (int) call(type("world.level.block.Block"), "getId", state);
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        NativeWorldgenRegistries.load();
        List<Object> states = new ArrayList<>(), bamboo = new ArrayList<>(), samples = new ArrayList<>();
        List<Integer> supports = new ArrayList<>();
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) states.add(state);
        Object origin = make("core.BlockPos", -1, 72, 16);
        Pos belowPos = Pos.from(call(origin, "below"));
        Object[] below = {state("AIR")};
        List<Object> events = new ArrayList<>();
        Object reader = Proxy.newProxyInstance(BambooShapeReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.LevelReader")}, (proxy, method, values) -> {
                if (!method.getName().equals("getBlockState") || !Pos.from(values[0]).equals(belowPos))
                    throw new UnsupportedOperationException("unexpected bamboo read " + method);
                events.add(List.of("get", belowPos.x(), belowPos.y(), belowPos.z(), id(below[0])));
                return below[0];
            });
        Object scheduler = Proxy.newProxyInstance(BambooShapeReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.ScheduledTickAccess")}, (proxy, method, values) -> {
                if (!method.getName().equals("scheduleTick") || values.length != 3 || !type("world.level.block.Block").isInstance(values[1]))
                    throw new UnsupportedOperationException("unexpected bamboo tick " + method);
                Pos p = Pos.from(values[0]);
                events.add(List.of("tick", p.x(), p.y(), p.z(), id(call(values[1], "defaultBlockState")), values[2]));
                return null;
            });
        for (Object state : states) {
            below[0] = state;
            boolean stalk = (boolean) call(state("BAMBOO"), "canSurvive", reader, origin);
            boolean sapling = (boolean) call(state("BAMBOO_SAPLING"), "canSurvive", reader, origin);
            if (stalk != sapling) throw new AssertionError("bamboo support predicates differ");
            if (stalk) supports.add(id(state));
            if (call(state, "getBlock") == field("world.level.block.Blocks", "BAMBOO") ||
                call(state, "getBlock") == field("world.level.block.Blocks", "BAMBOO_SAPLING")) bamboo.add(state);
            events.clear();
        }
        List<Object> neighbors = new ArrayList<>(bamboo);
        neighbors.add(state("AIR")); neighbors.add(state("STONE"));
        Object[] directions = type("core.Direction").getEnumConstants();
        for (Object state : bamboo) {
            for (String support : List.of("DIRT", "SAND", "RED_SAND", "MOSS_BLOCK", "AIR", "STONE", "WATER", "GRAVEL")) {
                below[0] = state(support);
                for (Object neighbor : neighbors) {
                    for (int direction = 0; direction < directions.length; direction++) {
                        Object random = make("world.level.levelgen.XoroshiroRandomSource", -17L);
                        Object untouched = make("world.level.levelgen.XoroshiroRandomSource", -17L);
                        events.clear();
                        Object result = call(state, "updateShape", reader, scheduler, origin, directions[direction],
                            call(origin, "relative", directions[direction]), neighbor, random);
                        if (!call(random, "nextLong").equals(call(untouched, "nextLong"))) throw new AssertionError("bamboo RNG consumed");
                        samples.add(Map.of("state", id(state), "pos", List.of(-1,72,16), "below", id(below[0]),
                            "direction", direction, "neighbor", id(neighbor), "result", id(result), "events", new ArrayList<>(events)));
                    }
                }
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("BAMBOO_SHAPE_REFERENCE=" + call(gson, "toJson", Map.of(
            "samples", samples, "supports_bamboo", supports, "state_count", states.size(), "random_untouched", true)));
    }
}
