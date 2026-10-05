import java.lang.reflect.*;
import java.util.*;
import java.util.function.*;
import java.util.jar.*;

/** Native extra-tree catalog and full feature oracle; historical probes stay immutable. */
public class ExtraTreeReference extends VegetationReference {
    static final Map<String, Map<String, Object>> configurations = new TreeMap<>();
    static final Map<String, Map<String, Object>> placements = new TreeMap<>();
    static final Map<String, int[]> extraSlots = new TreeMap<>();
    static final Map<String, Object> extraBiomes = new TreeMap<>();
    static final List<String> EXTRA_KINDS = List.of("jungle_bush", "jungle_tree", "mega_jungle_tree",
        "dark_oak", "mega_pine", "mega_spruce", "azalea_tree", "cherry", "cherry_bees_005",
        "birch_bees_002", "birch_bees_005", "birch_leaf_litter", "dark_oak_leaf_litter",
        "fancy_oak_bees", "fancy_oak_bees_002", "fancy_oak_leaf_litter", "jungle_tree_no_vine",
        "oak_bees_002", "oak_leaf_litter", "pale_oak_bonemeal", "super_birch_bees", "swamp_oak",
        "mangrove", "tall_mangrove");
    static Pos writeSource;

    static boolean canWrite(Pos pos) {
        return pos.y() >= -64 && pos.y() <= 319 && (writeSource == null
            || Math.abs((pos.x() >> 4) - writeSource.x()) <= 1
                && Math.abs((pos.z() >> 4) - writeSource.z()) <= 1);
    }

    static Object extraWorld() throws Exception {
        InvocationHandler base = Proxy.getInvocationHandler(world());
        return Proxy.newProxyInstance(ExtraTreeReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p,m,a) -> {
            if (m.getName().equals("ensureCanWrite")) return canWrite(position(a[0]));
            if (m.getName().equals("getRawBrightness")) return 12;
            if (m.getName().equals("setBlock")) {
                Pos pos = position(a[0]);
                if (!canWrite(pos)) {
                    int[] row = {pos.x(),pos.y(),pos.z(),id(a[1]),(int)a[2],0};
                    digest(current.writeDigest,row);current.writeCount++;
                    if (current.writePrefix.size() < current.writeLimit) current.writePrefix.add(row);
                    return false;
                }
                Object previous = current.blockEntities.get(pos);
                boolean sameBlock = call(current.block(pos),"getBlock") == call(a[1],"getBlock");
                Object result = base.invoke(p,m,a);
                if (sameBlock && previous != null && (boolean) result) current.blockEntities.put(pos,previous);
                return result;
            }
            try { return base.invoke(p,m,a); }
            catch (UnsupportedOperationException e) {
                if (m.isDefault()) return InvocationHandler.invokeDefault(p,m,a);
                throw e;
            }
        });
    }

    static Map<String,Object> extraIsolated(String kind, long seed, String scenario, Pos origin) throws Exception {
        Map<Pos,Object> initial = new HashMap<>();
        if (scenario.equals("obstructed")) initial.put(new Pos(origin.x()+1,origin.y()+2,origin.z()),state("STONE"));
        if (scenario.equals("wet_foliage") || scenario.equals("flowing_water")) {
            Object water = state("WATER");
            if (scenario.equals("flowing_water")) water=call(water,"setValue",field("world.level.block.LiquidBlock","LEVEL"),1);
            initial.put(new Pos(origin.x()+1,origin.y()+3,origin.z()),water);
        }
        if (scenario.equals("persistent_leaf")) initial.put(new Pos(origin.x()+1,origin.y()+3,origin.z()),
            call(state("OAK_LEAVES"),"setValue",field("world.level.block.state.properties.BlockStateProperties","PERSISTENT"),true));
        if (scenario.equals("vine_cover")) initial.put(new Pos(origin.x()+1,origin.y()+3,origin.z()),call(type("world.level.block.Block"),"stateById",8373));
        if (scenario.equals("nearby_plants")) {
            String[] plants={"SHORT_GRASS","FERN","DANDELION","AZALEA","BROWN_MUSHROOM","RED_MUSHROOM"};
            for (int x=-4; x<=4; x++) for (int z=-4; z<=4; z++) {
                if (Math.abs(x)+Math.abs(z)<3) continue;
                initial.put(new Pos(origin.x()+x,origin.y(),origin.z()+z),state(plants[Math.floorMod(x*7+z,plants.length)]));
            }
        }
        if (scenario.equals("water_roots") || scenario.equals("flowing_water_roots")) {
            Object water=state("WATER");
            if (scenario.equals("flowing_water_roots")) water=call(water,"setValue",field("world.level.block.LiquidBlock","LEVEL"),3);
            for (int x=-10;x<=10;x++) for (int z=-10;z<=10;z++) {
                initial.put(new Pos(origin.x()+x,origin.y()-2,origin.z()+z),state("STONE"));
                initial.put(new Pos(origin.x()+x,origin.y()-1,origin.z()+z),state((x+z)%2==0 ? "MUD" : "CLAY"));
                for (int y=0;y<3;y++) initial.put(new Pos(origin.x()+x,origin.y()+y,origin.z()+z),water);
            }
        }
        current = new Capture(new Terrain(origin.y()-1,grass,air,initial,scenario.endsWith("reject_writes")),holder(biomes,"forest"));
        current.writeLimit = Integer.MAX_VALUE;
        writeSource = scenario.equals("bounded_source") ? new Pos(0,0,0) : null;
        Object raw=make("world.level.levelgen.WorldgenRandom",make("world.level.levelgen.XoroshiroRandomSource",seed));
        if (scenario.startsWith("advanced_")) { call(raw,"nextInt",17);call(raw,"nextFloat");call(raw,"nextInt",1073741825); }
        List<Pos> attempts=scenario.equals("repeat_live") ? List.of(origin,new Pos(origin.x()+2,origin.y(),origin.z()+4),origin) : List.of(origin);
        List<Object> results=new ArrayList<>();
        for (int i=0;i<attempts.size();i++) {
            Pos pos=attempts.get(i);
            boolean result=(boolean) call(call(holder(configured,kind),"value"),"place",extraWorld(),generator,random(raw),make("core.BlockPos",pos.x(),pos.y(),pos.z()));
            Map<String,Object> outcome=new LinkedHashMap<>();
            outcome.put("placed",result);outcome.put("origin",xyz(pos));
            if (i+1==attempts.size()) outcome.put("next_i64",call(raw,"nextLong"));
            results.add(outcome);
        }
        Map<String,Object> result=snapshot(kind,seed,scenario,origin,List.of(),results);
        if (writeSource != null) result.put("write_source",new int[]{writeSource.x(),writeSource.z()});
        result.put("raw_brightness",12);
        return result;
    }

    static Map<String,Object> placements() throws Exception {
        List<Object> samples=new ArrayList<>();
        for (String kind:EXTRA_KINDS) {
            for (long seed:new long[]{0,1,17,42}) for (String scenario:List.of("flat","obstructed","wet_foliage","reject_writes")) {
                samples.add(extraIsolated(kind,seed,scenario,new Pos(-1,65,16)));
            }
            for (String scenario:List.of("flowing_water","persistent_leaf","vine_cover","nearby_plants","advanced_stream","advanced_reject_writes","repeat_live")) {
                samples.add(extraIsolated(kind,17,scenario,new Pos(-1,65,16)));
            }
            for (Pos pos:List.of(new Pos(-17,65,-17),new Pos(-1,-63,16),new Pos(15,315,15),new Pos(0,65,0))) {
                samples.add(extraIsolated(kind,-1,"flat",pos));
            }
            samples.add(extraIsolated(kind,17,"bounded_source",new Pos(31,65,31)));
            if (kind.equals("mangrove") || kind.equals("tall_mangrove")) {
                for (long seed:new long[]{0,1,17,42}) for (String scenario:List.of("water_roots","flowing_water_roots")) {
                    samples.add(extraIsolated(kind,seed,scenario,new Pos(-1,65,16)));
                }
            }
        }
        return Map.of("version",1,"samples",samples,"scope",
            "Complete native configured trees on explicit live terrain: roots/trunks/foliage/decorators, edge updates, ordered writes/draws, continuation, block entities and raw tick requests. Explicit source write-radius-one case; no global scheduler claim.");
    }

    static String local(String key) { return key.replace("minecraft:", ""); }

    @SuppressWarnings("unchecked")
    static void loadCatalog() throws Exception {
        var jarPath = type("SharedConstants").getProtectionDomain().getCodeSource().getLocation().toURI();
        try (JarFile jar = new JarFile(new java.io.File(jarPath))) {
            for (var entries = jar.entries(); entries.hasMoreElements();) {
                String path = entries.nextElement().getName();
                for (String directory : List.of("configured_feature", "placed_feature")) {
                    String prefix = "data/minecraft/worldgen/" + directory + "/";
                    if (path.startsWith(prefix) && path.endsWith(".json")) {
                        String name = path.substring(prefix.length(), path.length() - 5);
                        (directory.equals("configured_feature") ? configurations : placements)
                            .put(name, (Map<String, Object>) json(directory, name));
                    }
                }
            }
        }
        Object source = call(generator, "getBiomeSource");
        List<?> possible = new ArrayList<>((Set<?>) call(source, "possibleBiomes"));
        Function<Object, Object> features = biome -> {
            try { return call(call(call(biome, "value"), "getGenerationSettings"), "features"); }
            catch (Exception e) { throw new RuntimeException(e); }
        };
        List<?> steps = (List<?>) call(type("world.level.biome.FeatureSorter"), "buildFeaturesPerStep", possible, features, true);
        for (var entry : placements.entrySet()) {
            if (!containsTree(entry.getValue(), new HashSet<>())) continue;
            Object feature = call(holder(placed, entry.getKey()), "value");
            for (int step = 0; step < steps.size(); step++) {
                int index = ((List<?>) call(steps.get(step), "features")).indexOf(feature);
                if (index >= 0) extraSlots.put(entry.getKey(), new int[]{step, index});
            }
            for (Object biome : possible) {
                if ((boolean) call(call(call(biome, "value"), "getGenerationSettings"), "hasFeature", feature)) {
                    extraBiomes.put(entry.getKey(), biome); break;
                }
            }
        }
    }

    static boolean containsTree(Object document, Set<String> seen) {
        if (document instanceof String key) {
            key = local(key);
            if (!seen.add(key)) return false;
            if (configurations.containsKey(key) && containsTree(configurations.get(key), seen)) return true;
            return placements.containsKey(key) && containsTree(placements.get(key), seen);
        }
        if (document instanceof Map<?, ?> map) {
            if ("minecraft:tree".equals(map.get("type"))) return true;
            for (Object value : map.values()) if (containsTree(value, seen)) return true;
        }
        if (document instanceof List<?> list) for (Object value : list) if (containsTree(value, seen)) return true;
        return false;
    }

    static Map<String, Object> catalog() throws Exception {
        Map<String, Object> trees = new TreeMap<>(), selectors = new TreeMap<>(), roots = new TreeMap<>();
        for (var entry : configurations.entrySet()) {
            Object kind = entry.getValue().get("type");
            if ("minecraft:tree".equals(kind) || "minecraft:huge_brown_mushroom".equals(kind)
                || "minecraft:huge_red_mushroom".equals(kind)) trees.put(entry.getKey(), entry.getValue());
            else if (containsTree(entry.getValue(), new HashSet<>())) selectors.put(entry.getKey(), entry.getValue());
        }
        for (var entry : extraSlots.entrySet()) roots.put(entry.getKey(), Map.of(
            "definition", placements.get(entry.getKey()), "slot", entry.getValue(),
            "biome", call(call(extraBiomes.get(entry.getKey()), "key"), "identifier").toString()));
        Map<String, Object> blocks = new TreeMap<>();
        Map<String, List<int[]>> tags = new TreeMap<>();
        Object contextFree = Proxy.newProxyInstance(ExtraTreeReference.class.getClassLoader(), new Class<?>[]{type("world.level.BlockGetter")},
            (p,m,a) -> { throw new UnsupportedOperationException("Contextual plant support " + m); });
        for (Object block : (Iterable<?>) field("core.registries.BuiltInRegistries", "BLOCK")) {
            List<?> states = (List<?>) call(call(block, "getStateDefinition"), "getPossibleStates");
            List<Object> properties = new ArrayList<>();
            for (Object property : (Collection<?>) call(call(block, "getStateDefinition"), "getProperties")) {
                Set<String> values = new LinkedHashSet<>();
                for (Object state : states) values.add((String) call(property, "getName", call(state, "getValue", property)));
                properties.add(Map.of("name", call(property, "getName"), "values", values));
            }
            for (Object tag : ((java.util.stream.Stream<?>) call(call(block, "builtInRegistryHolder"), "tags")).toList()) {
                tags.computeIfAbsent(call(tag, "location").toString(), k -> new ArrayList<>())
                    .add(new int[]{id(states.get(0)), id(states.get(states.size()-1)) + 1});
            }
            Map<String,Object> definition = new LinkedHashMap<>(Map.of(
                "first", id(states.get(0)), "end", id(states.get(states.size()-1)) + 1,
                "default", id(call(block, "defaultBlockState")), "properties", properties,
                "class", block.getClass().getSimpleName(), "update_shape", methodOwner(block.getClass(), "updateShape", 8),
                "can_survive", methodOwner(block.getClass(), "canSurvive", 3)));
            if (type("world.level.block.VegetationBlock").isInstance(block)) {
                Method support = null;
                for (Class<?> cls=block.getClass(); cls != null && support == null; cls=cls.getSuperclass()) {
                    for (Method m:cls.getDeclaredMethods()) if (m.getName().equals("mayPlaceOn") && m.getParameterCount()==3) { support=m; break; }
                }
                Objects.requireNonNull(support).setAccessible(true);
                List<int[]> ranges = new ArrayList<>();
                int[] previous = null;
                try {
                    for (Object below : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
                        if (!(boolean) support.invoke(block, below, contextFree, make("core.BlockPos",0,64,0))) continue;
                        int value=id(below);
                        if (previous != null && previous[1] == value) previous[1]++;
                        else { previous=new int[]{value,value+1};ranges.add(previous); }
                    }
                    definition.put("may_place_on", ranges);
                } catch (InvocationTargetException e) {
                    if (!(e.getCause() instanceof UnsupportedOperationException)) throw e;
                }
            }
            blocks.put(call(field("core.registries.BuiltInRegistries", "BLOCK"), "getKey", block).toString(), definition);
        }
        for (List<int[]> ranges : tags.values()) ranges.sort(Comparator.comparingInt(r -> r[0]));
        return Map.of("trees", trees, "selectors", selectors, "roots", roots, "placements", placements,
            "blocks", blocks, "tags", tags, "samples", List.of());
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        boolean samples="placements".equals(System.getenv("BCORE_EXTRA_TREE_PROBE"));
        if (!samples) loadCatalog();
        System.out.println("EXTRA_TREE_REFERENCE=" + call(gson, "toJson", samples ? placements() : catalog()));
    }
}
