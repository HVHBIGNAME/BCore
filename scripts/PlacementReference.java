import java.security.MessageDigest;
import java.util.*;
import java.util.stream.*;

/** Native modifier streams consumed lazily by a stateful, RNG-consuming terminal. */
public class PlacementReference extends BaseFeatureReference {
    @SuppressWarnings("unchecked")
    static Map<String, Object> sample(String name, Object definition, Object top, long seed, Pos origin, Terrain terrain) throws Exception {
        current = new Capture(terrain);
        Object random = random(seed);
        Object world = world();
        Object context = make("world.level.levelgen.placement.PlacementContext", world, generator, Optional.of(top));
        List<?> modifiers = (List<?>) call(definition, "placement");
        Stream<Object> positions = Stream.of(nativePos(origin));
        for (Object modifier : modifiers) positions = positions.flatMap(pos -> {
            try { return (Stream<Object>) call(modifier, "getPositions", context, random, pos); }
            catch (Exception e) { throw new IllegalStateException(name, e); }
        });
        MessageDigest callsHash = MessageDigest.getInstance("MD5");
        List<int[]> prefix = new ArrayList<>();
        int[] calls = {0};
        boolean[] result = {false};
        Object stone = state("STONE"), gold = state("GOLD_BLOCK");
        positions.forEach(pos -> {
            try {
                if (!(boolean) call(world, "ensureCanWrite", pos)) return;
                Pos p = position(pos);
                int choice = (int) call(random, "nextInt", 7);
                for (int i = 0; i < choice; i++) call(random, "nextFloat");
                int[] row = {p.x(), p.y(), p.z(), choice};
                digest(callsHash, row); calls[0]++;
                if (prefix.size() < 32) prefix.add(row);
                call(world, "setBlock", pos, choice % 2 == 0 ? stone : gold, 2);
                result[0] |= choice % 2 == 0;
            } catch (Exception e) { throw new IllegalStateException(name, e); }
        });
        Map<String, Object> data = new LinkedHashMap<>(current.snapshot());
        data.putAll(Map.of("name", name, "seed", seed, "origin", xyz(origin), "terrain", terrain.json(),
            "result", result[0], "next_i64", call(random, "nextLong"), "calls", calls[0], "call_prefix", prefix,
            "calls_md5", HexFormat.of().formatHex(callsHash.digest())));
        return data;
    }

    static Map<String, String> overworldRoots() throws Exception {
        Map<String, String> names = new TreeMap<>();
        for (Object biome : (Set<?>) call(biomeSource, "possibleBiomes")) {
            String biomeName = call(call(biome, "key"), "identifier").toString();
            for (Object step : (List<?>) call(call(call(biome, "value"), "getGenerationSettings"), "features")) {
                for (Object feature : (Iterable<?>) step) {
                    String name = call(call(feature, "key"), "identifier").toString();
                    names.merge(name, biomeName, (a, b) -> a.compareTo(b) < 0 ? a : b);
                }
            }
        }
        return names;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(args[1]);
        List<Object> samples = new ArrayList<>();
        Map<String, String> roots = overworldRoots();
        Map<String, Set<String>> kernelRoots = new TreeMap<>();
        for (var entry : roots.entrySet()) {
            Object feature = call(holder(placedRegistry, entry.getKey()), "value");
            try (Stream<?> children = (Stream<?>) call(feature, "getFeatures")) {
                for (Object child : children.toList()) {
                    Object kernel = call(call(child, "value"), "feature");
                    String type = call(field("core.registries.BuiltInRegistries", "FEATURE"), "getKey", kernel).toString();
                    kernelRoots.computeIfAbsent(type, k -> new TreeSet<>()).add(entry.getKey());
                }
            }
            for (long seed : new long[]{0, 17}) {
                Terrain t = terrain(seed == 0 ? "flat" : "cave"); t.biome = entry.getValue();
                samples.add(sample(entry.getKey(), feature, feature, seed, new Pos(seed == 0 ? 0 : -32, -64, seed == 0 ? 0 : 16), t));
            }
        }
        for (String name : List.of("patch_sugar_cane", "seagrass_normal", "flower_meadow", "ore_diamond", "lake_lava_underground", "pointed_dripstone", "trees_plains")) {
            Object feature = call(holder(placedRegistry, name), "value");
            Terrain t = terrain("ocean"); t.biome = "minecraft:the_void";
            samples.add(sample(name, feature, feature, 1, new Pos(-16, -64, -16), t));
        }
        Map<String, Object> custom = new TreeMap<>();
        custom.put("every_layer", parse("[{\"type\":\"minecraft:count_on_every_layer\",\"count\":{\"type\":\"minecraft:uniform\",\"min_inclusive\":1,\"max_inclusive\":3}},{\"type\":\"minecraft:rarity_filter\",\"chance\":2}]"));
        custom.put("lazy_heightmap", parse("[{\"type\":\"minecraft:count\",\"count\":6},{\"type\":\"minecraft:heightmap\",\"heightmap\":\"WORLD_SURFACE\"}]"));
        custom.put("symmetric_zero", parse("[{\"type\":\"minecraft:count\",\"count\":4},{\"type\":\"minecraft:random_offset\",\"xz_spread\":{\"type\":\"minecraft:trapezoid\",\"min\":-2,\"max\":2,\"plateau\":0},\"y_spread\":{\"type\":\"minecraft:trapezoid\",\"min\":0,\"max\":0,\"plateau\":0}}]"));
        custom.put("asymmetric_trapezoid", parse("[{\"type\":\"minecraft:count\",\"count\":6},{\"type\":\"minecraft:random_offset\",\"xz_spread\":{\"type\":\"minecraft:trapezoid\",\"min\":-3,\"max\":5,\"plateau\":2},\"y_spread\":0}]"));
        custom.put("scan_down", parse("[{\"type\":\"minecraft:environment_scan\",\"direction_of_search\":\"down\",\"max_steps\":2,\"allowed_search_condition\":{\"type\":\"minecraft:matching_block_tag\",\"tag\":\"minecraft:air\"},\"target_condition\":{\"type\":\"minecraft:solid\"}}]"));
        custom.put("scan_up_bounds", parse("[{\"type\":\"minecraft:environment_scan\",\"direction_of_search\":\"up\",\"max_steps\":1,\"target_condition\":{\"type\":\"minecraft:solid\"}}]"));
        custom.put("surface_extremes", parse("[{\"type\":\"minecraft:surface_relative_threshold_filter\",\"heightmap\":\"WORLD_SURFACE\"}]"));
        custom.put("surface_exact", parse("[{\"type\":\"minecraft:surface_relative_threshold_filter\",\"heightmap\":\"WORLD_SURFACE\",\"min_inclusive\":0,\"max_inclusive\":0}]"));
        custom.put("fixed", parse("[{\"type\":\"minecraft:fixed_placement\",\"positions\":[[-16,65,0],[-1,66,15],[0,67,0],[-17,68,0]]}]"));
        custom.put("weighted_height", parse("[{\"type\":\"minecraft:count\",\"count\":12},{\"type\":\"minecraft:height_range\",\"height\":{\"type\":\"minecraft:weighted_list\",\"distribution\":[{\"data\":{\"type\":\"minecraft:biased_to_bottom\",\"min_inclusive\":{\"above_bottom\":0},\"max_inclusive\":{\"absolute\":-58},\"inner\":2},\"weight\":1},{\"data\":{\"type\":\"minecraft:very_biased_to_bottom\",\"min_inclusive\":{\"absolute\":313},\"max_inclusive\":{\"below_top\":0},\"inner\":1},\"weight\":2}]}}]"));
        custom.put("empty_height", parse("[{\"type\":\"minecraft:height_range\",\"height\":{\"type\":\"minecraft:uniform\",\"min_inclusive\":{\"above_bottom\":384},\"max_inclusive\":{\"below_top\":0}}}]"));
        Object top = call(holder(placedRegistry, "ore_dirt"), "value");
        Map<String, Object> definitions = new TreeMap<>();
        for (var entry : custom.entrySet()) {
            Object definition = parse((String) call(gson, "toJson", Map.of("feature", "minecraft:grass", "placement", entry.getValue())));
            definitions.put(entry.getKey(), definition);
            Object placed = decode("world.level.levelgen.placement.PlacedFeature", definition);
            for (long seed : new long[]{0, 17}) for (int y : new int[]{-64, 64, 65, 319}) {
                Terrain t = terrain(entry.getKey().equals("every_layer") ? "cave" : "flat");
                samples.add(sample(entry.getKey(), placed, top, seed, new Pos(-16, y, 0), t));
            }
        }
        System.out.println("BASE_FEATURE_REFERENCE=" + call(gson, "toJson", Map.of("samples", samples, "overworld_roots", roots, "custom_placed", definitions, "kernel_roots", kernelRoots)));
    }
}
