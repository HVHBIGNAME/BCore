import java.util.*;

/** Original registry order, state properties and integer climate rows from the loaded target. */
public class RegistryReference extends TreeReference {
    static List<Object> registry(Object registry) throws Exception {
        List<Object> rows = new ArrayList<>();
        for (Object value : (Iterable<?>) registry) {
            int id = (int) call(registry, "getId", value);
            if (id != rows.size()) throw new IllegalStateException("non-contiguous registry " + registry);
            rows.add(Map.of("id", id, "key", call(registry, "getKey", value).toString()));
        }
        return rows;
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object loaded = NativeWorldgenRegistries.load();
        Map<String, Object> registries = new TreeMap<>();
        for (String name : List.of("BLOCK", "ITEM", "ENTITY_TYPE", "ATTRIBUTE", "BLOCK_ENTITY_TYPE",
                                  "DATA_COMPONENT_TYPE", "FLUID", "POTION", "MOB_EFFECT")) {
            registries.put(name, registry(field("core.registries.BuiltInRegistries", name)));
        }
        Object biomes = call(loaded, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        registries.put("BIOME", registry(biomes));
        List<Object> states = new ArrayList<>();
        Object blocks = field("core.registries.BuiltInRegistries", "BLOCK");
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            int id = (int) call(type("world.level.block.Block"), "getId", state);
            if (id != states.size()) throw new IllegalStateException("non-contiguous states");
            Object block = call(state, "getBlock");
            Map<String, String> properties = new TreeMap<>();
            for (Object property : (Collection<?>) call(call(block, "getStateDefinition"), "getProperties")) {
                Object value = call(state, "getValue", property);
                properties.put((String) call(property, "getName"), (String) call(property, "getName", value));
            }
            states.add(Map.of("id", id, "name", call(blocks, "getKey", block).toString(),
                              "properties", properties, "default", state == call(block, "defaultBlockState")));
        }
        Object preset = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
            field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomes);
        List<Object> climate = new ArrayList<>();
        List<Object> encoded = new ArrayList<>();
        Object jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        for (Object pair : (List<?>) call(call(preset, "parameters"), "values")) {
            Object point = call(pair, "getFirst"), holder = call(pair, "getSecond");
            String biome = call(call(holder, "key"), "identifier").toString();
            List<Object> ranges = new ArrayList<>();
            for (String dimension : List.of("temperature", "humidity", "continentalness", "erosion", "depth", "weirdness")) {
                Object range = call(point, dimension);
                ranges.add(List.of(call(range, "min"), call(range, "max")));
            }
            climate.add(Map.of("biome", biome, "ranges", ranges, "offset", call(point, "offset")));
            Object value = call(call(field("world.level.biome.Climate$ParameterPoint", "CODEC"),
                                    "encodeStart", jsonOps, point), "getOrThrow");
            encoded.add(Map.of("biome", biome, "parameters", value));
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("MIGRATION_REFERENCE=" + call(gson, "toJson", Map.of(
            "registries", registries, "states", states, "climate_rows", climate,
            "parameters", Map.of("biomes", encoded))));
    }
}
