import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.stream.Stream;

/** Invoke target tree definitions and native provider methods on explicit live terrain. */
public final class TreeSemanticsReference extends ExtraTreeReference {
    private static Object registryOps;
    private static final List<String> TREES = List.of("oak", "oak_leaf_litter", "birch_leaf_litter",
        "fancy_oak_leaf_litter", "dark_oak_leaf_litter", "azalea_tree");
    private static final List<String> SCENARIOS = List.of("flat", "vine_cover", "reject_writes", "soil_no_rule",
        "soil_nested_no_rule", "soil_weighted");

    private static Object decode(String owner, Object value) throws Exception {
        Object json = call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", value));
        return call(call(field(owner, "CODEC"), "parse", registryOps, json), "getOrThrow");
    }

    private static Map<String, Object> emptyProvider() {
        return Map.of("type", "minecraft:rule_based_state_provider", "rules", List.of());
    }

    private static Map<String, Object> simple(String block) {
        return Map.of("type", "minecraft:simple_state_provider", "state", Map.of("Name", "minecraft:" + block));
    }

    private static Map<String, Object> nestedProvider() {
        return Map.of("type", "minecraft:rule_based_state_provider", "rules", List.of(
            Map.of("if_true", Map.of("type", "minecraft:true"), "then", emptyProvider())));
    }

    @SuppressWarnings("unchecked")
    private static Map<String, Object> sample(String kind, long seed, String scenario) throws Exception {
        Pos origin = new Pos(-1, 65, 16);
        Map<String, Object> config = new LinkedHashMap<>((Map<String, Object>)
            ((Map<?, ?>) json("configured_feature", kind)).get("config"));
        switch (scenario) {
            case "soil_no_rule" -> config.put("below_trunk_provider", emptyProvider());
            case "soil_nested_no_rule" -> config.put("below_trunk_provider", nestedProvider());
            case "soil_weighted" -> config.put("below_trunk_provider", Map.of(
                "type", "minecraft:weighted_state_provider", "entries", List.of(
                    Map.of("weight", 1, "data", Map.of("Name", "minecraft:dirt")))));
            default -> { }
        }
        Map<Pos, Object> initial = new HashMap<>();
        if (scenario.equals("vine_cover")) initial.put(new Pos(origin.x(), origin.y() + 2, origin.z()), state("VINE"));
        current = new Capture(new Terrain(64, grass, air, initial, scenario.equals("reject_writes")), holder(biomes, "forest"));
        current.writeLimit = Integer.MAX_VALUE;
        writeSource = null;
        Object raw = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        Object definition = decode("world.level.levelgen.feature.configurations.TreeConfiguration", config);
        Object feature = make("world.level.levelgen.feature.TreeFeature", field("world.level.levelgen.feature.configurations.TreeConfiguration", "CODEC"));
        Object context = make("world.level.levelgen.feature.FeaturePlaceContext", Optional.empty(), extraWorld(), generator,
            random(raw), make("core.BlockPos", origin.x(), origin.y(), origin.z()), definition);
        boolean placed = (boolean) call(feature, "place", context);
        long next = (long) call(raw, "nextLong");
        Map<String, Object> result = snapshot(kind, seed, scenario, origin, List.of(), List.of(
            Map.of("placed", placed, "origin", xyz(origin), "next_i64", next)));
        result.put("config", config);
        result.put("raw_brightness", 12);
        return result;
    }

    private static List<Object> providerSamples() throws Exception {
        List<Object> cases = new ArrayList<>();
        Map<String, Object> definitions = new LinkedHashMap<>();
        definitions.put("empty", emptyProvider());
        definitions.put("nested_match", nestedProvider());
        definitions.put("nested_fallback", Map.of("type", "minecraft:rule_based_state_provider", "rules", List.of(), "fallback", emptyProvider()));
        definitions.put("weighted_one", Map.of("type", "minecraft:weighted_state_provider", "entries", List.of(
            Map.of("weight", 1, "data", Map.of("Name", "minecraft:stone")))));
        definitions.put("unknown_property", Map.of("type", "minecraft:randomized_int_state_provider", "source", simple("stone"),
            "property", "age", "values", Map.of("type", "minecraft:uniform", "min_inclusive", 0, "max_inclusive", 1)));
        definitions.put("boolean_property", Map.of("type", "minecraft:randomized_int_state_provider", "source", simple("oak_leaves"),
            "property", "persistent", "values", Map.of("type", "minecraft:uniform", "min_inclusive", 0, "max_inclusive", 1)));
        Pos pos = new Pos(-3, 77, 19);
        for (var entry : definitions.entrySet()) for (String method : List.of("getOptionalState", "getState")) {
            current = new Capture(new Terrain(pos.y(), state("STONE"), air, Map.of(), false), holder(biomes, "forest"));
            Object provider = decode("world.level.levelgen.feature.stateproviders.BlockStateProvider", entry.getValue());
            Object raw = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", 17L));
            Object state = call(provider, method, extraWorld(), random(raw), make("core.BlockPos", pos.x(), pos.y(), pos.z()));
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("name", entry.getKey());
            row.put("provider", entry.getValue());
            row.put("optional", method.equals("getOptionalState"));
            row.put("pos", xyz(pos));
            row.put("state", state == null ? null : id(state));
            row.put("read_count", current.readCount);
            row.put("draws", current.draws);
            row.put("next_i64", call(raw, "nextLong"));
            cases.add(row);
        }
        return cases;
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 0) throw new IllegalArgumentException("TreeSemanticsReference takes no arguments");
        bootstrap();
        Object builtins = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", field("core.registries.BuiltInRegistries", "REGISTRY"));
        Object all = call(type("core.HolderLookup$Provider"), "create", Stream.concat(
            (Stream<?>) call(builtins, "listRegistries"), (Stream<?>) call(registries, "listRegistries")));
        registryOps = call(type("resources.RegistryOps"), "create", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), all);
        Map<?, ?> version;
        try (InputStream stream = type("SharedConstants").getResourceAsStream("/version.json")) {
            if (stream == null) throw new IllegalStateException("native version.json missing");
            version = (Map<?, ?>) call(gson, "fromJson", new String(stream.readAllBytes(), StandardCharsets.UTF_8), Map.class);
        }
        if (!"26.2".equals(version.get("name")) || ((Number) version.get("protocol_version")).intValue() != 776)
            throw new IllegalStateException("different native tree target");
        List<Object> trees = new ArrayList<>();
        for (String kind : TREES) for (long seed : new long[]{0, 17, 42}) for (String scenario : SCENARIOS)
            trees.add(sample(kind, seed, scenario));
        System.out.println("TREE_SEMANTICS_REFERENCE=" + call(gson, "toJson", Map.of(
            "schema", "bcore.native-tree-semantics.v1", "minecraft", "26.2", "protocol", 776,
            "data_version", 4903, "native_version", version, "samples", trees, "providers", providerSamples())));
    }
}
