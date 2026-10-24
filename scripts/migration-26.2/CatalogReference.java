import java.lang.reflect.Field;
import java.util.*;

/** Select metadata from the unchanged probes without running their sample suites. */
public final class CatalogReference extends TreeReference {
    static Object probe(String owner, String method, Object... args) throws Exception {
        return call(Class.forName(owner), method, args);
    }

    static Object loadedRegistry(Object loaded, String name) throws Exception {
        return call(loaded, "lookupOrThrow", field("core.registries.Registries", name));
    }

    static Object namedValue(Object registry, String name) throws Exception {
        Object key = call(type("resources.Identifier"), "withDefaultNamespace", name);
        return call(((Optional<?>) call(registry, "get", key)).orElseThrow(), "value");
    }

    static Object member(Object value, String name) throws Exception {
        Field field = value.getClass().getDeclaredField(name);
        field.setAccessible(true);
        return field.get(value);
    }

    static void setProbeField(String owner, String name, Object value) throws Exception {
        Field field = Class.forName(owner).getDeclaredField(name);
        field.setAccessible(true);
        field.set(null, value);
    }

    static Object mineshaftBlocks() throws Exception {
        probe("OreReference", "bootstrapOre");
        NativeWorldgenRegistries.load();
        setProbeField("MineshaftBlockReference", "blockEntities", new HashMap<>());
        setProbeField("MineshaftBlockReference", "minecarts", new ArrayList<>());
        setProbeField("MineshaftBlockReference", "shapeRandom", make("world.level.levelgen.LegacyRandomSource", 0L));
        @SuppressWarnings("unchecked")
        Map<String, Object> data = new LinkedHashMap<>((Map<String, Object>) probe("MineshaftBlockReference", "predicates"));
        data.put("materials", probe("MineshaftBlockReference", "materials"));
        return data;
    }

    static Object mineshaftStart() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object structures = loadedRegistry(NativeWorldgenRegistries.load(), "STRUCTURE");
        List<List<String>> steps = new ArrayList<>();
        for (Object ignored : type("world.level.levelgen.GenerationStep$Decoration").getEnumConstants()) steps.add(new ArrayList<>());
        for (Object structure : (Iterable<?>) structures) {
            int step = ((Enum<?>) call(structure, "step")).ordinal();
            steps.get(step).add(call(structures, "getKey", structure).toString());
        }
        Map<String, Object> data = new LinkedHashMap<>();
        for (String name : List.of("mineshaft", "mineshaft_mesa")) {
            Object structure = namedValue(structures, name);
            data.put(name + "_biomes", probe("MineshaftStartReference", "names", call(structure, "biomes")));
        }
        data.put("structure_steps", steps);
        return data;
    }

    static Object surfaceClimates() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object biomes = loadedRegistry(NativeWorldgenRegistries.load(), "BIOME");
        Map<String, Object> climates = new LinkedHashMap<>();
        for (Object biome : (Iterable<?>) biomes) {
            Object settings = member(biome, "climateSettings");
            boolean frozen = ((Enum<?>) call(settings, "temperatureModifier")).name().equals("FROZEN");
            int bits = Float.floatToRawIntBits((float) call(biome, "getBaseTemperature"));
            climates.put(call(biomes, "getKey", biome).toString(), List.of(String.format(Locale.ROOT, "%08x", bits), frozen));
        }
        return Map.of("climates", climates, "biome_count", climates.size());
    }

    static Object surfaceBlocks() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        List<Object> ranges = new ArrayList<>();
        int count = 0;
        int start = -1;
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            int id = (int) call(type("world.level.block.Block"), "getId", state);
            if (id != count++) throw new IllegalStateException("non-contiguous native state ID");
            boolean fluid = !(boolean) call(call(state, "getFluidState"), "isEmpty");
            if (fluid && start < 0) start = id;
            if (!fluid && start >= 0) {
                ranges.add(List.of(start, id));
                start = -1;
            }
        }
        if (start >= 0) ranges.add(List.of(start, count));
        Map<String, Integer> palette = new TreeMap<>();
        for (String name : List.of("WATER", "LAVA", "PACKED_ICE", "SNOW_BLOCK", "ICE", "STONE", "TUFF", "CAVE_AIR", "VOID_AIR")) {
            palette.put(name, (int) call(type("world.level.block.Block"), "getId", state(name)));
        }
        return Map.of("state_count", count, "fluid_ranges", ranges, "palette", palette);
    }

    static Object structures(String mode, String jar) throws Exception {
        probe("JigsawSupport", "bootstrap");
        try {
            probe("JigsawReference", "includeBuiltInRegistries");
            return switch (mode) {
                case "scattered_catalog" -> Map.of("catalog", probe("ScatteredStructureReference", "catalog", jar),
                    "block_properties", probe("ScatteredStructureReference", "blockProperties"));
                case "pyramid_catalog" -> Map.of("catalog", probe("DesertPyramidReference", "pyramidCatalog", jar));
                case "scattered_containers" -> Map.of("containers", probe("ScatteredStructureReference", "scatteredContainers"));
                default -> throw new IllegalArgumentException(mode);
            };
        } finally {
            Field resources = Class.forName("JigsawSupport").getDeclaredField("resources");
            resources.setAccessible(true);
            call(resources.get(null), "close");
        }
    }

    static Object capture(String mode, String jar) throws Exception {
        return switch (mode) {
            case "standing_blocks" -> {
                probe("VegetationReference", "bootstrap");
                yield probe("VegetationReference", "blockPredicates");
            }
            case "mineshaft_blocks" -> mineshaftBlocks();
            case "mineshaft_start" -> mineshaftStart();
            case "surface_blocks" -> surfaceBlocks();
            case "surface_climates" -> surfaceClimates();
            case "scattered_catalog", "pyramid_catalog", "scattered_containers" -> structures(mode, jar);
            default -> throw new IllegalArgumentException(mode);
        };
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 2) throw new IllegalArgumentException("usage: CatalogReference mode jar");
        Object data = capture(args[0], args[1]);
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("CATALOG_REFERENCE=" + call(gson, "toJson", data));
    }
}
