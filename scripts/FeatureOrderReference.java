import java.util.*;
import java.util.function.Function;
import java.util.stream.Stream;

/** Native overworld BiomeSource + FeatureSorter registry order. */
public class FeatureOrderReference extends TreeReference {
    static Map<String, Object> capture() throws Exception {
        Object lookup = call(type("data.registries.VanillaRegistries"), "createLookup");
        Object biomes = call(lookup, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        Object parameters = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
            field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomes);
        Object source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", parameters));
        List<?> possible = new ArrayList<>((Set<?>) call(source, "possibleBiomes"));
        List<String> biomeNames = new ArrayList<>();
        for (Object biome : possible) biomeNames.add(call(call(biome, "key"), "identifier").toString());
        Object registry = call(lookup, "lookupOrThrow", field("core.registries.Registries", "PLACED_FEATURE"));
        Map<Object, String> names = new IdentityHashMap<>();
        try (Stream<?> elements = (Stream<?>) call(registry, "listElements")) {
            for (Object holder : elements.toList()) names.put(call(holder, "value"), call(call(holder, "key"), "identifier").toString());
        }
        Function<Object, Object> features = holder -> {
            try { return call(call(call(holder, "value"), "getGenerationSettings"), "features"); }
            catch (Exception e) { throw new RuntimeException(e); }
        };
        List<?> sorted = (List<?>) call(type("world.level.biome.FeatureSorter"), "buildFeaturesPerStep", possible, features, true);
        List<List<String>> steps = new ArrayList<>();
        for (Object step : sorted) {
            List<String> row = new ArrayList<>();
            for (Object feature : (List<?>) call(step, "features")) row.add(Objects.requireNonNull(names.get(feature), "unregistered feature"));
            steps.add(row);
        }
        return Map.of("possible_biomes", biomeNames, "steps", steps, "samples", List.of());
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("FEATURE_ORDER_REFERENCE=" + call(gson, "toJson", capture()));
    }
}
