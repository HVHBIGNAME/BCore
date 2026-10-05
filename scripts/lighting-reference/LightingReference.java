import java.lang.reflect.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import java.util.function.*;

/** Native light engines, WorldGenRegion queries and state/shape metadata, 26.1.
 * No copied lighting/freezing/survival algorithm. Reflection permits javac 21.
 */
public class LightingReference extends BaseFeatureReference {
    static final Map<String, Method> methods = new HashMap<>();
    static Object call(Object target, String name, Object... args) throws Exception {
        Class<?> owner = target instanceof Class<?> c ? c : target.getClass();
        String key = owner.getName() + "." + name + Arrays.toString(Arrays.stream(args)
            .map(a -> a == null ? "null" : a.getClass().getName()).toArray());
        Method method = methods.get(key);
        if (method == null) {
            search: for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
                for (Method m : c.getDeclaredMethods()) {
                    if (m.getName().equals(name) && matches(m.getParameterTypes(), args)) {
                        method = m; break search;
                    }
                }
            }
            if (method == null) for (Method m : owner.getMethods()) {
                if (m.getName().equals(name) && matches(m.getParameterTypes(), args)) { method = m; break; }
            }
            if (method == null) throw new NoSuchMethodException(key);
            method.setAccessible(true); methods.put(key, method);
        }
        try { return method.invoke(target instanceof Class<?> ? null : target, args); }
        catch (InvocationTargetException e) {
            if (e.getCause() instanceof Exception cause) throw cause;
            if (e.getCause() instanceof Error cause) throw cause;
            throw e;
        }
    }

    static Field member(Class<?> owner, String name) throws Exception {
        for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
            try { Field f = c.getDeclaredField(name); f.setAccessible(true); return f; }
            catch (NoSuchFieldException ignored) { }
        }
        throw new NoSuchFieldException(owner.getName() + "." + name);
    }
    static void set(Object target, String name, Object value) throws Exception {
        member(target.getClass(), name).set(target, value);
    }
    static Object getField(Object target, String name) throws Exception {
        return member(target.getClass(), name).get(target);
    }
    static Object shell(String name) throws Exception {
        Class<?> unsafe = Class.forName("sun.misc.Unsafe");
        Field f = unsafe.getDeclaredField("theUnsafe"); f.setAccessible(true);
        return unsafe.getMethod("allocateInstance", Class.class).invoke(f.get(null), type(name));
    }
    static Map<String, Object> map(Object... args) {
        Map<String, Object> out = new LinkedHashMap<>();
        for (int i = 0; i < args.length; i += 2) out.put((String) args[i], args[i + 1]);
        return out;
    }

    static Map<String, Object> lightCatalog() throws Exception {
        Object[] directions = type("core.Direction").getEnumConstants();
        List<Object> shapes = new ArrayList<>();
        Map<String, Integer> shapeIds = new LinkedHashMap<>();
        List<List<String>> shapeBoxes = new ArrayList<>();
        List<int[]> ranges = new ArrayList<>();
        Object support = supportWorld(), snow = state("SNOW"), snowPos = make("core.BlockPos", 0, 64, 0);
        int[] previous = null;
        for (Object state : statesById) {
            int id = stateIds.get(state);
            int[] row = new int[11]; row[0] = id; row[1] = id + 1;
            row[2] = (int) call(state, "getLightEmission");
            row[3] = (int) call(state, "getLightDampening");
            supportSoil = state;
            row[4] = (boolean) call(snow, "canSurvive", support, snowPos) ? 1 : 0;
            for (int i = 0; i < 6; i++) {
                Object shape = call(type("world.level.lighting.LightEngine"), "getOcclusionShape", state, directions[i]);
                List<String> boxes = new ArrayList<>();
                for (Object box : (List<?>) call(shape, "toAabbs")) {
                    List<String> coords = new ArrayList<>();
                    for (String n : List.of("minX", "minY", "minZ", "maxX", "maxY", "maxZ"))
                        coords.add(Double.toHexString((double) getField(box, n)));
                    boxes.add(String.join(",", coords));
                }
                Collections.sort(boxes);
                String key = String.join(";", boxes);
                Integer index = shapeIds.get(key);
                if (index == null) {
                    index = shapes.size(); shapeIds.put(key, index); shapes.add(shape); shapeBoxes.add(boxes);
                }
                row[5 + i] = index;
            }
            if (previous != null && Arrays.equals(previous, 2, 11, row, 2, 11)) previous[1]++;
            else { ranges.add(row); previous = row; }
        }
        List<List<Integer>> occludes = new ArrayList<>();
        for (Object first : shapes) {
            List<Integer> matches = new ArrayList<>();
            for (int j = 0; j < shapes.size(); j++) {
                if ((boolean) call(type("world.phys.shapes.Shapes"), "faceShapeOccludes", first, shapes.get(j))) matches.add(j);
            }
            occludes.add(matches);
        }
        Map<String, Object> climates = new TreeMap<>();
        for (Object biome : (Iterable<?>) biomeRegistry) {
            Object climate = getField(biome, "climateSettings");
            climates.put(call(biomeRegistry, "getKey", biome).toString(), map(
                "temperature_bits", String.format("%08x", Float.floatToRawIntBits((float) call(biome, "getBaseTemperature"))),
                "has_precipitation", call(biome, "hasPrecipitation"),
                "temperature_modifier", call(getField(climate, "temperatureModifier"), "getSerializedName")));
        }
        return map("state_count", statesById.size(), "directions", Arrays.stream(directions).map(Object::toString).toList(),
            "state_columns", List.of("start", "end_exclusive", "emission", "dampening", "snow_support", "down", "up", "north", "south", "west", "east"),
            "state_ranges", ranges, "shape_boxes_hex", shapeBoxes, "occludes", occludes, "climates", climates);
    }

    static final class NativeWorld {
        final Map<Pos, Object> chunks = new LinkedHashMap<>();
        final Object engine, world, region;
        NativeWorld(List<int[]> positions, boolean sky) throws Exception {
            for (int[] p : positions) chunks.put(new Pos(p[0], 0, p[1]), make("world.level.chunk.ProtoChunk",
                make("world.level.ChunkPos", p[0], p[1]), field("world.level.chunk.UpgradeData", "EMPTY"), heightAccessor, chunkFactory, null));
            world = Proxy.newProxyInstance(getClass().getClassLoader(), new Class<?>[]{type("world.level.BlockGetter")}, (p, m, a) -> switch (m.getName()) {
                case "getMinY" -> -64;
                case "getHeight" -> 384;
                case "getBlockState" -> block(a[0]);
                case "getFluidState" -> call(block(a[0]), "getFluidState");
                case "getBlockEntity" -> null;
                default -> { if (m.isDefault()) yield InvocationHandler.invokeDefault(p, m, a); throw new UnsupportedOperationException(m.toString()); }
            });
            Object getter = Proxy.newProxyInstance(getClass().getClassLoader(), new Class<?>[]{type("world.level.chunk.LightChunkGetter")}, (p, m, a) -> switch (m.getName()) {
                case "getLevel" -> world;
                case "getChunkForLighting" -> chunks.get(new Pos((int) a[0], 0, (int) a[1]));
                case "onLightUpdate" -> null;
                default -> throw new UnsupportedOperationException(m.toString());
            });
            engine = make("world.level.lighting.LevelLightEngine", getter, true, sky);
            // Real query-only WorldGenRegion/ServerLevel/ServerChunkCache objects.
            // Allocate unused server-service fields without launching server threads.
            // No queried method is overridden; fail-fast nulls expose missing setup.
            Object threaded = shell("server.level.ThreadedLevelLightEngine");
            for (Field f : type("world.level.lighting.LevelLightEngine").getDeclaredFields()) if (!Modifier.isStatic(f.getModifiers())) {
                f.setAccessible(true); f.set(threaded, f.get(engine));
            }
            Object chunkMap = shell("server.level.ChunkMap");
            set(chunkMap, "worldGenContext", make("world.level.chunk.status.WorldGenContext", null, generator, null, threaded, null, null));
            Object source = shell("server.level.ServerChunkCache"); set(source, "lightEngine", threaded); set(source, "chunkMap", chunkMap);
            Object level = shell("server.level.ServerLevel"); set(level, "chunkSource", source);
            region = shell("server.level.WorldGenRegion"); set(region, "level", level);
            set(region, "center", chunks.values().iterator().next());
            Object dimensions = call(registries, "lookupOrThrow", field("core.registries.Registries", "DIMENSION_TYPE"));
            set(level, "dimensionTypeRegistration", holder(dimensions, "overworld"));
            set(region, "dimensionType", call(holder(dimensions, "overworld"), "value"));
            set(region, "generatingStep", call(field("world.level.chunk.status.ChunkPyramid", "GENERATION_PYRAMID"), "getStepTo", field("world.level.chunk.status.ChunkStatus", "FEATURES")));
            Object initializer = Proxy.newProxyInstance(getClass().getClassLoader(), new Class<?>[]{type("util.StaticCache2D$Initializer")}, (p, m, a) -> {
                Object chunk = chunks.get(new Pos((int) a[0], 0, (int) a[1]));
                if (chunk == null) throw new IllegalStateException("missing fixture chunk");
                Object holder = shell("server.level.ChunkHolder"); set(holder, "pos", call(chunk, "getPos"));
                List<?> statuses = (List<?>) call(type("world.level.chunk.status.ChunkStatus"), "getStatusList");
                AtomicReferenceArray<Object> futures = new AtomicReferenceArray<>(statuses.size());
                Object complete = CompletableFuture.completedFuture(call(type("server.level.ChunkResult"), "of", chunk));
                for (int i = 0; i < 8; i++) futures.set(i, complete);
                set(holder, "futures", futures);
                return holder;
            });
            int radius = (int) Math.round((Math.sqrt(positions.size()) - 1) / 2);
            set(region, "cache", call(type("util.StaticCache2D"), "create", 0, 0, radius, initializer));
        }
        Object block(Object p) throws Exception {
            Pos at = position(p);
            Object chunk = chunks.get(new Pos(at.x() >> 4, 0, at.z() >> 4));
            if (chunk == null) throw new IllegalStateException("unavailable chunk for " + at);
            return call(chunk, "getBlockState", p);
        }
        void put(int x, int y, int z, Object state) throws Exception {
            Object chunk = chunks.get(new Pos(x >> 4, 0, z >> 4));
            if (chunk == null) throw new IllegalStateException("write outside fixture");
            call(chunk, "setBlockState", make("core.BlockPos", x, y, z), state, 2);
        }
        void initialize() throws Exception {
            for (Object chunk : chunks.values()) {
                call(chunk, "initializeLightSources");
                Object[] sections = (Object[]) call(chunk, "getSections");
                for (int i = 0; i < sections.length; i++) if (!(boolean) call(sections[i], "hasOnlyAir")) {
                    call(engine, "updateSectionStatus", call(type("core.SectionPos"), "of", call(chunk, "getPos"), i - 4), false);
                }
            }
            drain();
            for (Object chunk : chunks.values()) call(engine, "setLightEnabled", call(chunk, "getPos"), false);
        }
        void light() throws Exception {
            for (Object chunk : chunks.values()) call(engine, "propagateLightSources", call(chunk, "getPos"));
            drain();
        }
        void initializeOne(int x, int z) throws Exception {
            Object chunk = chunks.get(new Pos(x,0,z));
            call(chunk, "initializeLightSources");
            Object[] sections = (Object[]) call(chunk, "getSections");
            for (int i = 0; i < sections.length; i++) if (!(boolean) call(sections[i], "hasOnlyAir"))
                call(engine, "updateSectionStatus", call(type("core.SectionPos"), "of", call(chunk, "getPos"), i - 4), false);
            drain();
            call(engine, "setLightEnabled", call(chunk, "getPos"), false);
        }
        void drain() throws Exception {
            int count = 0;
            // ThreadedLevelLightEngine.runUpdate always publishes after its
            // pre-update tasks, even when direct sky fills enqueue no nodes.
            do {
                if (++count > 100) throw new IllegalStateException("light failed to settle");
                call(engine, "runLightUpdates");
            } while ((boolean) call(engine, "hasLightWork"));
        }
    }

    static List<int[]> positions(int radius) {
        List<int[]> out = new ArrayList<>();
        // Center first for WorldGenRegion; remaining order is explicitly captured.
        out.add(new int[]{0, 0});
        for (int z = -radius; z <= radius; z++) for (int x = -radius; x <= radius; x++)
            if (x != 0 || z != 0) out.add(new int[]{x, z});
        return out;
    }

    static Map<String, Object> environment() throws Exception {
        NativeWorld world = new NativeWorld(positions(1), true);
        Object pos = make("core.BlockPos", 8, 64, 8);
        List<Object> brightness = new ArrayList<>();
        world.put(8, 63, 8, state("STONE")); world.put(8, 80, 8, state("STONE")); world.put(9, 64, 8, state("GLOWSTONE"));
        for (String stage : List.of("FEATURES", "INITIALIZE_LIGHT", "LIGHT")) {
            if (stage.equals("INITIALIZE_LIGHT")) world.initialize();
            if (stage.equals("LIGHT")) world.light();
            for (int[] p : List.of(new int[]{8, -65, 8}, new int[]{8, -64, 8}, new int[]{8, 64, 8}, new int[]{8, 80, 8}, new int[]{8, 81, 8}, new int[]{8, 319, 8}, new int[]{8, 320, 8}, new int[]{30000000, 64, 0})) {
                Object at = make("core.BlockPos", p[0], p[1], p[2]);
                brightness.add(map("stage", stage, "pos", p,
                    "sky", call(world.region, "getBrightness", field("world.level.LightLayer", "SKY"), at),
                    "block", call(world.region, "getBrightness", field("world.level.LightLayer", "BLOCK"), at),
                    "raw", call(world.region, "getMaxLocalRawBrightness", at),
                    "sky_darken", call(world.region, "getSkyDarken")));
            }
        }
        NativeWorld fresh = new NativeWorld(positions(1), true);
        List<Object> samples = new ArrayList<>();
        for (String biomeName : List.of("plains", "snowy_plains", "frozen_ocean", "deep_frozen_ocean", "windswept_hills", "snowy_taiga", "desert")) {
            Object biome = call(holder(biomeRegistry, biomeName), "value");
            for (int[] p : List.of(new int[]{8, 64, 8}, new int[]{-7, 319, 12}, new int[]{0, 80, 0}, new int[]{0, 81, 0}, new int[]{15, -64, 15}, new int[]{15, -65, 15}, new int[]{15, 320, 15})) {
                Object at = make("core.BlockPos", p[0], p[1], p[2]);
                for (String blockName : List.of("WATER", "STONE", "AIR", "SNOW", "BROWN_MUSHROOM", "ICE", "OAK_SLAB")) {
                    Object block = state(blockName);
                    for (String belowName : List.of("STONE", "ICE", "HONEY_BLOCK", "SOUL_SAND", "MYCELIUM", "OAK_LEAVES", "AIR")) {
                        fresh.put(p[0], p[1], p[2], block); fresh.put(p[0], p[1] - 1, p[2], state(belowName));
                        samples.add(map("biome", biomeName, "pos", p, "state", stateIds.get(block), "below", stateIds.get(state(belowName)),
                            "temperature_bits", String.format("%08x", Float.floatToRawIntBits((float) call(biome, "getTemperature", at, 63))),
                            "freeze", call(biome, "shouldFreeze", fresh.region, at, false),
                            "freeze_edge", call(biome, "shouldFreeze", fresh.region, at, true),
                            "snow", call(biome, "shouldSnow", fresh.region, at),
                            "survives", call(block, "canSurvive", fresh.region, at)));
                    }
                }
            }
        }
        // A source surrounded by water is not an edge; flowing and waterlogged
        // fluid are not equivalent to the liquid block's source-water state.
        List<Object> water = new ArrayList<>();
        Object cold = call(holder(biomeRegistry, "snowy_plains"), "value");
        for (int id = stateIds.get(state("WATER")); id < stateIds.get(state("WATER")) + 16; id++) {
            fresh.put(8, 64, 8, statesById.get(id));
            for (boolean neighbors : List.of(false, true)) {
                for (int[] d : List.of(new int[]{-1,0}, new int[]{1,0}, new int[]{0,-1}, new int[]{0,1})) fresh.put(8 + d[0], 64, 8 + d[1], neighbors ? state("WATER") : air);
                water.add(map("state", id, "all_water_neighbors", neighbors,
                    "freeze", call(cold, "shouldFreeze", fresh.region, pos, false),
                    "freeze_edge", call(cold, "shouldFreeze", fresh.region, pos, true)));
            }
        }
        return map("setup", "Real WorldGenRegion and native ProtoChunks; query-only server service shells; native LevelLightEngine, default LevelReader methods, Biome and BlockState methods unchanged", "brightness", brightness, "samples", samples, "water", water);
    }

    static Object variant(String block, String... properties) throws Exception {
        Object result = state(block);
        for (int i = 0; i < properties.length; i += 2) {
            Object property = call(call(call(result, "getBlock"), "getStateDefinition"), "getProperty", properties[i]);
            Object value = ((Optional<?>) call(property, "getValue", properties[i + 1])).orElseThrow();
            result = call(result, "setValue", property, value);
        }
        return result;
    }

    static void box(NativeWorld world, List<int[]> boxes, Object state, int x0, int y0, int z0, int x1, int y1, int z1) throws Exception {
        boxes.add(new int[]{x0, y0, z0, x1, y1, z1, stateIds.get(state)});
        for (int y = y0; y <= y1; y++) for (int z = z0; z <= z1; z++) for (int x = x0; x <= x1; x++) world.put(x, y, z, state);
    }

    static Map<String, Object> snapshot(NativeWorld world, int cx, int cz) throws Exception {
        List<Object> layers = new ArrayList<>();
        Object sky = call(world.engine, "getLayerListener", field("world.level.LightLayer", "SKY"));
        Object block = call(world.engine, "getLayerListener", field("world.level.LightLayer", "BLOCK"));
        for (int y = -5; y <= 20; y++) {
            Object section = call(type("core.SectionPos"), "of", cx, y, cz);
            Object sd = call(sky, "getDataLayerData", section), bd = call(block, "getDataLayerData", section);
            boolean skyEmpty = sd != null && (boolean) call(sd, "isEmpty");
            boolean blockEmpty = bd != null && (boolean) call(bd, "isEmpty");
            // Observe like ClientboundLightUpdatePacketData: test isEmpty before
            // COPY/getData. Materializing a live lazy-zero layer changes its mask.
            layers.add(new Object[]{sd == null ? null : HexFormat.of().formatHex((byte[]) call(call(sd, "copy"), "getData")),
                bd == null ? null : HexFormat.of().formatHex((byte[]) call(call(bd, "copy"), "getData")), skyEmpty, blockEmpty});
        }
        Object chunk = world.chunks.get(new Pos(cx, 0, cz));
        int[] heights = new int[256];
        if (chunk != null) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++)
            heights[z * 16 + x] = (int) call(call(chunk, "getSkyLightSources"), "getLowestSourceY", x, z);
        List<int[]> samples = new ArrayList<>();
        for (int y : new int[]{-100, -80, -65, -64, -63, -49, -48, -1, 0, 1, 15, 16, 31, 32, 47, 48, 63, 64, 65, 79, 80, 81, 95, 96, 191, 192, 199, 200, 207, 208, 318, 319, 320, 335, 336, 400}) {
            for (int x : new int[]{0, 1, 7, 8, 14, 15}) for (int z : new int[]{0, 7, 8, 15}) {
                Object pos = make("core.BlockPos", cx * 16 + x, y, cz * 16 + z);
                samples.add(new int[]{x, y, z, (int) call(sky, "getLightValue", pos), (int) call(block, "getLightValue", pos)});
            }
        }
        return map("pos", new int[]{cx, cz}, "min_section_y", -5, "sections", layers, "sources", heights, "samples", samples);
    }

    static Map<String, Object> lightCase(String name) throws Exception {
        int radius = name.equals("empty_single") || name.equals("single_border") ? 0 : 1;
        boolean sky = !name.equals("no_sky");
        List<int[]> positions = positions(radius);
        NativeWorld world = new NativeWorld(positions, sky);
        List<int[]> boxes = new ArrayList<>();
        if (name.startsWith("boundary_")) {
            String[] p = name.split("_");
            int y = Integer.parseInt(p[1]);
            Object block = switch (p[2]) {
                case "slab" -> variant("STONE_SLAB", "type", "bottom");
                case "stairs" -> variant("OAK_STAIRS", "half", "bottom", "facing", "north");
                case "snow" -> variant("SNOW", "layers", "1");
                default -> throw new IllegalArgumentException(name);
            };
            box(world, boxes, block, 8,y,8,8,y,8);
            if (p[3].equals("occupied") && y > -64) box(world,boxes,state("STONE"),0,y-1,0,0,y-1,0);
        } else switch (name) {
            case "empty_single", "empty_neighborhood" -> { }
            case "single_border" -> {
                box(world, boxes, state("GLOWSTONE"), 15, 64, 15, 15, 64, 15);
                box(world, boxes, state("STONE"), 0, -64, 0, 15, -64, 15);
            }
            case "floor_cave", "no_sky" -> {
                box(world, boxes, state("STONE"), -16, -64, -16, 31, 80, 31);
                box(world, boxes, air, -15, -60, -15, 30, 78, 30);
                box(world, boxes, air, 7, 79, 7, 8, 80, 8);
                box(world, boxes, state("TORCH"), 15, 64, 0, 15, 64, 0);
                box(world, boxes, state("LAVA"), 16, -59, 0, 16, -59, 0);
            }
            case "suspended_roof" -> {
                box(world, boxes, state("STONE"), -16, 200, -16, 31, 200, 31);
                box(world, boxes, air, 7, 200, 7, 8, 200, 8);
                box(world, boxes, state("GLOWSTONE"), 16, 0, 8, 16, 0, 8);
            }
            case "vertical_limits" -> {
                box(world, boxes, state("STONE"), -16, -64, -16, 31, -64, 31);
                box(world, boxes, state("STONE"), -16, 319, -16, 31, 319, 31);
                box(world, boxes, air, 15, 319, 7, 16, 319, 8);
                box(world, boxes, state("GLOWSTONE"), 0, -64, 0, 0, -64, 0);
                box(world, boxes, state("GLOWSTONE"), 15, 319, 15, 15, 319, 15);
            }
            case "attenuation" -> {
                List<Object> materials = List.of(state("WATER"), state("ICE"), state("OAK_LEAVES"), state("GLASS"), state("TINTED_GLASS"), state("COBWEB"), state("SLIME_BLOCK"), state("HONEY_BLOCK"));
                for (int i = 0; i < materials.size(); i++) {
                    int x = -16 + i * 6;
                    box(world, boxes, materials.get(i), x, 61, -5, x + 4, 66, 5);
                    box(world, boxes, state("GLOWSTONE"), x + 2, 63, 0, x + 2, 63, 0);
                }
            }
            case "shapes" -> {
                List<Object> materials = List.of(variant("STONE_SLAB", "type", "top"), variant("STONE_SLAB", "type", "bottom"), variant("OAK_STAIRS", "half", "top", "facing", "east"), variant("OAK_STAIRS", "half", "bottom", "facing", "west"), variant("SNOW", "layers", "1"), variant("SNOW", "layers", "8"), variant("PISTON", "extended", "true", "facing", "up"), variant("OAK_TRAPDOOR", "half", "top"));
                for (int i = 0; i < materials.size(); i++) {
                    int x = -16 + i * 6;
                    box(world, boxes, materials.get(i), x, 80, -7, x + 4, 80, 7);
                    box(world, boxes, materials.get((i + 1) % materials.size()), x, 79, -7, x + 4, 79, 7);
                    box(world, boxes, state("GLOWSTONE"), x + 2, 78, 0, x + 2, 78, 0);
                }
            }
            case "all_emissions" -> {
                for (int i = 0; i < 16; i++) {
                    int x = (i % 4) * 12 - 14, z = (i / 4) * 12 - 14;
                    box(world, boxes, variant("LIGHT", "level", Integer.toString(i)), x, 64, z, x, 64, z);
                }
            }
            case "complementary_faces" -> {
                box(world, boxes, state("STONE"), -16, 64, -16, 31, 80, 31);
                box(world, boxes, air, -16, 65, 0, 31, 79, 0);
                box(world, boxes, state("GLOWSTONE"), 0, 65, 0, 0, 65, 0);
                for (int x = 1; x < 16; x++) box(world, boxes, variant("STONE_SLAB", "type", x % 2 == 0 ? "top" : "bottom"), x, 65, 0, x, 65, 0);
                box(world, boxes, air, 31, 80, 0, 31, 80, 0);
            }
            default -> throw new IllegalArgumentException(name);
        }
        world.initialize();
        Object initialized = snapshot(world, 0, 0);
        call(world.engine, "propagateLightSources", make("world.level.ChunkPos", 0, 0)); world.drain();
        Object centerOnly = snapshot(world, 0, 0);
        world.light();
        return map("name", name, "has_sky", sky, "chunks", positions, "boxes", boxes,
            "initialized", initialized, "center_only", centerOnly, "all_lit", snapshot(world, 0, 0),
            "east", snapshot(world, 1, 0));
    }

    static Object lighting() throws Exception {
        List<Object> cases = new ArrayList<>();
        for (String name : List.of("empty_single", "empty_neighborhood", "single_border", "floor_cave", "no_sky", "suspended_roof", "vertical_limits", "attenuation", "shapes", "all_emissions", "complementary_faces")) cases.add(lightCase(name));
        return map("setup", "Native ProtoChunk.initializeLightSources, LevelLightEngine.updateSectionStatus/runLightUpdates/setLightEnabled(false), then propagateLightSources/runLightUpdates; explicit column order and phase snapshots", "cases", cases);
    }

    static Object histories() throws Exception {
        List<Object> cases = new ArrayList<>();
        for (String name : List.of("inactive_open_neighbors", "late_initialization", "sparse_sections")) {
            int radius = name.equals("late_initialization") ? 2 : 1;
            NativeWorld world = new NativeWorld(positions(radius), true);
            List<int[]> boxes = new ArrayList<>();
            if (name.equals("inactive_open_neighbors")) {
                box(world, boxes, state("STONE"), -16,-64,-16,31,-64,31);
            } else if (name.equals("late_initialization")) {
                box(world, boxes, state("STONE"), -32,-64,-32,47,0,47);
                box(world, boxes, state("STONE"), 32,200,-16,47,200,31);
                box(world, boxes, state("GLOWSTONE"), 15,1,8,15,1,8);
                box(world, boxes, state("GLOWSTONE"), 32,192,8,32,192,8);
            } else {
                box(world, boxes, state("STONE"), -16,200,-16,31,200,31);
                box(world, boxes, air, -1,200,7,0,200,8);
                box(world, boxes, state("STONE"), 16,64,8,16,64,8);
                box(world, boxes, state("STONE"), -16,0,8,-16,0,8);
                box(world, boxes, state("TORCH"), 16,65,8,16,65,8);
            }
            List<Object> steps = new ArrayList<>();
            List<int[]> inner = positions(1);
            for (int[] p : inner) world.initializeOne(p[0],p[1]);
            steps.add(map("initialize", inner, "snapshots", List.of(snapshot(world,0,0), snapshot(world,1,0))));
            call(world.engine, "propagateLightSources", make("world.level.ChunkPos",0,0)); world.drain();
            steps.add(map("light", new int[]{0,0}, "snapshots", List.of(snapshot(world,0,0), snapshot(world,1,0))));
            if (radius == 2) {
                List<int[]> outer = List.of(new int[]{2,-1},new int[]{2,0},new int[]{2,1});
                for (int[] p : outer) world.initializeOne(p[0],p[1]);
                steps.add(map("initialize",outer,"snapshots",List.of(snapshot(world,0,0),snapshot(world,1,0),snapshot(world,2,0))));
            }
            call(world.engine, "propagateLightSources", make("world.level.ChunkPos",1,0)); world.drain();
            steps.add(map("light",new int[]{1,0},"snapshots",List.of(snapshot(world,0,0),snapshot(world,1,0))));
            cases.add(map("name",name,"chunks",positions(radius),"boxes",boxes,"steps",steps));
        }
        return map("setup", "FEATURES-readable native ProtoChunks before light initialization; explicit initialize/propagate/drain histories; inactive neighbor layers retained", "cases", cases);
    }

    static Object boundaries() throws Exception {
        List<Object> cases = new ArrayList<>();
        for (int y : new int[]{-64,-48,0,16,64,80,304,319}) {
            for (String occupancy : List.of("sparse","occupied")) cases.add(lightCase("boundary_"+y+"_slab_"+occupancy));
        }
        for (String material : List.of("stairs","snow")) {
            for (String occupancy : List.of("sparse","occupied")) cases.add(lightCase("boundary_64_"+material+"_"+occupancy));
        }
        return map("setup", "Native section-boundary partial shapes, with/without unrelated occupied cell in section below; outputs include real ChunkSkyLightSources.fillFrom skipping of wholly empty sections", "cases", cases);
    }

    static Object publicationProof() throws Exception {
        NativeWorld world = new NativeWorld(positions(1),true);
        world.put(8,64,8,variant("STONE_SLAB","type","bottom"));
        world.initialize();
        Object pos = make("core.BlockPos",8,65,8);
        Object listener = call(world.engine,"getLayerListener",field("world.level.LightLayer","SKY"));
        call(world.engine,"propagateLightSources",make("world.level.ChunkPos",0,0));
        Object pending = call(world.engine,"hasLightWork");
        Object before = call(listener,"getLightValue",pos);
        Object processed = call(world.engine,"runLightUpdates");
        return map("setup","Unchanged native methods; ThreadedLevelLightEngine.runUpdate bytecode invokes super.runLightUpdates unconditionally at offset 92, even with empty queues", "has_light_work_before_publish",pending,
            "visible_sky_before_publish",before,"processed_queue_entries",processed,"visible_sky_after_publish",call(listener,"getLightValue",pos));
    }

    static Object updates() throws Exception {
        List<int[]> ranges = new ArrayList<>();
        for (Object state : statesById) {
            int id = stateIds.get(state);
            int flags = (boolean) call(state, "useShapeForLightOcclusion") ? 1 : 0;
            if ((boolean) call(type("world.level.lighting.LightEngine"), "isEmptyShape", state)) flags |= 2;
            if (!ranges.isEmpty() && ranges.get(ranges.size() - 1)[2] == flags) ranges.get(ranges.size() - 1)[1]++;
            else ranges.add(new int[]{id, id + 1, flags});
        }
        List<Object> cases = new ArrayList<>();
        for (String name : List.of("unlit_neighbor", "lit_emission", "lit_roof", "sparse_boundary", "no_sky", "net_zero")) {
            NativeWorld world = new NativeWorld(positions(1), !name.equals("no_sky"));
            List<int[]> boxes = new ArrayList<>();
            List<List<int[]>> batches = new ArrayList<>();
            int airId = stateIds.get(air), stone = stateIds.get(state("STONE")), glow = stateIds.get(state("GLOWSTONE"));
            int slab = stateIds.get(variant("STONE_SLAB", "type", "bottom"));
            if (!name.equals("sparse_boundary")) box(world, boxes, state("STONE"), -16, -64, -16, 31, 63, 31);
            if (name.equals("unlit_neighbor")) {
                box(world, boxes, state("GLOWSTONE"), 15, 64, 0, 15, 64, 0);
                batches.add(List.of(new int[]{16,64,0,stone}, new int[]{16,65,1,glow}));
                batches.add(List.of(new int[]{16,64,0,airId}, new int[]{16,65,1,airId}));
            } else if (name.equals("lit_emission") || name.equals("no_sky")) {
                box(world, boxes, state("GLOWSTONE"), 8, 64, 8, 8, 64, 8);
                batches.add(List.of(new int[]{8,64,8,airId}));
                batches.add(List.of(new int[]{15,64,8,glow}, new int[]{16,64,8,glow}));
                batches.add(List.of(new int[]{15,64,8,stone}, new int[]{16,64,8,airId}));
                batches.add(List.of(new int[]{15,64,8,stateIds.get(state("COAL_ORE"))}));
            } else if (name.equals("lit_roof")) {
                batches.add(List.of(new int[]{15,80,7,stone}, new int[]{16,80,7,stone}, new int[]{15,80,8,stone}, new int[]{16,80,8,stone}));
                batches.add(List.of(new int[]{15,80,7,airId}, new int[]{16,80,7,airId}, new int[]{15,80,8,airId}, new int[]{16,80,8,airId}));
            } else if (name.equals("sparse_boundary")) {
                batches.add(List.of(new int[]{8,64,8,slab}));
                batches.add(List.of(new int[]{8,64,8,airId}));
                batches.add(List.of(new int[]{8,-64,8,slab}, new int[]{15,319,8,glow}));
                batches.add(List.of(new int[]{8,-64,8,airId}, new int[]{15,319,8,airId}));
            } else {
                batches.add(List.of(new int[]{8,96,8,glow}, new int[]{8,96,8,airId}));
                batches.add(List.of(new int[]{8,64,8,stone}, new int[]{8,64,8,slab}, new int[]{8,64,8,airId}));
            }
            world.initialize();
            for (Object chunk : world.chunks.values()) {
                call(chunk, "setLightEngine", world.engine);
                call(chunk, "setPersistedStatus", field("world.level.chunk.status.ChunkStatus", "INITIALIZE_LIGHT"));
            }
            List<int[]> enabled = name.equals("unlit_neighbor") ? List.of(new int[]{0,0}) : positions(1);
            for (int[] p : enabled) { call(world.engine, "propagateLightSources", make("world.level.ChunkPos",p[0],p[1])); world.drain(); }
            Object before = List.of(snapshot(world,0,0),snapshot(world,1,0));
            List<Object> steps = new ArrayList<>();
            for (List<int[]> batch : batches) {
                List<Object> checks = new ArrayList<>();
                for (int[] p : batch) {
                    Object pos = make("core.BlockPos",p[0],p[1],p[2]);
                    Object old = world.block(pos), next = statesById.get(p[3]);
                    checks.add(map("pos", new int[]{p[0],p[1],p[2]}, "before", stateIds.get(old), "after", p[3],
                        "check", call(type("world.level.lighting.LightEngine"), "hasDifferentLightProperties", old, next)));
                    world.put(p[0],p[1],p[2],next);
                }
                world.drain();
                steps.add(map("edits",batch,"checks",checks,"snapshots",List.of(snapshot(world,0,0),snapshot(world,1,0))));
            }
            if (name.equals("unlit_neighbor")) {
                call(world.engine, "propagateLightSources", make("world.level.ChunkPos",1,0)); world.drain();
                steps.add(map("light",new int[]{1,0},"snapshots",List.of(snapshot(world,0,0),snapshot(world,1,0))));
            }
            cases.add(map("name",name,"has_sky",!name.equals("no_sky"),"chunks",positions(1),"boxes",boxes,
                "enabled",enabled,"before",before,"steps",steps));
        }
        return map("setup","Native ProtoChunk.setBlockState after INITIALIZE_LIGHT invokes section status, incremental sky sources and checkBlock. Each explicit batch is followed by runLightUpdates; snapshots never materialize live layers.",
            "state_count",statesById.size(),"update_state_ranges",ranges,"cases",cases);
    }

    public static void main(String[] args) throws Exception {
        bootstrap(args[1]);
        Object output = switch (args[0]) {
            case "catalog" -> lightCatalog();
            case "environment" -> environment();
            case "light" -> lighting();
            case "history" -> histories();
            case "boundaries" -> boundaries();
            case "publish" -> publicationProof();
            case "updates" -> updates();
            default -> throw new IllegalArgumentException(args[0]);
        };
        System.out.println("LIGHTING_REFERENCE=" + call(gson, "toJson", output));
    }
}
