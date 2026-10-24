import java.nio.charset.StandardCharsets;
import java.util.*;

/** Target feature calls using the preserved bounded cave-world observer. */
public final class SpeleothemReference extends CaveFeatureReference {
    @SuppressWarnings("unchecked")
    static Map<String, Object> config(Object document) {
        return (Map<String, Object>) ((Map<?, ?>) document).get("config");
    }

    static Object canonical(Object feature) throws Exception {
        Object ops = call(type("resources.RegistryOps"), "create",
            Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), codecRegistries);
        Object encoded = call(call(field("world.level.levelgen.feature.ConfiguredFeature", "DIRECT_CODEC"),
            "encodeStart", ops, feature), "getOrThrow");
        return call(gson, "fromJson", encoded.toString(), Map.class);
    }

    static Map<String, Object> admission(String name, Object document) {
        Map<String, Object> row = new LinkedHashMap<>();
        row.put("name", name);
        row.put("document", document);
        try {
            row.put("canonical_document", canonical(decode(document)));
            row.put("admitted", true);
        } catch (Exception error) {
            Throwable cause = error;
            while (cause.getCause() != null) cause = cause.getCause();
            row.put("admitted", false);
            row.put("error", cause.getClass().getName() + ": " + cause.getMessage());
        }
        return row;
    }

    static List<Object> admissions(Object pointed, Object cluster) throws Exception {
        List<Object> rows = new ArrayList<>();
        Map<String, Object> defaults = copyDocument(pointed);
        for (String name : List.of("chance_of_taller_generation", "chance_of_directional_spread",
                "chance_of_spread_radius2", "chance_of_spread_radius3")) config(defaults).remove(name);
        rows.add(admission("pointed_defaults", defaults));
        for (Object value : List.of(-0.1, 1.0001, "0.2")) {
            Map<String, Object> document = copyDocument(pointed);
            config(document).put("chance_of_taller_generation", value);
            rows.add(admission("pointed_chance_" + value, document));
        }
        Map<String, Object> missing = copyDocument(pointed);
        config(missing).remove("base_block");
        rows.add(admission("missing_base", missing));
        Map<String, Object> unknown = copyDocument(pointed);
        config(unknown).put("pointed_block", Map.of("Name", "minecraft:missing_speleothem"));
        rows.add(admission("unknown_pointed", unknown));
        Map<String, Object> tag = copyDocument(pointed);
        config(tag).put("replaceable_blocks", "#minecraft:missing_speleothem_blocks");
        rows.add(admission("unknown_tag", tag));
        rows.add(admission("cluster_stock", cluster));
        for (Object value : List.of(10.0, 2.9, true, false, 0, 65)) {
            Map<String, Object> document = copyDocument(cluster);
            config(document).put("height_deviation", value);
            rows.add(admission("cluster_deviation_" + value, document));
        }
        for (String key : List.of("floor_to_ceiling_search_range", "height", "radius")) {
            Map<String, Object> document = copyDocument(cluster);
            config(document).put(key, key.equals("height") ? 129 : key.equals("radius") ? 1 : 0);
            rows.add(admission("cluster_" + key, document));
        }
        Map<String, Object> empty = copyDocument(pointed);
        config(empty).put("replaceable_blocks", List.of());
        rows.add(admission("empty_replaceable_set", empty));
        return rows;
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        bootstrap();
        Map<?, ?> selector = (Map<?, ?>) json("worldgen/configured_feature/pointed_dripstone");
        Object pointed = ((Map<?, ?>) ((List<?>) ((Map<?, ?>) selector.get("config")).get("features")).get(0)).get("feature");
        Map<String, Object> sulfurPointed = copyDocument(pointed);
        Map<String, Object> sulfurCluster = copyDocument(json("worldgen/configured_feature/sulfur_spike_cluster"));
        for (String key : List.of("base_block", "pointed_block", "replaceable_blocks"))
            config(sulfurPointed).put(key, config(sulfurCluster).get(key));
        Map<String, Object> documents = new LinkedHashMap<>();
        documents.put("pointed_dripstone", pointed);
        documents.put("dripstone_cluster", json("worldgen/configured_feature/dripstone_cluster"));
        documents.put("sulfur_spike", sulfurPointed);
        documents.put("sulfur_spike_cluster", sulfurCluster);
        List<Object> samples = new ArrayList<>();
        for (Map.Entry<String, Object> entry : documents.entrySet()) {
            String name = entry.getKey();
            Object document = entry.getValue();
            boolean isPointed = !name.endsWith("cluster");
            Object base = state(name.startsWith("sulfur") ? "SULFUR" : "STONE");
            for (long seed : new long[]{0, 1, 17, 42, 1234})
                samples.add(sample(name, document, seed, "base_block", new Pos(15, isPointed ? 21 : 25, -17),
                    new Capture(20, 30, base, air, 20, false), 1));
            for (String scenario : List.of("water", "flowing_water", "lava", "bedrock", "deepslate",
                    "no_floor", "no_ceiling", "reject_writes", "denied_origin", "advanced_repeat", "bottom", "top")) {
                int floor = scenario.equals("bottom") ? -64 : scenario.equals("top") ? 309 : 20;
                int roof = floor + 10;
                Object rock = scenario.equals("bedrock") ? state("BEDROCK") : scenario.equals("deepslate") ? state("DEEPSLATE") : base;
                Object fill = switch (scenario) {
                    case "water" -> state("WATER");
                    case "flowing_water" -> call(state("WATER"), "setValue", field("world.level.block.LiquidBlock", "LEVEL"), 1);
                    case "lava" -> state("LAVA");
                    default -> air;
                };
                Capture world = new Capture(scenario.equals("no_floor") ? -1000 : floor,
                    scenario.equals("no_ceiling") ? 1000 : roof, rock, fill, roof - 1, scenario.equals("reject_writes"));
                world.denyOrigin = scenario.equals("denied_origin");
                samples.add(sample(name, document, 42, scenario, new Pos(-16, floor + (isPointed ? 1 : 5), 31),
                    world, scenario.contains("repeat") ? 3 : 1));
            }
            for (int y : new int[]{-65, -64, 319, 320})
                samples.add(sample(name, document, 1, "build_boundary", new Pos(-17, y, 16),
                    new Capture(y - 1, y + 1, base, air, y - 1, false), 1));
            if (isPointed) {
                Map<String, Object> properties = copyDocument(document);
                Map<String, Object> block = new LinkedHashMap<>((Map<String, Object>) config(properties).get("pointed_block"));
                block.put("Properties", Map.of("vertical_direction", "down", "thickness", "base", "waterlogged", "true"));
                config(properties).put("pointed_block", block);
                config(properties).put("chance_of_taller_generation", 1.0);
                samples.add(sample(name, properties, 17, "configured_state_properties", new Pos(0, 21, 0),
                    new Capture(20, 30, base, air, 20, false), 1));
            } else {
                for (int gap : new int[]{1, 2, 3, 6}) {
                    Map<String, Object> merge = copyDocument(document);
                    config(merge).put("height", 12);
                    config(merge).put("radius", 2);
                    config(merge).put("density", 2.0);
                    config(merge).put("wetness", 0.0);
                    config(merge).put("chance_of_speleothem_at_max_distance_from_center", 1.0);
                    samples.add(sample(name, merge, 17, "merge_gap_" + gap, new Pos(0, 21, 0),
                        new Capture(20, 21 + gap, base, air, 20, false), 2));
                }
                samples.add(sample(name, document, 17, "reseed_repeat", new Pos(15, 25, -17),
                    new Capture(20, 30, base, air, 20, false), 3));
            }
        }
        List<Object> providers = List.of(
            Map.of("type", "minecraft:constant", "value", 3),
            Map.of("type", "minecraft:uniform", "min_inclusive", 3, "max_inclusive", 3),
            Map.of("type", "minecraft:biased_to_bottom", "min_inclusive", 1, "max_inclusive", 8),
            Map.of("type", "minecraft:clamped_normal", "mean", 4.5, "deviation", 2.5, "min_inclusive", 1, "max_inclusive", 9),
            Map.of("type", "minecraft:clamped", "source", Map.of("type", "minecraft:uniform", "min_inclusive", -2, "max_inclusive", 10), "min_inclusive", 2, "max_inclusive", 6),
            Map.of("type", "minecraft:weighted_list", "distribution", List.of(Map.of("data", 4, "weight", 0), Map.of("data", 2, "weight", 1))));
        for (int index = 0; index < providers.size(); index++) {
            Map<String, Object> document = copyDocument(documents.get("dripstone_cluster"));
            config(document).put("height", providers.get(index));
            config(document).put("radius", 2);
            samples.add(sample("dripstone_cluster", document, 42, "int_provider_" + index, new Pos(0, 25, 0),
                new Capture(20, 30, state("STONE"), air, 20, false), 2));
        }
        Object version;
        try (var stream = SpeleothemReference.class.getResourceAsStream("/version.json")) {
            version = call(gson, "fromJson", new String(Objects.requireNonNull(stream).readAllBytes(), StandardCharsets.UTF_8), Map.class);
        }
        System.out.println("SPELEOTHEM_REFERENCE=" + call(gson, "toJson", Map.of(
            "native_version", version, "configs", documents, "samples", samples,
            "admission", admissions(pointed, documents.get("dripstone_cluster")))));
    }
}
