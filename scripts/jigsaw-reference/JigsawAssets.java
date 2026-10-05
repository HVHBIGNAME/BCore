import java.util.*;
import java.util.jar.*;

/** Native version metadata, not sampled world layouts. */
public class JigsawAssets extends JigsawSupport {
    static void locations(Object value, Set<String> names) {
        if (value instanceof Map<?, ?> map) {
            if (map.get("location") instanceof String name) names.add(name);
            for (Object child : map.values()) locations(child, names);
        } else if (value instanceof List<?> list) for (Object child : list) locations(child, names);
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        Set<String> families = args.length > 1 ? new HashSet<>(Arrays.asList(Arrays.copyOfRange(args, 1, args.length)))
            : Set.of("ancient_city", "village");
        Map<String, Object> templates = new TreeMap<>();
        Set<Object> usedBlocks = new HashSet<>();
        Set<String> referenced = new TreeSet<>();
        Set<String> missing = new TreeSet<>();
        try (JarFile jar = new JarFile(args[0])) {
            for (JarEntry entry : Collections.list(jar.entries())) {
                String path = entry.getName();
                if (path.startsWith("data/minecraft/worldgen/template_pool/") && path.endsWith(".json")
                    && families.stream().anyMatch(family -> path.contains("/" + family + "/"))) {
                    String json = new String(jar.getInputStream(entry).readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
                    Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
                    locations(call(gson, "fromJson", json, Object.class), referenced);
                }
                if (!path.startsWith("data/minecraft/structure/") || !path.endsWith(".nbt")
                    || families.stream().noneMatch(family -> path.contains("/" + family + "/"))) continue;
                String name = "minecraft:" + path.substring("data/minecraft/structure/".length(), path.length() - 4);
                Object template = template(name);
                templates.put(name, nbt64(call(template, "save", make("nbt.CompoundTag"))));
                for (Object palette : (List<?>) member(template, "palettes")) {
                    for (Object info : (List<?>) call(palette, "blocks")) usedBlocks.add(call(call(info, "state"), "getBlock"));
                }
            }
        }
        // Some bundled pools really reference absent files. Capture the native
        // manager's getOrCreate result instead of substituting a guessed piece.
        for (String name : referenced) if (!templates.containsKey(name)) {
            if (((Optional<?>) call(templateManager, "get", identifier(name))).isPresent()) throw new IllegalStateException("uncaptured existing template " + name);
            missing.add(name);
            Object emptyTemplate = call(templateManager, "getOrCreate", identifier(name));
            templates.put(name, nbt64(call(emptyTemplate, "save", make("nbt.CompoundTag"))));
        }
        // Include every state in the registry: processors read arbitrary live
        // terrain, and callers may supply custom templates using other blocks.
        List<Object> states = new ArrayList<>();
        Map<String, Integer> defaults = new TreeMap<>();
        Object empty = field("world.level.EmptyBlockGetter", "INSTANCE");
        Object zero = field("core.BlockPos", "ZERO");
        Object[] rotations = type("world.level.block.Rotation").getEnumConstants();
        Object[] mirrors = type("world.level.block.Mirror").getEnumConstants();
        Map<String, Object> blockEntities = new TreeMap<>();
        for (Object block : (Iterable<?>) blockRegistry) {
            String name = call(blockRegistry, "getKey", block).toString();
            defaults.put(name, stateId(call(block, "defaultBlockState")));
            // Archaeology processors create brushable blocks which are absent
            // from the templates' original gravel/sand palettes.
            if ((usedBlocks.contains(block) || name.startsWith("minecraft:sculk_")
                || families.contains("trail_ruins") && name.startsWith("minecraft:suspicious_"))
                && type("world.level.block.EntityBlock").isInstance(block)) {
                Object entity = call(block, "newBlockEntity", zero, call(block, "defaultBlockState"));
                if (entity != null) {
                    Object entityType = call(entity, "getType");
                    Object types = field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE");
                    blockEntities.put(name, Map.of("type_id", call(types, "getId", entityType),
                        "id", call(types, "getKey", entityType).toString(),
                        "randomizable", type("world.RandomizableContainer").isInstance(entity),
                        "nbt", nbt64(call(entity, "saveWithFullMetadata", registries))));
                }
            }
            for (Object state : (List<?>) call(call(block, "getStateDefinition"), "getPossibleStates")) {
                int id = stateId(state);
                List<Integer> rotated = new ArrayList<>(), mirrored = new ArrayList<>();
                for (Object rotation : rotations) rotated.add(stateId(call(state, "rotate", rotation)));
                for (Object mirror : mirrors) mirrored.add(stateId(call(state, "mirror", mirror)));
                Object spec = jsonValue(call(type("nbt.NbtUtils"), "writeBlockState", state));
                int flags = 0;
                if ((boolean) call(state, "isAir")) flags |= 1;
                if ((boolean) call(state, "canBeReplaced")) flags |= 2;
                if (!(boolean) call(block, "hasDynamicShape") && (boolean) call(state, "isCollisionShapeFullBlock", empty, zero)) flags |= 4;
                if ((boolean) call(state, "hasBlockEntity")) flags |= 8;
                if (type("world.level.block.LiquidBlockContainer").isInstance(block)) flags |= 16;
                if ((boolean) call(type("world.level.block.Block"), "isShapeFullBlock", call(state, "getShape", empty, zero))) flags |= 32;
                Object fluid = call(state, "getFluidState");
                Object fluidType = call(fluid, "getType");
                int fluidKind = (boolean) call(fluid, "isEmpty") ? 0 :
                    (fluidType == field("world.level.material.Fluids", "WATER") || fluidType == field("world.level.material.Fluids", "FLOWING_WATER") ? 1 : 2);
                boolean source = (boolean) call(fluid, "isSource");
                // A compact, positional schema keeps this complete registry small.
                states.add(List.of(id, spec, rotated, mirrored, flags, fluidKind, source));
            }
        }
        states.sort(Comparator.comparingInt(row -> (Integer) ((List<?>) row).get(0)));
        Map<String, Object> structureMetadata = new TreeMap<>();
        int[] indices = new int[11];
        Object structures = registry("STRUCTURE");
        Object biomes = registry("BIOME");
        for (Object structure : (Iterable<?>) structures) {
            String name = call(structures, "getKey", structure).toString();
            int step = ((Enum<?>) call(structure, "step")).ordinal();
            int index = indices[step]++;
            if (families.stream().anyMatch(family -> name.equals("minecraft:" + family)
                || family.equals("village") && name.startsWith("minecraft:village_"))) {
                List<Integer> allowed = new ArrayList<>();
                for (Object holder : (Iterable<?>) call(structure, "biomes")) allowed.add((int) call(biomes, "getId", call(holder, "value")));
                Collections.sort(allowed);
                structureMetadata.put(name, Map.of("step", step, "feature_index", index, "biomes", allowed));
            }
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("template_nbt", templates); result.put("states", states); result.put("defaults", defaults);
        result.put("block_entities", blockEntities); result.put("missing_templates", missing);
        result.put("water_fluid_id", call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", field("world.level.material.Fluids", "WATER")));
        result.put("rotation_order", Arrays.stream(rotations).map(Object::toString).toList());
        result.put("mirror_order", Arrays.stream(mirrors).map(Object::toString).toList());
        result.put("structure_metadata", structureMetadata);
        output("JIGSAWASSETS", result);
        call(resources, "close");
    }
}
