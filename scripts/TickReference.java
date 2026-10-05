import java.util.*;
import java.util.stream.Stream;

/** Native 26.1 chunk tick containers; no level tick execution or copied scheduler. */
public class TickReference extends TreeReference {
    static final String PROTO = "world.ticks.ProtoChunkTicks";
    static final String LEVEL = "world.ticks.LevelChunkTicks";
    static final String SAVED = "world.ticks.SavedTick";
    static final String SCHEDULED = "world.ticks.ScheduledTick";
    static final String PRIORITY = "world.ticks.TickPriority";
    record Target(String kind, int id, String name) {}
    record Query(Object target, Pos pos) {}
    record Action(String op, List<Object> ticks, long time, int container) {}
    static final Map<Object, Target> targets = new LinkedHashMap<>();
    static Object airBlock, stone, leaves, log, waterBlock, emptyFluid, water, flowingWater, lava;
    static Object nbtOps, jsonOps;
    static final Object[] listCodecs = new Object[2];

    static Object register(String kind, String fieldName) throws Exception {
        boolean fluid = kind.equals("Fluid");
        Object value = field(fluid ? "world.level.material.Fluids" : "world.level.block.Blocks", fieldName);
        Object registry = field("core.registries.BuiltInRegistries", fluid ? "FLUID" : "BLOCK");
        int id = fluid ? (int) call(registry, "getId", value)
            : (int) call(type("world.level.block.Block"), "getId", call(value, "defaultBlockState"));
        targets.put(value, new Target(kind, id, call(registry, "getKey", value).toString()));
        return value;
    }

    static Map<String, Integer> target(Object type) {
        Target value = Objects.requireNonNull(targets.get(type), "Unregistered probe target");
        return Map.of(value.kind(), value.id());
    }

    static int container(Object target) { return targets.get(target).kind().equals("Fluid") ? 1 : 0; }
    static String containerName(int index) { return index == 0 ? "blocks" : "fluids"; }

    static Object position(Pos pos) throws Exception { return make("core.BlockPos", pos.x(), pos.y(), pos.z()); }
    static Object priority(int value) throws Exception { return call(type(PRIORITY), "byValue", value); }
    static Object scheduled(Object target, Pos pos, long time, int priority, long order) throws Exception {
        return make(SCHEDULED, target, position(pos), time, priority(priority), order);
    }
    static Object saved(Object target, Pos pos, int delay, int priority) throws Exception {
        return make(SAVED, target, position(pos), delay, priority(priority));
    }
    static Action schedule(Object... ticks) { return new Action("schedule", Arrays.asList(ticks), 0, 0); }
    static Action schedule(List<Object> ticks) { return new Action("schedule", ticks, 0, 0); }
    static Action action(String op, long time) { return new Action(op, List.of(), time, 0); }
    static Action poll(int container) { return new Action("poll", List.of(), 0, container); }

    static Map<String, Object> tickRow(Object tick) throws Exception {
        if (tick == null) return null;
        Pos pos = Pos.from(call(tick, "pos"));
        Map<String, Object> row = new LinkedHashMap<>();
        row.put("block_pos", List.of(pos.x(), pos.y(), pos.z()));
        row.put("target", target(call(tick, "type")));
        row.put("priority", call(call(tick, "priority"), "getValue"));
        if (type(SAVED).isInstance(tick)) row.put("delay", call(tick, "delay"));
        else {
            row.put("trigger_tick", call(tick, "triggerTick"));
            row.put("sub_tick_order", call(tick, "subTickOrder"));
        }
        return row;
    }

    static List<Object> rows(List<?> ticks) throws Exception {
        List<Object> rows = new ArrayList<>();
        for (Object tick : ticks) rows.add(tickRow(tick));
        return rows;
    }

    static List<Object> queryRows(List<Query> queries) {
        List<Object> rows = new ArrayList<>();
        for (Query query : queries) rows.add(Map.of("target", target(query.target()),
            "block_pos", List.of(query.pos().x(), query.pos().y(), query.pos().z())));
        return rows;
    }

    static Object saveData(List<?> ticks, int container) throws Exception {
        Object codec = listCodecs[container];
        Object tag = call(call(codec, "encodeStart", nbtOps, ticks), "getOrThrow");
        Object decoded = call(call(codec, "parse", nbtOps, tag), "getOrThrow");
        if (!ticks.equals(decoded)) throw new IllegalStateException("Native saved-tick NBT codec round trip changed data");
        return call(nbtOps, "convertTo", jsonOps, tag);
    }

    static class Queues {
        String mode;
        Object[] values = new Object[2];
        Queues(String mode, List<?> initial) throws Exception {
            this.mode = mode;
            for (int index = 0; index < 2; index++) {
                if (initial == null) values[index] = make(mode.equals("proto") ? PROTO : LEVEL);
                else {
                    List<Object> ticks = new ArrayList<>();
                    for (Object tick : initial) if (container(call(tick, "type")) == index) ticks.add(tick);
                    values[index] = mode.equals("proto") ? call(type(PROTO), "load", ticks) : make(LEVEL, ticks);
                }
            }
        }
        void reload(String newMode, long time) throws Exception {
            List<Object> ticks = new ArrayList<>();
            for (Object queue : values) ticks.addAll((List<?>) call(queue, "pack", time));
            Queues next = new Queues(newMode, ticks);
            mode = next.mode;
            values = next.values;
        }
        List<Object> drain(int index) throws Exception {
            List<Object> drained = new ArrayList<>();
            Object tick;
            while ((tick = call(values[index], "poll")) != null) drained.add(tickRow(tick));
            return drained;
        }
        Map<String, Object> snapshot(long time, List<Query> queries) throws Exception {
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("mode", mode);
            result.put("pack_time", time);
            for (int index = 0; index < 2; index++) {
                Object queue = values[index];
                List<?> packed = (List<?>) call(queue, "pack", time);
                Map<String, Object> state = new LinkedHashMap<>();
                state.put("count", call(queue, "count"));
                state.put("pack", rows(packed));
                state.put("save_data", saveData(packed, index));
                if (mode.equals("proto")) state.put("scheduled_ticks", rows((List<?>) call(queue, "scheduledTicks")));
                else {
                    state.put("get_all", rows(((Stream<?>) call(queue, "getAll")).toList()));
                    state.put("peek", tickRow(call(queue, "peek")));
                }
                result.put(containerName(index), state);
            }
            List<Object> membership = new ArrayList<>();
            for (Query query : queries)
                membership.add(call(values[container(query.target())], "hasScheduledTick", position(query.pos()), query.target()));
            result.put("has_scheduled", membership);
            return result;
        }
    }

    static Map<String, Object> queueCase(String name, String mode, List<?> initial, long packTime, List<Action> actions) throws Exception {
        Set<Query> uniqueQueries = new LinkedHashSet<>();
        if (initial != null) for (Object tick : initial)
            uniqueQueries.add(new Query(call(tick, "type"), Pos.from(call(tick, "pos"))));
        for (Action action : actions) for (Object tick : action.ticks())
            uniqueQueries.add(new Query(call(tick, "type"), Pos.from(call(tick, "pos"))));
        for (Object target : targets.keySet()) uniqueQueries.add(new Query(target, new Pos(-123456, 71, 654321)));
        List<Query> queries = new ArrayList<>(uniqueQueries);
        Queues queues = new Queues(mode, initial);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("name", name);
        result.put("initial_mode", mode);
        result.put("initial_saved", initial == null ? null : rows(initial));
        result.put("queries", queryRows(queries));
        result.put("initial", queues.snapshot(packTime, queries));
        List<Object> steps = new ArrayList<>();
        for (Action action : actions) {
            Map<String, Object> step = new LinkedHashMap<>();
            step.put("op", action.op());
            switch (action.op()) {
                case "schedule" -> {
                    step.put("ticks", rows(action.ticks()));
                    List<Object> accepted = new ArrayList<>();
                    for (Object tick : action.ticks()) {
                        Object queue = queues.values[container(call(tick, "type"))];
                        int before = (int) call(queue, "count");
                        call(queue, "schedule", tick);
                        accepted.add((int) call(queue, "count") > before);
                    }
                    step.put("accepted", accepted);
                }
                case "poll" -> {
                    step.put("container", containerName(action.container()));
                    step.put("polled", tickRow(call(queues.values[action.container()], "poll")));
                }
                case "unpack" -> {
                    step.put("game_time", action.time());
                    for (Object queue : queues.values) call(queue, "unpack", action.time());
                }
                case "reload", "transfer" -> {
                    step.put("game_time", action.time());
                    queues.reload(action.op().equals("transfer") ? "level" : queues.mode, action.time());
                }
                case "pack_time" -> {
                    packTime = action.time();
                    step.put("game_time", action.time());
                }
                case "drain" -> step.put("drained", Map.of("blocks", queues.drain(0), "fluids", queues.drain(1)));
                default -> throw new IllegalArgumentException("Unknown action " + action.op());
            }
            step.put("after", queues.snapshot(packTime, queries));
            steps.add(step);
        }
        result.put("steps", steps);
        return result;
    }

    static List<Object> queueCases() throws Exception {
        List<Object> cases = new ArrayList<>();
        Pos origin = new Pos(0, 65, 0), negative = new Pos(-1, -64, -16);
        cases.add(queueCase("proto_empty_transfer", "proto", null, 0, List.of(
            action("reload", Long.MAX_VALUE), action("transfer", 50), poll(0),
            action("unpack", -50), action("unpack", 90), action("drain", 0))));
        cases.add(queueCase("level_empty_reload", "level", List.of(), -1, List.of(
            poll(1), action("unpack", 0), action("reload", 123), action("unpack", 456), action("drain", 0))));
        List<Object> identities = List.of(
            scheduled(airBlock, origin, 900, 3, 99), scheduled(emptyFluid, origin, -900, -3, -99),
            scheduled(airBlock, origin, -500, -3, -100), scheduled(emptyFluid, origin, 500, 3, 100),
            scheduled(stone, origin, 0, 0, 0), scheduled(water, origin, 0, 0, 0),
            scheduled(flowingWater, origin, 0, 0, 0), scheduled(leaves, negative, -20, -1, Long.MIN_VALUE),
            scheduled(leaves, negative, Long.MIN_VALUE, -3, Long.MAX_VALUE),
            scheduled(leaves, new Pos(-17, 319, 16), Long.MAX_VALUE, 2, Long.MAX_VALUE));
        cases.add(queueCase("proto_identity_first_wins", "proto", null, -100, List.of(
            schedule(identities), action("pack_time", Long.MIN_VALUE), action("reload", Long.MAX_VALUE),
            action("transfer", 700), poll(0), action("unpack", 1000), action("unpack", 2000), action("drain", 0))));
        cases.add(queueCase("level_identity_first_wins", "level", null, -100, List.of(
            schedule(identities), poll(0), poll(1), schedule(identities),
            action("pack_time", Long.MAX_VALUE), action("drain", 0))));
        List<Object> pending = List.of(
            saved(leaves, origin, -20, 0), saved(leaves, origin, 20, -3),
            saved(log, negative, 0, 2), saved(water, origin, 1, 1),
            saved(water, origin, 100, -2), saved(flowingWater, origin, -1, 0));
        cases.add(queueCase("proto_load_preserves_first_delay", "proto", pending, 900, List.of(
            schedule(scheduled(leaves, origin, 0, -3, -10)), action("reload", -900),
            action("transfer", 0), action("unpack", -7), action("drain", 0))));
        cases.add(queueCase("level_loaded_duplicates_membership", "level", pending, 900, List.of(
            poll(0), poll(1), schedule(scheduled(leaves, origin, -1000, -3, -100)),
            action("unpack", 100), poll(0), schedule(scheduled(leaves, origin, 90, 3, 0)),
            poll(0), action("unpack", 9999), action("drain", 0))));
        cases.add(queueCase("level_pending_and_live_reload", "level", pending, 50, List.of(
            schedule(scheduled(stone, origin, 25, -2, 5), scheduled(lava, negative, 125, 2, 6)),
            action("reload", 50), poll(0), action("unpack", 1000), action("drain", 0))));
        List<Object> priorities = new ArrayList<>(), equal = new ArrayList<>();
        for (int p = 3; p >= -3; p--) {
            priorities.add(scheduled(leaves, new Pos(-16 + p, 65, 0), 50, p, Long.MAX_VALUE));
            priorities.add(scheduled(leaves, new Pos(-16 + p, 65, 1), 50, p, Long.MIN_VALUE));
            priorities.add(scheduled(water, new Pos(p, -65, -17), 50, p, -p));
            equal.add(scheduled(stone, new Pos(p, 320, 0), 99, 0, 7));
            equal.add(scheduled(water, new Pos(p, 320, 0), 99, 0, 7));
        }
        cases.add(queueCase("level_priority_and_suborder", "level", null, 25, List.of(
            schedule(priorities), poll(0), poll(1), action("drain", 0))));
        cases.add(queueCase("level_comparator_ties", "level", null, 99, List.of(
            schedule(equal), poll(0), poll(1), schedule(equal), action("drain", 0))));
        for (int seed = 0; seed < 12; seed++) {
            List<Object> shuffled = new ArrayList<>();
            Random random = new Random(seed);
            for (int index = 0; index < 17; index++) {
                Pos pos = new Pos(-32 + index, (index % 3) - 1, 16 - index);
                long time = seed % 3 == 0 ? 100 : random.nextInt(11) - 5;
                int priority = seed % 3 == 0 ? 0 : random.nextInt(7) - 3;
                long order = seed % 3 == 2 ? random.nextInt(3) - 1 : index;
                shuffled.add(scheduled(index % 2 == 0 ? leaves : water, pos, time, priority, order));
            }
            Collections.shuffle(shuffled, random);
            shuffled.add(shuffled.get(0));
            shuffled.add(shuffled.get(3));
            cases.add(queueCase("level_heap_reload_" + seed, "level", null, 90, List.of(
                schedule(shuffled), poll(0), poll(1), schedule(shuffled), action("reload", 90),
                action("unpack", 1000), action("pack_time", 1003), action("drain", 0))));
        }
        List<Object> extremes = List.of(
            scheduled(stone, new Pos(Integer.MIN_VALUE, Integer.MIN_VALUE, Integer.MAX_VALUE), Long.MIN_VALUE, 0, Long.MAX_VALUE),
            scheduled(leaves, origin, Long.MAX_VALUE, -3, Long.MIN_VALUE),
            scheduled(log, negative, 2147483648L, 3, -1),
            scheduled(water, origin, -2147483649L, -2, 1),
            scheduled(lava, negative, Long.MAX_VALUE - 10, 2, Long.MAX_VALUE));
        cases.add(queueCase("level_time_narrowing_and_wrap", "level", null, Long.MAX_VALUE, List.of(
            schedule(extremes), action("reload", Long.MAX_VALUE), action("unpack", Long.MIN_VALUE),
            action("pack_time", Long.MIN_VALUE + 1), action("drain", 0))));
        return cases;
    }

    static List<Object> preparations() throws Exception {
        List<Object> cases = new ArrayList<>();
        for (long time : new long[]{0, -100, Long.MIN_VALUE, Long.MAX_VALUE}) {
            Queues queues = new Queues("proto", null);
            List<Object> requests = new ArrayList<>();
            List<Query> queries = new ArrayList<>();
            List<Object> accepted = new ArrayList<>();
            int[] delays = {0, 1, -1, 5, 600, Integer.MIN_VALUE, Integer.MAX_VALUE};
            Object[] requestTypes = {airBlock, emptyFluid, leaves, water, flowingWater, waterBlock, lava};
            long order = Long.MAX_VALUE - 3;
            for (int index = 0; index < delays.length; index++) {
                Pos pos = new Pos(-16, index == 0 ? -64 : 319, 15);
                Object target = requestTypes[index];
                for (int delay : new int[]{delays[index], delays[index] ^ 7}) {
                    requests.add(Map.of("block_pos", List.of(pos.x(), pos.y(), pos.z()), "target", target(target), "delay", delay));
                    Object tick = make(SCHEDULED, target, position(pos), time + delay, order++);
                    Object queue = queues.values[container(target)];
                    int before = (int) call(queue, "count");
                    call(queue, "schedule", tick);
                    accepted.add((int) call(queue, "count") > before);
                }
                queries.add(new Query(target, pos));
            }
            cases.add(Map.of("game_time", time, "first_sub_tick_order", Long.MAX_VALUE - 3,
                "requests", requests, "queries", queryRows(queries), "accepted", accepted, "expected", queues.snapshot(time, queries)));
        }
        return cases;
    }

    static List<Object> conversions() throws Exception {
        long[][] inputs = {
            {0, 0, 0, 0}, {1, 0, -1, -1}, {-1, 0, 1, 1},
            {Integer.MAX_VALUE, 0, 42, Long.MIN_VALUE}, {2147483648L, 0, Long.MAX_VALUE, Long.MAX_VALUE},
            {-2147483649L, 0, Long.MIN_VALUE, 0}, {Long.MIN_VALUE, Long.MAX_VALUE, Long.MAX_VALUE, Long.MAX_VALUE},
            {Long.MAX_VALUE, Long.MIN_VALUE, Long.MIN_VALUE, Long.MIN_VALUE},
            {(1L << 40) + 17, 5, -100, -7}, {-(1L << 40) - 17, -5, 100, 7},
            {Integer.MIN_VALUE, 0, Long.MIN_VALUE, -1}, {900, 1000, -1000, 13}
        };
        List<Object> cases = new ArrayList<>();
        for (Object target : List.of(leaves, water)) for (int index = 0; index < inputs.length; index++) {
            long[] input = inputs[index];
            Object tick = scheduled(target, new Pos(-17, -64, 16), input[0], index % 7 - 3, input[3]);
            Object packed = call(tick, "toSavedTick", input[1]);
            Object unpacked = call(packed, "unpack", input[2], input[3]);
            cases.add(Map.of("scheduled", tickRow(tick), "pack_time", input[1], "saved", tickRow(packed),
                "unpack_time", input[2], "sub_tick_order", input[3], "unpacked", tickRow(unpacked),
                "save_data", saveData(List.of(packed), container(target))));
        }
        return cases;
    }

    @SuppressWarnings("unchecked")
    static List<Object> comparisons() throws Exception {
        Comparator<Object> drain = (Comparator<Object>) field(SCHEDULED, "DRAIN_ORDER");
        Comparator<Object> intra = (Comparator<Object>) field(SCHEDULED, "INTRA_TICK_DRAIN_ORDER");
        Object identity = field(SCHEDULED, "UNIQUE_TICK_HASH");
        Pos pos = new Pos(-1, 65, 16);
        List<Object> ticks = List.of(
            scheduled(stone, pos, 10, 0, 5), scheduled(stone, pos, -10, 3, -5),
            scheduled(stone, pos, 10, -3, Long.MAX_VALUE), scheduled(stone, pos, 10, 0, Long.MIN_VALUE),
            scheduled(leaves, pos, 10, 0, 5), scheduled(water, pos, 10, 0, 5),
            scheduled(stone, new Pos(-1, 65, 17), 10, 0, 5),
            scheduled(stone, pos, Long.MIN_VALUE, 3, 5), scheduled(stone, pos, Long.MAX_VALUE, -3, 5));
        List<Object> cases = new ArrayList<>();
        for (Object left : ticks) for (Object right : ticks) cases.add(Map.of(
            "left", tickRow(left), "right", tickRow(right),
            "drain", Integer.signum(drain.compare(left, right)), "intra", Integer.signum(intra.compare(left, right)),
            "unique_equal", call(identity, "equals", left, right), "record_equal", left.equals(right)));
        return cases;
    }

    static List<Object> filters() throws Exception {
        List<Pos> positions = List.of(new Pos(-17, 65, -17), new Pos(-16, -65, -16), new Pos(-1, 320, -1),
            new Pos(-1, Integer.MIN_VALUE, -1), new Pos(-16, Integer.MAX_VALUE, -16), new Pos(0, 0, 0),
            new Pos(15, 319, 15), new Pos(16, -64, 16), new Pos(31, 65, 31), new Pos(32, 65, 32),
            new Pos(Integer.MAX_VALUE, 65, Integer.MIN_VALUE));
        List<Object> cases = new ArrayList<>();
        for (Object target : List.of(leaves, water)) {
            List<Object> ticks = new ArrayList<>();
            int index = 0;
            for (Pos pos : positions) ticks.add(saved(target, pos, index, index++ % 7 - 3));
            ticks.add(saved(target, new Pos(-1, 320, -1), -99, -3));
            for (int[] owner : new int[][]{{-2,-2}, {-1,-1}, {0,0}, {1,1}, {2,2}, {134217727,-134217728}}) {
                Object chunk = make("world.level.ChunkPos", owner[0], owner[1]);
                List<?> filtered = (List<?>) call(type(SAVED), "filterTickListForChunk", ticks, chunk);
                cases.add(Map.of("owner", owner, "input", rows(ticks), "filtered", rows(filtered)));
            }
        }
        return cases;
    }

    static List<Object> priorities() throws Exception {
        List<Object> values = new ArrayList<>();
        Object codec = field(PRIORITY, "CODEC");
        for (int input : new int[]{Integer.MIN_VALUE, -4, -3, -2, -1, 0, 1, 2, 3, 4, Integer.MAX_VALUE}) {
            Object tag = call(type("nbt.IntTag"), "valueOf", input);
            Object decoded = call(call(codec, "parse", nbtOps, tag), "getOrThrow");
            Object encoded = call(call(codec, "encodeStart", nbtOps, decoded), "getOrThrow");
            values.add(Map.of("input", input, "by_value", call(priority(input), "getValue"),
                "codec_decoded", call(decoded, "getValue"), "codec_encoded", call(encoded, "intValue")));
        }
        return values;
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        nbtOps = field("nbt.NbtOps", "INSTANCE");
        jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        airBlock = register("Block", "AIR"); stone = register("Block", "STONE");
        leaves = register("Block", "OAK_LEAVES"); log = register("Block", "OAK_LOG");
        waterBlock = register("Block", "WATER"); emptyFluid = register("Fluid", "EMPTY");
        water = register("Fluid", "WATER"); flowingWater = register("Fluid", "FLOWING_WATER");
        lava = register("Fluid", "LAVA"); register("Fluid", "FLOWING_LAVA");
        for (int index = 0; index < 2; index++) {
            Object registry = field("core.registries.BuiltInRegistries", index == 0 ? "BLOCK" : "FLUID");
            listCodecs[index] = call(call(type(SAVED), "codec", call(registry, "byNameCodec")), "listOf");
        }
        Map<String, Object> root = new LinkedHashMap<>();
        root.put("block_target_ids", "native default block-state IDs (canonical block identities)");
        root.put("fluid_target_ids", "native BuiltInRegistries.FLUID IDs");
        List<Object> types = new ArrayList<>();
        for (var entry : targets.entrySet()) types.add(Map.of("target", target(entry.getKey()), "name", entry.getValue().name()));
        root.put("types", types);
        root.put("priority_cases", priorities());
        root.put("preparations", preparations());
        root.put("conversion_cases", conversions());
        root.put("comparison_cases", comparisons());
        root.put("filter_cases", filters());
        root.put("samples", queueCases());
        Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        Object gson = call(call(builder, "serializeNulls"), "create");
        System.out.println("TICK_REFERENCE=" + call(gson, "toJson", root));
    }
}
