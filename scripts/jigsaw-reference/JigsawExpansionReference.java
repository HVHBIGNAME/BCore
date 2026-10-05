import java.lang.reflect.*;
import java.util.*;

/** Native trail-ruins assembly and complete/clipped archaeology placement. */
public class JigsawExpansionReference extends JigsawReference {
    static Object shell(String name) throws Exception {
        Class<?> unsafe = Class.forName("sun.misc.Unsafe");
        Field field = unsafe.getDeclaredField("theUnsafe"); field.setAccessible(true);
        return unsafe.getMethod("allocateInstance",Class.class).invoke(field.get(null),type(name));
    }
    static void assign(Object target, String name, Object value) throws Exception {
        for (Class<?> c = target.getClass(); c != null; c = c.getSuperclass()) {
            try { Field f = c.getDeclaredField(name); f.setAccessible(true); f.set(target,value); return; }
            catch (NoSuchFieldException ignored) { }
        }
        throw new NoSuchFieldException(name);
    }
    static Object seededWorld(long seed) throws Exception {
        // CappedProcessor calls ServerLevelAccessor.getLevel().getSeed(), not the
        // world's getSeed default. Keep the real native getters, binding only the
        // query-only server services they actually read.
        Object server = shell("server.dedicated.DedicatedServer"), level = shell("server.level.ServerLevel");
        Object options = make("world.level.levelgen.WorldOptions",seed,true,false);
        assign(server,"worldGenSettings",make("world.level.levelgen.WorldGenSettings",options,(Object)null));
        assign(level,"server",server);
        if ((long) call(level,"getSeed") != seed) throw new IllegalStateException("native world seed binding");
        Object base = JigsawReference.world();
        return Proxy.newProxyInstance(JigsawExpansionReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
                if (m.getName().equals("getSeed")) return call(level,"getSeed");
                if (m.getName().equals("getLevel")) return level;
                try { return m.invoke(base, a); }
                catch (InvocationTargetException error) { throw error.getCause(); }
            });
    }

    static Map<String,Object> trail(long seed, int cx, int cz, boolean entire, String environment) throws Exception {
        Object start = nativeStart("minecraft:trail_ruins", seed, 0, 0);
        terrain = environment; fill = state("STONE");
        placed = new HashMap<>(); blockEntities = new HashMap<>(); writes = new ArrayList<>();
        ticks = new ArrayList<>(); markChunks = new HashMap<>();
        Object clip = entire ? call(start,"getBoundingBox") : make("world.level.levelgen.structure.BoundingBox",cx*16,-64,cz*16,cx*16+15,319,cz*16+15);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource",918273L));
        call(start,"placeInChunk",seededWorld(seed),null,generator,random,clip,make("world.level.ChunkPos",cx,cz));
        List<Pos> positions = new ArrayList<>(placed.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        List<List<Integer>> states = new ArrayList<>();
        List<Object> entities = new ArrayList<>();
        for (Pos p : positions) {
            states.add(List.of(p.x(),p.y(),p.z(),stateId(placed.get(p))));
            if (blockEntities.containsKey(p)) entities.add(Map.of("pos",List.of(p.x(),p.y(),p.z()),
                "nbt",nbt64(call(blockEntities.get(p),"saveWithFullMetadata",registries))));
        }
        Map<String,Object> result = new LinkedHashMap<>();
        result.put("structure","minecraft:trail_ruins"); result.put("seed",seed); result.put("chunk",List.of(cx,cz));
        result.put("entire",entire); result.put("terrain",environment); result.put("placement_seed",918273L);
        result.put("clip",bounds(clip)); result.put("write_count",writes.size()); result.put("writes_md5",digest(writes));
        result.put("state_count",states.size()); result.put("states_md5",digest(states)); result.put("block_entities",entities);
        result.put("ticks",ticks); result.put("next_i64",call(random,"nextLong"));
        return result;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        List<Object> assemblies = new ArrayList<>(), placements = new ArrayList<>();
        for (long seed : new long[]{0,1,42,-17}) {
            assemblies.add(assembly("minecraft:trail_ruins",seed,seed==42?-3:0,seed==-17?5:0));
            placements.add(trail(seed,0,0,true,"stone"));
            placements.add(trail(seed,0,0,false,"slope"));
        }
        output("JIGSAWEXPANSIONREFERENCE",Map.of("assembly",assemblies,"placement",placements,
            "scope","Actual native trail-ruins jigsaw assembly and StructureStart.placeInChunk; controlled stone/slope worlds, native capped archaeology processors and saved brushable-block NBT. No server scheduler or entity factory."));
        call(resources,"close");
    }
}
