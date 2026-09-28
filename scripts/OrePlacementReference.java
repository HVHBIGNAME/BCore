import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;

/** Native modifier + OreFeature streams. Biome membership is isolated separately. */
public class OrePlacementReference extends OreReference {
    static Object json(String directory, String name) throws Exception {
        String path = "/data/minecraft/worldgen/" + directory + "/" + name + ".json";
        try (var stream = OrePlacementReference.class.getResourceAsStream(path)) {
            if (stream == null) throw new IllegalArgumentException("missing " + path);
            return call(call(Class.forName("com.google.gson.JsonParser"), "parseString",
                new String(stream.readAllBytes(), StandardCharsets.UTF_8)), "getAsJsonObject");
        }
    }

    static Object generator() throws Exception {
        Object dummy = call(type("world.level.levelgen.NoiseGeneratorSettings"), "dummy");
        Object settings = make("world.level.levelgen.NoiseGeneratorSettings",
            call(type("world.level.levelgen.NoiseSettings"), "create", -64, 384, 1, 2),
            stone, water, call(dummy, "noiseRouter"), call(dummy, "surfaceRule"), List.of(),
            63, true, false, false, false);
        Object biome = call(type("core.Holder"), "direct", "unused by ore placement probe");
        return make("world.level.levelgen.NoiseBasedChunkGenerator", make("world.level.biome.FixedBiomeSource", biome),
            call(type("core.Holder"), "direct", settings));
    }

    static Map<String, Object> probePlacement(Object feature, Object generator, String name, int step, int index,
            long seed, String scenario, int cx, int cz) throws Exception {
        terrain = scenario;
        base = scenario.equals("deepslate") ? deepslate : stone;
        baseFactory = factory(base);
        surface = 64;
        chunks = new HashMap<>();
        visited = new HashSet<>();
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        long decorationSeed = (long) call(random, "setDecorationSeed", seed, cx * 16, cz * 16);
        call(random, "setFeatureSeed", decorationSeed, index, step);
        boolean placed = (boolean) call(feature, "place", world(), generator, random, make("core.BlockPos", cx * 16, -64, cz * 16));
        MessageDigest digest = MessageDigest.getInstance("MD5");
        int count = 0;
        List<Pos> sorted = new ArrayList<>(visited);
        sorted.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        for (Pos pos : sorted) {
            Object state = read(pos);
            if (state == initial(pos)) continue;
            int id = (int) call(type("world.level.block.Block"), "getId", state);
            digest.update(ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN).putInt(pos.x()).putInt(pos.y()).putInt(pos.z()).putInt(id).array());
            count++;
        }
        return Map.of("name", name, "step", step, "index", index, "seed", seed, "terrain", scenario, "chunk", List.of(cx, cz),
            "placed", placed, "changed_blocks", count, "writes_md5", HexFormat.of().formatHex(digest.digest()), "next_i64", call(random, "nextLong"));
    }

    public static void main(String[] args) throws Exception {
        bootstrapOre();
        Object generator = generator();
        Object ops = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        List<Object> samples = new ArrayList<>();
        Map<String, Object> definitions = new TreeMap<>();
        List<?> steps = (List<?>) FeatureOrderReference.capture().get("steps");
        Map<String, int[]> order = new HashMap<>();
        for (int step = 0; step < steps.size(); step++) {
            List<?> features = (List<?>) steps.get(step);
            for (int i = 0; i < features.size(); i++) order.put(((String) features.get(i)).split(":")[1], new int[]{step, i});
        }
        String[] names = {"ore_dirt", "ore_gravel", "ore_granite_upper", "ore_granite_lower", "ore_diorite_upper", "ore_diorite_lower",
            "ore_andesite_upper", "ore_andesite_lower", "ore_tuff", "ore_coal_upper", "ore_coal_lower", "ore_iron_upper", "ore_iron_middle",
            "ore_iron_small", "ore_gold", "ore_gold_lower", "ore_redstone", "ore_redstone_lower", "ore_diamond", "ore_diamond_medium",
            "ore_diamond_large", "ore_diamond_buried", "ore_lapis", "ore_lapis_buried", "ore_copper_large", "ore_copper",
            "ore_clay", "ore_gold_extra", "ore_emerald", "ore_infested"};
        for (String name : names) {
            int step = order.get(name)[0], index = order.get(name)[1];
            Object doc = json("placed_feature", name);
            String configName = ((String) call(call(doc, "get", "feature"), "getAsString")).split(":")[1];
            Object configured = json("configured_feature", configName);
            definitions.put(name, Map.of("placed", call(doc, "deepCopy"), "configured", configured));
            call(doc, "add", "feature", configured);
            Object modifiers = call(doc, "getAsJsonArray", "placement");
            // This fixture verifies count/in_square/height and shape RNG. The
            // biome filter has no RNG; its world lookup needs a separate oracle.
            for (int i = (int) call(modifiers, "size") - 1; i >= 0; i--) {
                Object modifier = call(call(modifiers, "get", i), "getAsJsonObject");
                if (call(call(modifier, "get", "type"), "getAsString").equals("minecraft:biome")) call(modifiers, "remove", i);
            }
            Object feature = call(call(field("world.level.levelgen.placement.PlacedFeature", "DIRECT_CODEC"), "parse", ops, doc), "getOrThrow");
            for (long seed : new long[]{0, 1, 17, 846692123413862008L}) {
                int cx = seed == 1 ? -2 : seed == 17 ? 3 : 0;
                int cz = seed == 1 ? -3 : seed == 17 ? -2 : 0;
                for (String scenario : List.of("stone", "deepslate", "cave")) samples.add(probePlacement(feature, generator, name, step, index, seed, scenario, cx, cz));
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        Map<String, Integer> states = new TreeMap<>();
        for (String block : List.of("CLAY", "INFESTED_STONE", "INFESTED_DEEPSLATE")) states.put(block, (int) call(type("world.level.block.Block"), "getId", state(block)));
        System.out.println("ORE_PLACEMENT_REFERENCE=" + call(gson, "toJson", Map.of("definitions", definitions, "samples", samples, "block_states", states)));
    }
}
