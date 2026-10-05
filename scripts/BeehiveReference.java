import java.io.*;
import java.lang.reflect.*;
import java.util.*;

/** Native generated-bee persistence, block-state validity, and network update NBT. */
public class BeehiveReference extends TreeReference {
    static final String BEEHIVE = "world.level.block.entity.BeehiveBlockEntity";
    static final String OCCUPANT = BEEHIVE + "$Occupant";
    static Object registries, nbtOps, jsonOps, beehiveType, heightAccessor, border;
    static final SortedMap<Integer, Object> supported = new TreeMap<>();

    static int stateId(Object state) throws Exception {
        return (int) call(type("world.level.block.Block"), "getId", state);
    }

    static int constant(String owner, String name) throws Exception {
        Field value = type(owner).getDeclaredField(name);
        value.setAccessible(true);
        return value.getInt(null);
    }

    static Object jsonNbt(Object tag) throws Exception {
        return call(field("nbt.NbtOps", "INSTANCE"), "convertTo", jsonOps, tag);
    }

    // [tag ID, payload]; compounds are name/tag maps and lists are ordered tag arrays.
    static Object typedNbt(Object tag) throws Exception {
        int id = ((Number) call(tag, "getId")).intValue();
        Object value;
        if (id == 10) {
            Map<String, Object> entries = new TreeMap<>();
            for (Object entry : (Set<?>) call(tag, "entrySet")) {
                Map.Entry<?, ?> pair = (Map.Entry<?, ?>) entry;
                entries.put((String) pair.getKey(), typedNbt(pair.getValue()));
            }
            value = entries;
        } else if (id == 9) {
            List<Object> entries = new ArrayList<>();
            for (Object entry : (List<?>) tag) entries.add(typedNbt(entry));
            value = entries;
        } else {
            value = switch (id) {
                case 1, 2, 3, 4, 5, 6 -> call(tag, "box");
                case 7 -> call(tag, "getAsByteArray");
                case 8 -> call(tag, "value");
                case 11 -> call(tag, "getAsIntArray");
                case 12 -> call(tag, "getAsLongArray");
                default -> throw new IllegalArgumentException("Unexpected NBT tag ID " + id);
            };
        }
        return List.of(id, value);
    }

    static byte[] nbtBytes(Object tag, boolean network) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (DataOutputStream output = new DataOutputStream(bytes)) {
            call(type("nbt.NbtIo"), network ? "writeAnyTag" : "write", tag, output);
        }
        byte[] encoded = bytes.toByteArray();
        try (DataInputStream input = new DataInputStream(new ByteArrayInputStream(encoded))) {
            Object decoded = network
                ? call(type("nbt.NbtIo"), "readAnyTag", input, call(type("nbt.NbtAccounter"), "unlimitedHeap"))
                : call(type("nbt.NbtIo"), "read", input);
            if (!tag.equals(decoded) || input.available() != 0)
                throw new IllegalStateException("Native NbtIo round trip changed NBT");
        }
        return encoded;
    }

    static String networkNbt(Object tag) throws Exception {
        Object raw = call(Class.forName("io.netty.buffer.Unpooled"), "buffer");
        try {
            Object buffer = make("network.FriendlyByteBuf", raw);
            call(buffer, "writeNbt", tag);
            byte[] bytes = new byte[(int) call(raw, "readableBytes")];
            call(raw, "getBytes", (int) call(raw, "readerIndex"), bytes);
            if (!Arrays.equals(bytes, nbtBytes(tag, true)))
                throw new IllegalStateException("FriendlyByteBuf differs from NbtIo.writeAnyTag");
            Object decoded = call(buffer, "readNbt");
            if (!tag.equals(decoded) || (int) call(raw, "readableBytes") != 0)
                throw new IllegalStateException("Native FriendlyByteBuf round trip changed NBT");
            return HexFormat.of().formatHex(bytes);
        } finally {
            call(raw, "release");
        }
    }

    static Map<String, Object> nativeError(InvocationTargetException error) {
        Throwable cause = error.getCause();
        while (cause instanceof InvocationTargetException nested) cause = nested.getCause();
        return Map.of("class", cause.getClass().getName(), "message", Objects.toString(cause.getMessage(), ""));
    }

    static Object checkedSave(Object entity, Map<String, Object> record, String problemsKey) throws Exception {
        Object problems = make("util.ProblemReporter$Collector");
        Object output = call(type("world.level.storage.TagValueOutput"), "createWithContext", problems, registries);
        call(entity, "saveWithFullMetadata", output);
        record.put(problemsKey, call(problems, "getReport"));
        Object tag = call(output, "buildResult");
        if (!tag.equals(call(entity, "saveWithFullMetadata", registries)))
            throw new IllegalStateException("saveWithFullMetadata overloads differ");
        return tag;
    }

    static Map<String, Object> sample(String kind, Object state, Pos position, int... ticks) throws Exception {
        Map<String, Object> record = new LinkedHashMap<>();
        record.put("kind", kind);
        record.put("state", stateId(state));
        record.put("pos", List.of(position.x(), position.y(), position.z()));
        record.put("ticks_in_hive", ticks);
        Object pos = make("core.BlockPos", position.x(), position.y(), position.z());
        record.put("inside_build_height", call(heightAccessor, "isInsideBuildHeight", pos));
        record.put("inside_world_border", call(border, "isWithinBounds", pos));
        String stage = "constructor";
        try {
            Object entity = make(BEEHIVE, pos, state);
            stage = "Occupant.create/storeBee";
            for (int tick : ticks) call(entity, "storeBee", call(type(OCCUPANT), "create", tick));
            record.put("occupant_count", call(entity, "getOccupantCount"));
            record.put("is_full", call(entity, "isFull"));
            record.put("is_empty", call(entity, "isEmpty"));
            stage = "saveWithFullMetadata";
            Object saved = checkedSave(entity, record, "save_problems");
            record.put("nbt", jsonNbt(saved));
            record.put("typed_nbt", typedNbt(saved));
            record.put("nbt_hex", HexFormat.of().formatHex(nbtBytes(saved, false)));
            stage = "getUpdateTag/FriendlyByteBuf.writeNbt";
            Object update = call(entity, "getUpdateTag", registries);
            record.put("update", jsonNbt(update));
            record.put("typed_update", typedNbt(update));
            record.put("update_nbt_hex", networkNbt(update));
            record.put("update_packet_null", call(entity, "getUpdatePacket") == null);
            stage = "loadWithComponents";
            Object loaded = make(BEEHIVE, pos, state);
            Object problems = make("util.ProblemReporter$Collector");
            Object input = call(type("world.level.storage.TagValueInput"), "create", problems, registries, saved);
            call(loaded, "loadWithComponents", input);
            record.put("load_problems", call(problems, "getReport"));
            Object resaved = checkedSave(loaded, record, "resave_problems");
            record.put("load_round_trip_equal", saved.equals(resaved));
            if (!saved.equals(resaved)) record.put("reloaded_nbt", typedNbt(resaved));
        } catch (InvocationTargetException error) {
            record.put("error_stage", stage);
            record.put("native_error", nativeError(error));
        }
        return record;
    }

    static void captureStates(Map<String, Object> root) throws Exception {
        Object blockRegistry = field("core.registries.BuiltInRegistries", "BLOCK");
        Object facing = field("world.level.block.BeehiveBlock", "FACING");
        Object honey = field("world.level.block.BeehiveBlock", "HONEY_LEVEL");
        List<Object> states = new ArrayList<>();
        List<int[]> ranges = new ArrayList<>();
        Map<String, Integer> defaults = new TreeMap<>();
        Object previousBlock = null;
        int[] range = null;
        int registeredCount = 0;
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            registeredCount++;
            if (!(boolean) call(beehiveType, "isValid", state)) continue;
            int id = stateId(state);
            supported.put(id, state);
            Object block = call(state, "getBlock");
            String key = call(blockRegistry, "getKey", block).toString();
            defaults.put(key, stateId(call(block, "defaultBlockState")));
            states.add(Map.of("state", id, "block", key,
                "facing", call(call(state, "getValue", facing), "getSerializedName"),
                "honey_level", call(state, "getValue", honey)));
            if (range == null || range[1] != id || previousBlock != block) {
                range = new int[]{id, id + 1};
                ranges.add(range);
            } else range[1] = id + 1;
            previousBlock = block;
            Object factory = call(block, "newBlockEntity", make("core.BlockPos", 0, 65, 0), state);
            if (call(factory, "getType") != beehiveType || !(boolean) call(factory, "isValidBlockState", state))
                throw new IllegalStateException("Block factory disagrees with registered beehive type at " + id);
        }
        if (supported.isEmpty()) throw new IllegalStateException("No registered beehive states");
        root.put("registered_state_count", registeredCount);
        root.put("supported_states", ranges);
        root.put("states", states);
        root.put("default_states", defaults);
    }

    static List<Object> stateValidation() throws Exception {
        Set<Integer> ids = new TreeSet<>(List.of(stateId(state("AIR")), stateId(state("CHEST")),
            supported.firstKey() - 1, supported.lastKey() + 1));
        List<Object> results = new ArrayList<>();
        for (int id : ids) {
            Object state = call(type("world.level.block.Block"), "stateById", id);
            if (stateId(state) != id) throw new IllegalStateException("Unregistered validation state " + id);
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("state", id);
            result.put("block_state", state.toString());
            result.put("type_is_valid", call(beehiveType, "isValid", state));
            try {
                make(BEEHIVE, make("core.BlockPos", 0, 65, 0), state);
                result.put("constructor_accepted", true);
            } catch (InvocationTargetException error) {
                result.put("constructor_accepted", false);
                result.put("native_error", nativeError(error));
            }
            results.add(result);
        }
        return results;
    }

    static Map<String, Object> fluids() throws Exception {
        Object registry = field("core.registries.BuiltInRegistries", "FLUID");
        Map<String, Integer> ids = new TreeMap<>();
        for (Object fluid : (Iterable<?>) registry)
            ids.put(call(registry, "getKey", fluid).toString(), (int) call(registry, "getId", fluid));
        if (ids.size() != (int) call(registry, "size"))
            throw new IllegalStateException("Incomplete native fluid registry enumeration");
        return Map.of("count", ids.size(), "ids", ids);
    }

    static Optional<?> dataResult(Object result, Map<String, Object> record, String operation) throws Exception {
        Optional<?> value = (Optional<?>) call(result, "result");
        record.put(operation + "_accepted", value.isPresent());
        Optional<?> error = (Optional<?>) call(result, "error");
        if (error.isPresent()) record.put(operation + "_error", call(error.get(), "message"));
        return value;
    }

    static void codecRoundTrip(Object codec, Object value, Map<String, Object> record) throws Exception {
        Optional<?> encoded = dataResult(call(codec, "encodeStart", nbtOps, value), record, "encode");
        if (encoded.isEmpty()) return;
        Optional<?> decoded = dataResult(call(codec, "parse", nbtOps, encoded.get()), record, "decode");
        record.put("round_trip_equal", decoded.isPresent() && value.equals(decoded.get()));
    }

    static List<Object> codecCases() throws Exception {
        Object codec = field(OCCUPANT, "CODEC");
        Object listCodec = field(OCCUPANT, "LIST_CODEC");
        Object generated = call(type(OCCUPANT), "create", 0);
        List<Object> results = new ArrayList<>();
        for (String tested : List.of("ticks_in_hive", "min_ticks_in_hive")) {
            for (int value : new int[]{Integer.MIN_VALUE, -1, 0, 1, 598, 599, 600, 2400, Integer.MAX_VALUE}) {
                int ticks = tested.equals("ticks_in_hive") ? value : 0;
                int minimum = tested.equals("min_ticks_in_hive") ? value : (int) call(generated, "minTicksInHive");
                Object occupant = make(OCCUPANT, call(generated, "entityData"), ticks, minimum);
                Map<String, Object> result = new LinkedHashMap<>();
                result.put("kind", "occupant");
                result.put("tested_field", tested);
                result.put("ticks_in_hive", ticks);
                result.put("min_ticks_in_hive", minimum);
                codecRoundTrip(codec, occupant, result);
                results.add(result);
            }
        }
        for (int count : new int[]{0, 1, 2, 3, 4, 64}) {
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("kind", "occupant_list");
            result.put("count", count);
            codecRoundTrip(listCodec, Collections.nCopies(count, generated), result);
            results.add(result);
        }
        Object encoded = call(call(codec, "encodeStart", nbtOps, generated), "getOrThrow");
        for (String key : List.of("ticks_in_hive", "min_ticks_in_hive", "entity_data")) {
            for (String change : List.of("missing", "string")) {
                Object input = call(encoded, "copy");
                if (change.equals("missing")) call(input, "remove", key);
                else call(input, "putString", key, "invalid");
                Map<String, Object> result = new LinkedHashMap<>();
                result.put("kind", "invalid_occupant");
                result.put("field", key);
                result.put("change", change);
                result.put("input", typedNbt(input));
                dataResult(call(codec, "parse", nbtOps, input), result, "decode");
                results.add(result);
            }
        }
        return results;
    }

    static Map<String, Object> constants() throws Exception {
        Map<String, Object> values = new LinkedHashMap<>();
        for (String name : List.of("MAX_OCCUPANTS", "MIN_OCCUPATION_TICKS_NECTARLESS",
                "MIN_OCCUPATION_TICKS_NECTAR", "MIN_TICKS_BEFORE_REENTERING_HIVE"))
            values.put(name.toLowerCase(Locale.ROOT), constant(BEEHIVE, name));
        values.put("max_honey_levels", constant("world.level.block.BeehiveBlock", "MAX_HONEY_LEVELS"));
        values.put("overworld_min_y", call(heightAccessor, "getMinY"));
        values.put("overworld_max_y", call(heightAccessor, "getMaxY"));
        values.put("world_border_min_x", call(border, "getMinX"));
        values.put("world_border_max_x", call(border, "getMaxX"));
        Map<String, Object> priorities = new LinkedHashMap<>();
        for (Object priority : type("world.ticks.TickPriority").getEnumConstants())
            priorities.put(((Enum<?>) priority).name(), call(priority, "getValue"));
        values.put("tick_priorities", priorities);
        return values;
    }

    static List<Object> samples() throws Exception {
        List<Object> samples = new ArrayList<>();
        Pos origin = new Pos(8, 65, 8);
        for (Object state : supported.values()) samples.add(sample("registered_state", state, origin, 0, 1, 598));
        int minY = (int) call(heightAccessor, "getMinY");
        int maxY = (int) call(heightAccessor, "getMaxY");
        int minX = (int) Math.ceil((double) call(border, "getMinX"));
        int maxX = (int) Math.floor((double) call(border, "getMaxX")) - 1;
        for (String block : List.of("BEE_NEST", "BEEHIVE")) {
            Object state = state(block);
            samples.add(sample("count_ticks", state, origin));
            for (int count = 1; count <= 3; count++) {
                for (int tick : new int[]{0, 1, 598, 599, 600}) {
                    int[] ticks = new int[count];
                    Arrays.fill(ticks, tick);
                    samples.add(sample("count_ticks", state, origin, ticks));
                }
            }
            for (Pos pos : List.of(new Pos(-1, minY, 16), new Pos(-16, -1, -17), new Pos(15, maxY, 15),
                    new Pos(maxX, maxY, minX), new Pos(minX, minY, maxX), new Pos(16, 0, -16)))
                samples.add(sample("coordinates", state, pos, 599, 600));
            samples.add(sample("signed_tick_bounds", state, origin, Integer.MIN_VALUE, -1, Integer.MAX_VALUE));
            samples.add(sample("over_capacity_storeBee", state, origin, 0, 1, 598, 599));
            samples.add(sample("occupant_order", state, origin, 599, 0, 598));
            samples.add(sample("occupant_order", state, origin, 598, 599, 0));
        }
        return samples;
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        registries = NativeWorldgenRegistries.load();
        nbtOps = call(registries, "createSerializationContext", field("nbt.NbtOps", "INSTANCE"));
        jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        beehiveType = field("world.level.block.entity.BlockEntityType", "BEEHIVE");
        Object dimensions = call(registries, "lookupOrThrow", field("core.registries.Registries", "DIMENSION_TYPE"));
        Object dimension = call(call(dimensions, "getOrThrow", field("world.level.dimension.BuiltinDimensionTypes", "OVERWORLD")), "value");
        heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", call(dimension, "minY"), call(dimension, "height"));
        border = make("world.level.border.WorldBorder");
        Object typeRegistry = field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE");
        Map<String, Object> root = new LinkedHashMap<>();
        root.put("type_id", call(typeRegistry, "getId", beehiveType));
        root.put("type_key", call(typeRegistry, "getKey", beehiveType).toString());
        root.put("encodings", Map.of("typed_nbt", "[tag_id,payload]; compounds: name/tag maps; lists: ordered tag arrays",
            "nbt_hex", "NbtIo.write; named root with empty UTF name",
            "update_nbt_hex", "FriendlyByteBuf.writeNbt; anonymous NETWORK root; checked against NbtIo.writeAnyTag"));
        captureStates(root);
        root.put("fluid_registry", fluids());
        root.put("constants", constants());
        root.put("samples", samples());
        root.put("state_validation", stateValidation());
        root.put("codec_cases", codecCases());
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("BEEHIVE_REFERENCE=" + call(gson, "toJson", root));
    }
}
