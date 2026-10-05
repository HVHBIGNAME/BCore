import java.util.*;

/** Real brushable load/save/update codecs, independent of structure placement. */
public class BrushableRuntimeReference extends StructureRuntimeReference {
    static Object tag() throws Exception { return make("nbt.CompoundTag"); }

    static void putNumber(Object tag, String kind, String key, Object value) throws Exception {
        Class<?> type = switch (kind) {
            case "Byte" -> byte.class;
            case "Short" -> short.class;
            case "Int" -> int.class;
            case "Long" -> long.class;
            case "Float" -> float.class;
            case "Double" -> double.class;
            default -> String.class;
        };
        tag.getClass().getMethod("put" + kind, String.class, type).invoke(tag, key, value);
    }

    static Object loot(long seed) throws Exception {
        Object tag = tag();
        call(tag, "putString", "LootTable", "minecraft:archaeology/trail_ruins_rare");
        call(tag, "putLong", "LootTableSeed", seed);
        return tag;
    }

    static Object item(String name, Integer count) throws Exception {
        Object item = tag();
        call(item, "putString", "id", name);
        if (count != null) call(item, "putInt", "count", count);
        return item;
    }

    static List<Object> loads() throws Exception {
        Map<String, Object> inputs = new LinkedHashMap<>();
        inputs.put("empty", tag());
        for (long seed : new long[]{0, 1, -17, Long.MIN_VALUE, Long.MAX_VALUE})
            inputs.put("loot_" + seed, loot(seed));
        Object seedOnly = tag(); call(seedOnly, "putLong", "LootTableSeed", 123L);
        inputs.put("seed_without_table", seedOnly);
        Object discarded = loot(42);
        call(discarded, "putString", "unrecognized", "must not survive native load/save");
        call(discarded, "put", "item", item("minecraft:emerald", 3));
        inputs.put("loot_discards_item_and_unknown_fields", discarded);
        for (int count : new int[]{-1, 0, 1, 3, 99, 127, 128, Integer.MAX_VALUE}) {
            Object input = tag(); call(input, "put", "item", item("minecraft:emerald", count));
            inputs.put("item_count_" + count, input);
        }
        Object defaultCount = tag(); call(defaultCount, "put", "item", item("minecraft:emerald", null));
        inputs.put("item_default_count", defaultCount);
        Object emptyItem = tag(); call(emptyItem, "put", "item", item("minecraft:air", 1));
        inputs.put("empty_item", emptyItem);
        Object unknownItem = tag(); call(unknownItem, "put", "item", item("minecraft:missing_brushable_probe", 1));
        inputs.put("unknown_item", unknownItem);
        Object shortName = tag(); call(shortName, "put", "item", item("emerald", 1));
        inputs.put("item_short_name", shortName);
        Object emptyComponents = item("minecraft:emerald", 2); call(emptyComponents, "put", "components", tag());
        call(emptyComponents, "putString", "ignored", "not item data");
        Object itemInput = tag(); call(itemInput, "put", "item", emptyComponents);
        inputs.put("item_empty_components_unknown_field", itemInput);
        for (String kind : List.of("Byte", "Short", "Int", "Long", "Float", "Double", "String")) {
            Object number = switch (kind) {
                case "Byte" -> (byte) 2;
                case "Short" -> (short) 2;
                case "Int" -> 2;
                case "Long" -> 4294967298L;
                case "Float" -> 2.75F;
                case "Double" -> -2.75;
                default -> "2";
            };
            Object stack = item("minecraft:emerald", null); putNumber(stack, kind, "count", number);
            Object input = tag(); call(input, "put", "item", stack);
            inputs.put("item_count_type_" + kind, input);
            input = loot(0); putNumber(input, kind, "LootTableSeed", number);
            inputs.put("loot_seed_type_" + kind, input);
            input = loot(0); putNumber(input, kind, "hit_direction", number);
            inputs.put("direction_type_" + kind, input);
        }
        for (int direction : new int[]{-7, -1, 0, 1, 2, 3, 4, 5, 6, 7, Integer.MAX_VALUE}) {
            Object input = loot(123);
            call(input, "putInt", "hit_direction", direction);
            inputs.put("direction_" + direction, input);
        }
        List<Object> result = new ArrayList<>();
        Object position = make("core.BlockPos", -17, 72, 33);
        for (String name : List.of("SUSPICIOUS_GRAVEL", "SUSPICIOUS_SAND")) {
            Object state = state(name), block = call(state, "getBlock");
            for (var entry : inputs.entrySet()) {
                Object load = call(entry.getValue(), "copy");
                Object entity = call(block, "newBlockEntity", position, state);
                Object input = call(type("world.level.storage.TagValueInput"), "create",
                    field("util.ProblemReporter", "DISCARDING"), registries, load);
                call(entity, "loadWithComponents", input);
                result.add(Map.of("name", name + "/" + entry.getKey(), "state", stateId(state),
                    "pos", xyz(position), "load", Map.of("nbt", nbt64(load)),
                    "full", Map.of("nbt", nbt64(call(entity, "saveWithFullMetadata", registries))),
                    "update", Map.of("nbt", nbt64(call(entity, "getUpdateTag", registries)))));
            }
        }
        return result;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries();
        List<String> items = new ArrayList<>();
        for (Object item : (Iterable<?>) registry("ITEM")) items.add(call(registry("ITEM"), "getKey", item).toString());
        Collections.sort(items);
        output("STRUCTURERUNTIMEREFERENCE", Map.of("scope",
            "Native BrushableBlockEntity loadWithComponents, saveWithFullMetadata and getUpdateTag; no gameplay/loot unpacking.",
            "item_names", items, "block_entity_loads", loads()));
        call(resources, "close");
    }
}
