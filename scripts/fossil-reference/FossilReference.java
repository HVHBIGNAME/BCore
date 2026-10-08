import java.lang.reflect.*;
import java.util.*;

/** Real FossilFeature/StructureTemplate on explicitly scripted component worlds. */
public class FossilReference extends JigsawSupport {
    static Object templateLevel;

    static class World implements InvocationHandler {
        final String mode;
        final int cx, cz;
        final Object base, airState, random;
        final Map<Pos,Object> changed = new HashMap<>();
        final List<Object> writes = new ArrayList<>(), ticks = new ArrayList<>(), marks = new ArrayList<>(), heights = new ArrayList<>();

        World(String mode, Pos origin) throws Exception {
            this.mode = mode; cx = origin.x() >> 4; cz = origin.z() >> 4;
            airState = state("AIR");
            base = state(mode.equals("half_air") ? "STONE" : mode.toUpperCase(Locale.ROOT));
            random = make("world.level.levelgen.XoroshiroRandomSource", 0L);
        }
        Object initial(Pos p) {
            return p.y() < -64 || p.y() >= 65 || mode.equals("half_air") && (p.x() & 1) == 0 ? airState : base;
        }
        Object block(Object value) throws Exception {
            Pos p = Pos.from(value);
            if (Math.abs((p.x() >> 4) - cx) > 1 || Math.abs((p.z() >> 4) - cz) > 1)
                throw new IllegalStateException("read beyond fossil write-square fixture: " + p);
            return changed.getOrDefault(p, initial(p));
        }
        boolean writable(Pos p) {
            return p.y() >= -64 && p.y() <= 319 && Math.abs((p.x() >> 4) - cx) <= 1 && Math.abs((p.z() >> 4) - cz) <= 1;
        }
        public Object invoke(Object proxy, Method method, Object[] a) throws Throwable {
            try {
                return switch (method.getName()) {
                    case "getLevel" -> templateLevel;
                    case "getMinY" -> -64;
                    case "getMaxY" -> 319;
                    case "getHeight" -> {
                        if (a == null || a.length == 0) yield 384;
                        heights.add(List.of(a[0].toString(), a[1], a[2], 65)); yield 65;
                    }
                    case "ensureCanWrite" -> writable(Pos.from(a[0]));
                    case "isOutsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : Pos.from(a[0]).y(); yield y < -64 || y > 319; }
                    case "getBlockState" -> block(a[0]);
                    case "getFluidState" -> call(block(a[0]), "getFluidState");
                    case "isEmptyBlock" -> call(block(a[0]), "isAir");
                    case "getRandom" -> random;
                    case "registryAccess" -> registries;
                    case "getBlockEntity" -> null;
                    case "setBlock" -> {
                        Pos p = Pos.from(a[0]);
                        if (!writable(p)) yield false;
                        changed.put(p,a[1]); writes.add(List.of(p.x(),p.y(),p.z(),stateId(a[1]),a[2]));
                        if (((int)a[2] & 16) == 0) {
                            Object mark = call(a[1], "getPostProcessPos", proxy, a[0]);
                            if (mark != null) marks.add(xyz(mark));
                        }
                        yield true;
                    }
                    case "scheduleTick" -> {
                        boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                        int id = fluid ? (int)call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]) : stateId(call(a[1], "defaultBlockState"));
                        Pos p = Pos.from(a[0]);
                        ticks.add(List.of(p.x(),p.y(),p.z(),id,a[2],fluid ? 1 : 0)); yield null;
                    }
                    // The production WorldGenRegion inherits this native no-op.
                    case "updateNeighborsAt" -> InvocationHandler.invokeDefault(proxy,method,a);
                    case "toString" -> "FossilReference scripted " + mode;
                    default -> throw new UnsupportedOperationException(method.toString());
                };
            } catch (InvocationTargetException e) { throw e.getCause(); }
        }
        Object view() throws Exception {
            return Proxy.newProxyInstance(getClass().getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, this);
        }
        List<Object> changes() throws Exception {
            List<Pos> positions = new ArrayList<>(changed.keySet());
            positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
            List<Object> out = new ArrayList<>();
            for (Pos p : positions) if (changed.get(p) != initial(p)) out.add(List.of(p.x(),p.y(),p.z(),stateId(changed.get(p))));
            return out;
        }
    }

    static Object random(long seed) throws Exception {
        return make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource",seed));
    }
    static Map<String,Object> run(String feature, long seed, Pos origin, String mode, String selection) throws Exception {
        World world = new World(mode,origin);
        Object configured = call(call(registry("CONFIGURED_FEATURE"),"getOrThrow",key("CONFIGURED_FEATURE","minecraft:"+feature)),"value");
        Object rng = random(seed);
        boolean placed = (boolean)call(configured,"place",world.view(),null,rng,make("core.BlockPos",origin.x(),origin.y(),origin.z()));
        return Map.of("feature",feature,"seed",seed,"origin",List.of(origin.x(),origin.y(),origin.z()),"mode",mode,"selection",selection,
            "result",placed,"next_long",call(rng,"nextLong"),"writes",world.writes,"ticks",world.ticks,
            "snapshot",Map.of("changes",world.changes(),"postprocessing",world.marks,"height_queries",world.heights));
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        // FossilFeature only uses these two native accessors to reach the real
        // template manager. All other world methods are guarded scripted inputs.
        Object server = allocate(type("server.dedicated.DedicatedServer"));
        Field manager = type("server.MinecraftServer").getDeclaredField("structureTemplateManager");
        manager.setAccessible(true); manager.set(server,templateManager);
        templateLevel = allocate(type("server.level.ServerLevel"));
        Field owner = type("server.level.ServerLevel").getDeclaredField("server");
        owner.setAccessible(true); owner.set(templateLevel,server);
        Map<String,Long> selections = new TreeMap<>();
        for (long seed=0; selections.size()<32 && seed<10000; seed++) {
            Object rng = random(seed);
            String rotation = call(type("world.level.block.Rotation"),"getRandom",rng).toString();
            selections.putIfAbsent(rotation+":"+call(rng,"nextInt",8),seed);
        }
        if (selections.size()!=32) throw new IllegalStateException("incomplete rotation/template input coverage");
        List<Object> cases = new ArrayList<>();
        for (String feature : List.of("fossil_coal","fossil_diamonds")) {
            int index=0;
            for (var entry : selections.entrySet()) {
                Pos origin = switch(index++ % 3) { case 0 -> new Pos(-17,180,-1); case 1 -> new Pos(31,-64,32); default -> new Pos(-32,-20,-17); };
                cases.add(run(feature,entry.getValue(),origin,feature.equals("fossil_coal")?"stone":"deepslate",entry.getKey()));
            }
            // NONE + skull_1 has even width, giving exactly four empty corners
            // in half_air, alongside eight-corner rejections and protected writes.
            for (String mode : List.of("air","water","lava","half_air","bedrock","gravel"))
                cases.add(run(feature,selections.get("NONE:4"),new Pos(-17,80,-1),mode,"NONE:4"));
        }
        Field shapeOrder = type("world.level.block.state.BlockBehaviour").getDeclaredField("UPDATE_SHAPE_ORDER");
        shapeOrder.setAccessible(true);
        output("FOSSILREFERENCE",Map.of("cases",cases,"shape_order",Arrays.stream((Object[])shapeOrder.get(null)).map(Object::toString).toList(),
            "scope","Actual configured FossilFeature and native loaded templates/processors. Scripted 3x3 fresh worlds: y<65 base terrain (or half-air columns); WG height 65; write guard [-64,319]. No Rust-generated expected data. Native server/level accessors only expose the template manager; gameplay and chunk scheduling are outside this component probe."));
        call(resources,"close");
    }
}
