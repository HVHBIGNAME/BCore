import java.util.*;
import java.util.jar.*;

/** Real decorated-pot load/full/update codecs, including every trial template payload. */
public class DecoratedPotRuntimeReference extends BrushableRuntimeReference {
    static Map<String, Object> observe(String name, Object state, Object load) throws Exception {
        Object position = make("core.BlockPos", -17, 21, 128);
        Object entity = call(call(state, "getBlock"), "newBlockEntity", position, state);
        Object problems = make("util.ProblemReporter$Collector");
        Object input = call(type("world.level.storage.TagValueInput"), "create", problems, registries, call(load, "copy"));
        call(entity, "loadWithComponents", input);
        return Map.of("name", name, "state", stateId(state), "pos", xyz(position),
            "load", Map.of("nbt", nbt64(load)), "load_has_problems", !(boolean) call(problems, "isEmpty"),
            "full", Map.of("nbt", nbt64(call(entity, "saveWithFullMetadata", registries))),
            "update", Map.of("nbt", nbt64(call(entity, "getUpdateTag", registries))));
    }

    static Object decorations(String... names) throws Exception {
        Object input = tag(), list = make("nbt.ListTag");
        for (String name : names) call(list, "add", call(type("nbt.StringTag"), "valueOf", name));
        call(input, "put", "sherds", list);
        return input;
    }

    static Object potLoot(String table, long seed) throws Exception {
        Object input = decorations("angler_pottery_sherd", "brick", "archer_pottery_sherd", "brick");
        call(input, "putString", "LootTable", table);
        call(input, "putLong", "LootTableSeed", seed);
        return input;
    }

    static List<Object> potLoads(String jarPath) throws Exception {
        List<Object> rows = new ArrayList<>();
        Set<String> names = new TreeSet<>(), seen = new HashSet<>();
        try (JarFile jar = new JarFile(jarPath)) {
            for (JarEntry entry : Collections.list(jar.entries())) {
                String path = entry.getName();
                if (path.startsWith("data/minecraft/structure/trial_chambers/") && path.endsWith(".nbt"))
                    names.add("minecraft:" + path.substring("data/minecraft/structure/".length(), path.length() - 4));
            }
        }
        Object block = field("world.level.block.Blocks", "DECORATED_POT");
        for (String name : names) {
            for (Object palette : (List<?>) member(template(name), "palettes")) {
                for (Object info : (List<?>) call(palette, "blocks")) {
                    Object state = call(info, "state"), load = call(info, "nbt");
                    if (load == null || call(state, "getBlock") != block) continue;
                    if (seen.add(stateId(state) + ":" + nbt64(load)))
                        rows.add(observe("template/" + name, state, load));
                }
            }
        }
        for (Object state : (Iterable<?>) call(call(block, "getStateDefinition"), "getPossibleStates"))
            rows.add(observe("default/state_" + stateId(state), state, tag()));

        Map<String, Object> inputs = new LinkedHashMap<>();
        Object emptyComponents = tag(); call(emptyComponents, "put", "components", tag());
        inputs.put("empty_components", emptyComponents);
        Object unknown = tag(); call(unknown, "putString", "unknown", "discarded");
        call(unknown, "putString", "CustomName", "also discarded");
        inputs.put("unknown_fields", unknown);
        for (int length = 0; length <= 5; length++) {
            String[] sherds = new String[length];
            for (int i = 0; i < length; i++) sherds[i] = i % 2 == 0 ? "angler_pottery_sherd" : "brick";
            inputs.put("sherds_length_" + length, decorations(sherds));
        }
        inputs.put("all_bricks", decorations("brick", "brick", "brick", "brick"));
        inputs.put("arbitrary_registered_items", decorations("air", "emerald", "stone", "brick"));
        inputs.put("ordered_sides", decorations("archer_pottery_sherd", "blade_pottery_sherd", "flow_pottery_sherd", "scrape_pottery_sherd"));
        inputs.put("unknown_sherd", decorations("angler_pottery_sherd", "minecraft:missing_pot_probe", "brick"));
        inputs.put("invalid_sherd_name", decorations("UPPER CASE", "angler_pottery_sherd"));
        Object wrongList = tag(), list = make("nbt.ListTag");
        call(list, "add", call(type("nbt.IntTag"), "valueOf", 7));
        call(wrongList, "put", "sherds", list);
        inputs.put("numeric_sherd_list", wrongList);
        for (String kind : List.of("Byte", "Short", "Int", "Long", "Float", "Double", "String")) {
            Object number = switch (kind) {
                case "Byte" -> (byte) 2;
                case "Short" -> (short) 2;
                case "Int" -> 2;
                case "Long" -> 4294967298L;
                case "Float" -> -2.75F;
                case "Double" -> -2.75;
                default -> "2";
            };
            Object input = potLoot("chests/trial_chambers/corridor", 0);
            putNumber(input, kind, "LootTableSeed", number);
            inputs.put("seed_type_" + kind, input);
            Object stack = item("emerald", null); putNumber(stack, kind, "count", number);
            input = tag(); call(input, "put", "item", stack);
            inputs.put("count_type_" + kind, input);
            input = tag(); putNumber(input, kind, "sherds", number);
            inputs.put("sherds_wrong_type_" + kind, input);
        }
        for (long seed : new long[]{0, 1, -17, Long.MIN_VALUE, Long.MAX_VALUE})
            inputs.put("loot_" + seed, potLoot("minecraft:chests/trial_chambers/corridor", seed));
        for (String table : List.of("", "minecraft:", "unknown:loot", "Bad Name"))
            inputs.put("table_" + table, potLoot(table, 42));
        Object seedOnly = tag(); call(seedOnly, "putLong", "LootTableSeed", 42L);
        inputs.put("seed_without_table", seedOnly);
        Object lootAndItem = potLoot("chests/trial_chambers/corridor", 42);
        call(lootAndItem, "put", "item", item("emerald", 3));
        inputs.put("loot_and_item", lootAndItem);
        for (int count : new int[]{Integer.MIN_VALUE, -1, 0, 1, 64, 99, 100, Integer.MAX_VALUE}) {
            Object input = decorations("angler_pottery_sherd");
            call(input, "put", "item", item("emerald", count));
            inputs.put("item_count_" + count, input);
        }
        for (String name : List.of("air", "minecraft:missing_pot_probe", "emerald", "Bad Name")) {
            Object input = tag(); call(input, "put", "item", item(name, null));
            inputs.put("item_" + name, input);
        }
        Object stack = item("emerald", 3); call(stack, "put", "components", tag());
        call(stack, "putString", "unknown", "discarded");
        Object input = tag(); call(input, "put", "item", stack);
        inputs.put("item_empty_components", input);
        for (var entry : inputs.entrySet())
            rows.add(observe("codec/" + entry.getKey(), call(block, "defaultBlockState"), entry.getValue()));
        return rows;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries();
        output("STRUCTURERUNTIMEREFERENCE", Map.of("scope",
            "Native decorated-pot template/default and boundary loadWithComponents, saveWithFullMetadata and getUpdateTag. Invalid codecs retain native fallback results; no loot unpacking or gameplay ticks.",
            "block_entity_loads", potLoads(args[0])));
        call(resources, "close");
    }
}
