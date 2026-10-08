import java.util.*;

/** Observe the original overworld clock and moon timeline without gameplay ticks. */
final class GenerationSpawnInputs {
    static Object call(Object target, String method, Object... args) throws Exception {
        return GenerationSpawnProbe.call(target, method, args);
    }
    static Object capture() throws Exception {
        Object level = GenerationSpawnProbe.level;
        Object clock = call(GenerationSpawnProbe.registries, "getOrThrow",
            GenerationSpawnProbe.constant("world.clock.WorldClocks", "OVERWORLD"));
        Object manager = call(level, "clockManager");
        Object attributes = call(level, "environmentAttributes");
        Object phaseAttribute = GenerationSpawnProbe.constant("world.attribute.EnvironmentAttributes", "MOON_PHASE");
        Object pos = GenerationSpawnProbe.make("core.BlockPos", 0, 65, 0);
        Object levelData = call(level, "getLevelData");
        long oldClock = (long) call(level, "getOverworldClockTime");
        long oldGame = (long) call(levelData, "getGameTime");
        List<Object> transitions = new ArrayList<>(), boundaries = new ArrayList<>();
        try {
            call(levelData, "setGameTime", 1234L);
            int last = -1;
            // Read every tick across a complete cycle and both adjacent days.
            // Keep transitions rather than duplicating 240001 identical samples.
            for (long time = -24000; time <= 216000; time++) {
                call(manager, "setTotalTicks", clock, time);
                call(attributes, "invalidateTickCache");
                int phase = (int) call(call(attributes, "getValue", phaseAttribute, pos), "index");
                if (phase != last) {
                    transitions.add(GenerationSpawnProbe.map("time", time, "phase", phase,
                        "brightness", call(level, "getMoonBrightness", pos)));
                    last = phase;
                }
            }
            for (long time : new long[]{Long.MIN_VALUE, -192001, -192000, -1, 0, 1, 23999,
                    24000, 191999, 192000, 192001, Long.MAX_VALUE}) {
                call(manager, "setTotalTicks", clock, time);
                call(attributes, "invalidateTickCache");
                boundaries.add(GenerationSpawnProbe.map("requested", Long.toString(time),
                    "overworld_time", Long.toString((long) call(level, "getOverworldClockTime")),
                    "game_time", Long.toString((long) call(level, "getGameTime")),
                    "phase", call(call(attributes, "getValue", phaseAttribute, pos), "index"),
                    "brightness", call(level, "getMoonBrightness", pos)));
            }
        } finally {
            call(levelData, "setGameTime", oldGame);
            call(manager, "setTotalTicks", clock, oldClock);
            call(attributes, "invalidateTickCache");
        }
        return GenerationSpawnProbe.map("native_entry", "ServerLevel.getMoonBrightness / overworld clock",
            "scanned_ticks", 240001, "transitions", transitions, "boundaries", boundaries);
    }
}
