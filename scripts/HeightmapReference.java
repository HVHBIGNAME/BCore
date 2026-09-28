import java.lang.reflect.Proxy;
import java.util.*;
import java.util.function.Predicate;

/** Extract the actual 26.1 heightmap predicates and sapling support tag. */
public class HeightmapReference extends TreeReference {
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object supportTag = field("tags.BlockTags", "SUPPORTS_VEGETATION");
        Set<Object> supportsVegetation = tagBlocks("supports_vegetation");
        for (Object block : supportsVegetation) {
            call(call(block, "builtInRegistryHolder"), "bindTags", Set.of(supportTag));
        }
        Predicate<Object> surface = (Predicate<Object>) call(field("world.level.levelgen.Heightmap$Types", "WORLD_SURFACE"), "isOpaque");
        Predicate<Object> floor = (Predicate<Object>) call(field("world.level.levelgen.Heightmap$Types", "OCEAN_FLOOR"), "isOpaque");
        List<List<Integer>> ranges = new ArrayList<>();
        Set<Object> protectedSoils = tagBlocks("cannot_replace_below_tree_trunk");
        List<Integer> protectedStates = new ArrayList<>();
        int count = 0, start = 0, previous = -1;
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            int id = (int) call(type("world.level.block.Block"), "getId", state);
            if (id != count) throw new IllegalStateException("non-contiguous state registry");
            if (protectedSoils.contains(call(state, "getBlock"))) protectedStates.add(id);
            int flags = (surface.test(state) ? 1 : 0) | (floor.test(state) ? 2 : 0)
                | (supportsVegetation.contains(call(state, "getBlock")) ? 4 : 0);
            if (flags != previous) {
                if (count > 0) ranges.add(List.of(start, count, previous));
                start = count; previous = flags;
            }
            count++;
        }
        ranges.add(List.of(start, count, previous));
        List<Object> samples = new ArrayList<>();
        String[][] columns = {
            {}, {"GRASS_BLOCK"}, {"DIRT"}, {"COARSE_DIRT"}, {"PODZOL"}, {"ROOTED_DIRT"},
            {"MOSS_BLOCK"}, {"MUD"}, {"FARMLAND"}, {"SAND"}, {"GRASS_BLOCK", "SHORT_GRASS"},
            {"GRASS_BLOCK", "LEAF_LITTER"}, {"DIRT", "WATER"}, {"DIRT", "WATER", "WATER"},
            {"DIRT", "WATER", "WATER", "WATER"}, {"DIRT", "LAVA"},
            {"GRASS_BLOCK", "OAK_LOG", "OAK_LEAVES"}, {"GRASS_BLOCK", "AIR", "BIRCH_LEAVES"},
            {"STONE", "CAVE_AIR"}, {"STONE", "VOID_AIR"}
        };
        for (int baseY : new int[]{40, 64}) for (String[] names : columns) {
            Map<Integer, Object> column = new HashMap<>();
            List<Integer> ids = new ArrayList<>();
            int surfaceY = -64, floorY = -64;
            for (int i = 0; i < names.length; i++) {
                Object state = state(names[i]);
                column.put(baseY + i, state);
                ids.add((int) call(type("world.level.block.Block"), "getId", state));
                if (surface.test(state)) surfaceY = baseY + i + 1;
                if (floor.test(state)) floorY = baseY + i + 1;
            }
            Object reader = Proxy.newProxyInstance(HeightmapReference.class.getClassLoader(), new Class<?>[]{type("world.level.LevelReader")}, (p, m, a) -> {
                if (m.getName().equals("getBlockState")) return column.getOrDefault(Pos.from(a[0]).y(), state("AIR"));
                throw new UnsupportedOperationException(m.toString());
            });
            boolean survives = (boolean) call(state("OAK_SAPLING"), "canSurvive", reader, make("core.BlockPos", 8, floorY, 8));
            samples.add(Map.of("base_y", baseY, "states", ids, "world_surface", surfaceY, "ocean_floor", floorY, "sapling_survives", survives));
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("HEIGHTMAP_REFERENCE=" + call(gson, "toJson", Map.of("state_count", count, "ranges", ranges, "protected_trunk_states", protectedStates, "samples", samples)));
    }
}
