// Real BlockState.updateShape and virtual FallingBlock delay, on the pinned JAR.
import java.lang.reflect.*;
import java.util.*;

public final class FallingShapeReference extends TreeReference {
    static Class<?> owner(Class<?> type, String name, int count) {
        for (Class<?> current = type; current != null; current = current.getSuperclass()) {
            for (Method method : current.getDeclaredMethods()) {
                if (method.getName().equals(name) && method.getParameterCount() == count) return current;
            }
        }
        throw new IllegalArgumentException(type + "." + name);
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        List<Object> states = new ArrayList<>();
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) states.add(state);
        List<Object> blocks = new ArrayList<>(), samples = new ArrayList<>();
        List<int[]> ticks = new ArrayList<>();
        Object reader = Proxy.newProxyInstance(FallingShapeReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.LevelReader")}, (proxy, method, values) -> {
                throw new UnsupportedOperationException("unexpected falling shape world read " + method);
            });
        Object scheduler = Proxy.newProxyInstance(FallingShapeReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.ScheduledTickAccess")}, (proxy, method, values) -> {
                if (!method.getName().equals("scheduleTick") || values.length != 3) {
                    throw new UnsupportedOperationException("unexpected tick call " + method);
                }
                Pos pos = Pos.from(values[0]);
                int block = (int) call(type("world.level.block.Block"), "getId", call(values[1], "defaultBlockState"));
                ticks.add(new int[]{pos.x(), pos.y(), pos.z(), block, (int) values[2], 0});
                return null;
            });
        Object[] directions = type("core.Direction").getEnumConstants();
        Set<Object> seen = Collections.newSetFromMap(new IdentityHashMap<>());
        Object registry = field("core.registries.BuiltInRegistries", "BLOCK");
        for (int id = 0; id < states.size(); id++) {
            Object state = states.get(id), block = call(state, "getBlock");
            if (!owner(block.getClass(), "updateShape", 8).getSimpleName().equals("FallingBlock")) continue;
            int delay = (int) call(block, "getDelayAfterPlace");
            int defaultId = (int) call(type("world.level.block.Block"), "getId", call(block, "defaultBlockState"));
            if (seen.add(block)) {
                blocks.add(Map.of("name", call(registry, "getKey", block).toString(), "first", id,
                    "count", ((Collection<?>) call(call(block, "getStateDefinition"), "getPossibleStates")).size(),
                    "default", defaultId, "delay", delay));
            }
            for (int direction = 0; direction < directions.length; direction++) {
                for (int y : new int[]{-65, -64, 62, 319, 320}) {
                    Object pos = make("core.BlockPos", -1, y, 16);
                    Object next = call(pos, "relative", directions[direction]);
                    for (Object neighbor : new Object[]{state("AIR"), state("STONE"), state("WATER")}) {
                        Object random = make("world.level.levelgen.XoroshiroRandomSource", 42L);
                        Object untouched = make("world.level.levelgen.XoroshiroRandomSource", 42L);
                        ticks.clear();
                        Object updated = call(state, "updateShape", reader, scheduler, pos, directions[direction], next, neighbor, random);
                        if (!call(random, "nextLong").equals(call(untouched, "nextLong"))) {
                            throw new AssertionError("falling update unexpectedly consumed RNG");
                        }
                        samples.add(Map.of("state", id, "pos", new int[]{-1, y, 16}, "direction", direction,
                            "neighbor", call(type("world.level.block.Block"), "getId", neighbor),
                            "result", call(type("world.level.block.Block"), "getId", updated),
                            "ticks", new ArrayList<>(ticks)));
                    }
                }
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("FALLING_SHAPE_REFERENCE=" + call(gson, "toJson", Map.of(
            "blocks", blocks, "samples", samples, "state_count", states.size(), "random_untouched", true)));
    }
}
