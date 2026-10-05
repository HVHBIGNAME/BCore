import java.lang.reflect.*;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Function;

/** Calls the pinned server's sculk features and worldgen spreader on native ProtoChunks.
 * WorldGenRegion.setBlock, its write-radius check, pending block entities, and
 * postprocessing are native. The proxy records calls; it does not implement sculk.
 */
public class SculkReference extends TreeReference {
    static final String REGION = "server.level.WorldGenRegion";
    static final String STATUS = "world.level.chunk.status.ChunkStatus";
    static final List<String> CONFIGS = List.of("sculk_patch_deep_dark", "sculk_patch_ancient_city", "sculk_vein");
    static final Map<String, String> WORLDGEN_DYNAMIC_SHAPES = Map.of(
        "BambooStalkBlock", "Explicit non-full collision; translated 3-pixel-wide collision column has no full support face",
        "PointedDripstoneBlock", "Explicit non-full collision; all translated collision/support columns are at most 12 pixels wide",
        "ScaffoldingBlock", "CollisionContext.empty selects SHAPE_STABLE; support is independent of neighbouring blocks",
        "PowderSnowBlock", "CollisionContext.empty has no entity; collision/support is empty");
    static final Comparator<Pos> POS_ORDER = Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z);
    static Object registries, jsonOps, gson, height, factory, serverLevel, dimension;
    static Object[] directions;
    static Method getId, sectionSet;
    static final Map<String, Object> states = new HashMap<>();
    static final Map<Object, Object> factories = new IdentityHashMap<>();
    static final Map<String, Object> configured = new TreeMap<>();
    static final Map<String, Object> placed = new TreeMap<>(), biomes = new TreeMap<>();
    static Object noiseSettings;

    static int id(Object state) throws Exception { return (int) getId.invoke(null, state); }
    static Object blockState(String name) throws Exception {
        Object value = states.get(name);
        if (value == null) { value = state(name); states.put(name, value); }
        return value;
    }
    static Object pos(Pos p) throws Exception { return make("core.BlockPos", p.x(), p.y(), p.z()); }
    static List<Integer> xyz(Pos p) { return List.of(p.x(), p.y(), p.z()); }
    static Object member(Object object, String name) throws Exception {
        for (Class<?> c = object.getClass(); c != null; c = c.getSuperclass()) {
            try { Field f = c.getDeclaredField(name); f.setAccessible(true); return f.get(object); }
            catch (NoSuchFieldException ignored) { }
        }
        throw new NoSuchFieldException(name);
    }
    static Object document(String category, String name) throws Exception {
        String path = "/data/minecraft/" + category + "/" + name + ".json";
        try (var stream = SculkReference.class.getResourceAsStream(path)) {
            if (stream == null) throw new IllegalArgumentException("missing " + path);
            return call(Class.forName("com.google.gson.JsonParser"), "parseString", new String(stream.readAllBytes(), StandardCharsets.UTF_8));
        }
    }
    static Object jsonNbt(Object tag) throws Exception {
        return call(field("nbt.NbtOps", "INSTANCE"), "convertTo", jsonOps, tag);
    }
    static Object typedNbt(Object tag) throws Exception {
        int type = ((Number) call(tag, "getId")).intValue();
        Object value;
        if (type == 10) {
            Map<String, Object> entries = new TreeMap<>();
            for (Object entry : (Set<?>) call(tag, "entrySet")) {
                Map.Entry<?, ?> e = (Map.Entry<?, ?>) entry;
                entries.put((String) e.getKey(), typedNbt(e.getValue()));
            }
            value = entries;
        } else if (type == 9) {
            List<Object> entries = new ArrayList<>();
            for (Object entry : (List<?>) tag) entries.add(typedNbt(entry));
            value = entries;
        } else {
            value = switch (type) {
                case 1, 2, 3, 4, 5, 6 -> call(tag, "box");
                case 8 -> call(tag, "value");
                case 11 -> call(tag, "getAsIntArray");
                default -> throw new IllegalArgumentException("unexpected NBT " + type);
            };
        }
        return List.of(type, value);
    }

    static Object factoryFor(Object state) throws Exception {
        Object result = factories.get(state);
        if (result == null) {
            result = make("world.level.chunk.PalettedContainerFactory", member(factory, "blockStatesStrategy"), state,
                member(factory, "blockStatesContainerCodec"), member(factory, "biomeStrategy"), member(factory, "defaultBiome"), member(factory, "biomeContainerCodec"));
            factories.put(state, result);
        }
        return result;
    }

    record Scenario(String name, String terrain, Pos origin, int writeRadius, int advance, int repetitions) { }

    static class World {
        final Scenario scenario;
        final Map<Pos, Object> initialBlocks;
        final Object region, proxy;
        final Map<Pos, Object> chunks = new HashMap<>();
        final Set<Pos> emptySections = new HashSet<>();
        final Set<Pos> written = new TreeSet<>(POS_ORDER);
        final List<int[]> writes = new ArrayList<>();
        final List<Object> tickRequests = new ArrayList<>();
        final List<int[]> featureOrigins = new ArrayList<>();
        Object biome;
        final Object airState, stone, deepslate, water, flowingWater, lava, caveAir, bedrock, sculk;
        final Object[] obstacleStates;

        World(Scenario scenario) throws Exception { this(scenario, Map.of()); }

        World(Scenario scenario, Map<Pos, Object> initialBlocks) throws Exception {
            this.scenario = scenario;
            this.initialBlocks = initialBlocks;
            airState = blockState("AIR"); stone = blockState("STONE"); deepslate = blockState("DEEPSLATE");
            water = blockState("WATER"); lava = blockState("LAVA"); caveAir = blockState("CAVE_AIR");
            flowingWater = call(water, "setValue", field("world.level.block.LiquidBlock", "LEVEL"), 1);
            bedrock = blockState("BEDROCK"); sculk = blockState("SCULK");
            obstacleStates = new Object[]{blockState("STONE"), blockState("DEEPSLATE"), blockState("TUFF"),
                blockState("CALCITE"), blockState("GRAVEL"), blockState("DIRT"), blockState("CLAY"), blockState("BEDROCK"),
                blockState("GLASS"), blockState("OAK_SLAB"), blockState("FIRE"), blockState("SHORT_GRASS"),
                blockState("DEEPSLATE_BRICKS"), blockState("DEEPSLATE_TILES"), blockState("COBBLED_DEEPSLATE"),
                blockState("CRACKED_DEEPSLATE_BRICKS"), blockState("CRACKED_DEEPSLATE_TILES"), blockState("POLISHED_DEEPSLATE"),
                blockState("POINTED_DRIPSTONE"), blockState("BAMBOO"), blockState("SCAFFOLDING"), blockState("POWDER_SNOW")};
            Pos origin = scenario.origin();
            Object carvers = field(STATUS, "CARVERS");
            Object initializer = Proxy.newProxyInstance(SculkReference.class.getClassLoader(), new Class<?>[]{type("util.StaticCache2D$Initializer")}, (p, m, a) -> {
                if (!m.getName().equals("get")) throw new UnsupportedOperationException(m.toString());
                int cx = (int) a[0], cz = (int) a[1];
                Object chunk = createChunk(cx, cz);
                call(chunk, "setPersistedStatus", carvers);
                Object owner = call(chunk, "getPos");
                Object holder = make("server.level.ChunkHolder", owner, 0, height, null, null, null);
                call(holder, "completeFuture", carvers, chunk);
                chunks.put(new Pos(cx, 0, cz), chunk);
                return holder;
            });
            Object cache = call(type("util.StaticCache2D"), "create", origin.x() >> 4, origin.z() >> 4, 2, initializer);
            Object nativeStep = call(field("world.level.chunk.status.ChunkPyramid", "GENERATION_PYRAMID"), "getStepTo", field(STATUS, "FEATURES"));
            Object dependencies = make("world.level.chunk.status.ChunkDependencies", call(Class.forName("com.google.common.collect.ImmutableList"), "copyOf", List.of(carvers, carvers, carvers)));
            Object step = make("world.level.chunk.status.ChunkStep", field(STATUS, "FEATURES"), dependencies,
                call(nativeStep, "accumulatedDependencies"), scenario.writeRadius(), call(nativeStep, "task"));
            region = TreeEffectReference.allocate(type(REGION));
            for (var e : Map.of("cache", cache, "center", chunks.get(new Pos(origin.x() >> 4, 0, origin.z() >> 4)),
                    "level", serverLevel, "generatingStep", step, "subTickCount", new AtomicLong(), "dimensionType", dimension).entrySet())
                TreeEffectReference.setField(region, REGION, e.getKey(), e.getValue());
            Object levelData = Proxy.newProxyInstance(SculkReference.class.getClassLoader(), new Class<?>[]{type("world.level.storage.LevelData")}, (p, m, a) -> {
                if (m.getName().equals("getGameTime")) return 100L;
                throw new UnsupportedOperationException(m.toString());
            });
            TreeEffectReference.setField(region, REGION, "levelData", levelData);
            for (String kind : List.of("Block", "Fluid")) {
                Function<Object, Object> getter = p -> {
                    try { return call(call(region, "getChunk", p), "get" + kind + "Ticks"); }
                    catch (Exception e) { throw new RuntimeException(e); }
                };
                TreeEffectReference.setField(region, REGION, kind.toLowerCase(Locale.ROOT) + "Ticks", make("world.ticks.WorldGenTickAccess", getter));
            }
            proxy = Proxy.newProxyInstance(SculkReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
                if (m.getName().equals("getBiome")) {
                    if (biome == null) throw new IllegalStateException("unexpected configured sculk biome query");
                    return biome;
                }
                if (m.getName().equals("ensureCanWrite")) {
                    Pos location = Pos.from(a[0]);
                    featureOrigins.add(new int[]{location.x(), location.y(), location.z()});
                }
                if (m.getName().equals("setBlock")) {
                    Pos location = Pos.from(a[0]);
                    boolean accepted = (boolean) call(region, "setBlock", a[0], a[1], a[2], a.length == 4 ? a[3] : 512);
                    writes.add(new int[]{location.x(), location.y(), location.z(), id(a[1]), (int) a[2], accepted ? 1 : 0});
                    written.add(location);
                    return accepted;
                }
                if (m.getName().equals("scheduleTick")) {
                    boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                    int target = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]) : id(call(a[1], "defaultBlockState"));
                    tickRequests.add(Map.of("pos", xyz(Pos.from(a[0])), "id", target, "fluid", fluid, "delay", a[2]));
                }
                try { return m.invoke(region, a); }
                catch (InvocationTargetException e) { throw e.getCause(); }
            });
            for (int dy = -1; dy <= 1; dy++) {
                Pos p = new Pos(origin.x(), origin.y() + dy, origin.z());
                if (p.y() >= -64 && p.y() <= 319 && call(region, "getBlockState", pos(p)) != initialView(p)) {
                    Object chunk = call(region, "getChunk", pos(p));
                    Object section = call(chunk, "getSection", (p.y() + 64) >> 4);
                    throw new IllegalStateException("native chunk initialization differs at " + p + ": got " + call(region, "getBlockState", pos(p))
                        + ", expected " + initial(p.x(), p.y(), p.z()) + ", minY=" + call(chunk, "getMinY") + ", height=" + call(chunk, "getHeight")
                        + ", sectionState=" + call(section, "getBlockState", p.x() & 15, p.y() & 15, p.z() & 15) + ", empty=" + call(section, "hasOnlyAir"));
                }
            }
        }

        Object initial(int x, int y, int z) throws Exception {
            if (y < -64 || y > 319) return blockState("VOID_AIR");
            Object overridden = initialBlocks.get(new Pos(x, y, z));
            if (overridden != null) return overridden;
            Pos o = scenario.origin();
            int dx = x - o.x(), dy = y - o.y(), dz = z - o.z();
            String terrain = scenario.terrain();
            if (terrain.equals("layers")) return Math.floorMod(y, 8) < 2 ? (y < 0 ? deepslate : stone) : caveAir;
            if (terrain.equals("air")) return airState;
            if (terrain.equals("solid")) return deepslate;
            if (terrain.equals("sculk_floor")) return dy < 0 ? sculk : airState;
            if (terrain.equals("sculk_water")) return dy < 0 ? sculk : dy < 4 ? water : airState;
            if (terrain.equals("origin_sculk") && dy == 0 && dx == 0 && dz == 0) return sculk;
            if (terrain.equals("origin_vein") && dy == 0 && dx == 0 && dz == 0)
                return call(blockState("SCULK_VEIN"), "setValue", call(type("world.level.block.MultifaceBlock"), "getFaceProperty", directions[0]), true);
            if (terrain.equals("ceiling")) return dy > 0 ? deepslate : caveAir;
            if (terrain.equals("walls")) return Math.abs(dx) >= 3 || Math.abs(dz) >= 4 ? deepslate : caveAir;
            if (terrain.equals("search") && dx == 2 && dy == 0 && dz == 0) return stone;
            if (terrain.equals("search")) return airState;
            if (terrain.equals("cave") || terrain.equals("mixed")) {
                if (dy >= 6 || (Math.abs(dx - 4) <= 1 && Math.abs(dz - 3) <= 1)) return deepslate;
            }
            if (dy < 0) {
                if (terrain.equals("bedrock")) return bedrock;
                if (terrain.equals("mixed") && dy == -1) return obstacleStates[Math.floorMod(x * 3 + z * 5, 12)];
                if (terrain.equals("city")) return obstacleStates[12 + Math.floorMod(x + z * 3, 6)];
                if (terrain.equals("dynamic") && dy == -1) return dx == 0 && dz == 0 ? deepslate : obstacleStates[18 + Math.floorMod(x + z, 4)];
                return y < 0 ? deepslate : stone;
            }
            if (terrain.equals("water") && dy < 4) return water;
            if (terrain.equals("lava") && dy < 4) return lava;
            if (terrain.equals("flowing_water") && dy < 4) return flowingWater;
            return terrain.equals("cave") || terrain.equals("mixed") ? caveAir : airState;
        }

        Object initialView(Pos p) throws Exception {
            return emptySections.contains(new Pos(p.x() >> 4, p.y() >> 4, p.z() >> 4)) ? airState : initial(p.x(), p.y(), p.z());
        }

        Object createChunk(int cx, int cz) throws Exception {
            Object sections = Array.newInstance(type("world.level.chunk.LevelChunkSection"), 24);
            for (int section = 0; section < 24; section++) {
                int bottom = -64 + section * 16;
                Object defaultState = initial(cx * 16, bottom, cz * 16);
                Object data = make("world.level.chunk.LevelChunkSection", factoryFor(defaultState));
                for (int y = 0; y < 16; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
                    Object value = initial(cx * 16 + x, bottom + y, cz * 16 + z);
                    if (value != defaultState) sectionSet.invoke(data, x, y, z, value, false);
                }
                call(data, "recalcBlockCounts");
                if ((boolean) call(data, "hasOnlyAir")) emptySections.add(new Pos(cx, bottom >> 4, cz));
                Array.set(sections, section, data);
            }
            return make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", cx, cz), field("world.level.chunk.UpgradeData", "EMPTY"),
                sections, make("world.ticks.ProtoChunkTicks"), make("world.ticks.ProtoChunkTicks"), height, factory, null);
        }

        Map<String, Object> result(boolean trace) throws Exception {
            List<Object> finalWrites = new ArrayList<>(), entities = new ArrayList<>(), ticks = new ArrayList<>();
            Map<String, Integer> counts = new TreeMap<>();
            for (Pos p : written) {
                Object location = pos(p), state = call(region, "getBlockState", location);
                if (id(state) != id(initialView(p))) {
                    finalWrites.add(List.of(p.x(), p.y(), p.z(), id(state)));
                    String block = call(field("core.registries.BuiltInRegistries", "BLOCK"), "getKey", call(state, "getBlock")).toString();
                    counts.merge(block, 1, Integer::sum);
                }
                if ((boolean) call(state, "hasBlockEntity")) {
                    Object chunk = call(region, "getChunk", location);
                    Object pending = call(chunk, "getBlockEntityNbt", location);
                    Object entity = call(region, "getBlockEntity", location);
                    if (entity == null) throw new IllegalStateException("missing generated block entity at " + p);
                    Map<String, Object> row = new LinkedHashMap<>();
                    row.put("pos", xyz(p)); row.put("state", id(state));
                    row.put("pending_nbt", pending == null ? null : jsonNbt(pending));
                    row.put("type_id", call(field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE"), "getId", call(entity, "getType")));
                    row.put("nbt", jsonNbt(call(entity, "saveWithFullMetadata", registries)));
                    row.put("update", jsonNbt(call(entity, "getUpdateTag", registries)));
                    entities.add(row);
                }
            }
            Map<Pos, Integer> marks = new TreeMap<>(POS_ORDER);
            Method unpack = type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates", short.class, int.class, type("world.level.ChunkPos"));
            List<Pos> owners = new ArrayList<>(chunks.keySet()); owners.sort(POS_ORDER);
            List<int[]> orderedMarks = new ArrayList<>();
            for (Pos owner : owners) {
                Object chunk = chunks.get(owner);
                Object[] sections = (Object[]) call(chunk, "getPostProcessing");
                for (int s = 0; s < sections.length; s++) if (sections[s] != null) {
                    for (Object packed : (Iterable<?>) sections[s]) {
                        Pos p = Pos.from(unpack.invoke(null, packed, s - 4, call(chunk, "getPos")));
                        marks.merge(p, 1, Integer::sum);
                        orderedMarks.add(new int[]{p.x(), p.y(), p.z()});
                    }
                }
                for (String kind : List.of("Block", "Fluid")) {
                    for (Object tick : (List<?>) call(call(chunk, "get" + kind + "Ticks"), "scheduledTicks")) {
                        ticks.add(Map.of("pos", xyz(Pos.from(call(tick, "pos"))), "kind", kind, "delay", call(tick, "delay")));
                    }
                }
            }
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("writes", finalWrites); result.put("counts", counts);
            result.put("write_calls", writes.size()); result.put("write_trace_sha256", hash(writes));
            result.put("marks", marks.entrySet().stream().map(e -> List.of(e.getKey().x(), e.getKey().y(), e.getKey().z(), e.getValue())).toList());
            result.put("mark_calls", orderedMarks.size()); result.put("section_marks_sha256", hash(orderedMarks));
            result.put("tick_requests", tickRequests); result.put("native_ticks", ticks); result.put("block_entities", entities);
            if (trace) result.put("write_trace", writes);
            return result;
        }
    }

    static String hash(List<int[]> rows) throws Exception {
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        for (int[] row : rows) {
            ByteBuffer buffer = ByteBuffer.allocate(row.length * 4).order(ByteOrder.LITTLE_ENDIAN);
            for (int v : row) buffer.putInt(v);
            digest.update(buffer.array());
        }
        return HexFormat.of().formatHex(digest.digest());
    }

    static int[] shapePredicates(Object state, Object world, Object location) throws Exception {
        int full = (boolean) call(state, "isCollisionShapeFullBlock", world, location) ? 16384 : 0, sturdy = 0, attach = 0;
        for (int d = 0; d < directions.length; d++) {
            if ((boolean) call(state, "isFaceSturdy", world, location, directions[d])) sturdy |= 1 << d;
            if ((boolean) call(type("world.level.block.MultifaceBlock"), "canAttachTo", world, directions[d], location, state)) attach |= 1 << d;
        }
        return new int[]{full, sturdy, attach};
    }

    static Map<String, Object> predicates() throws Exception {
        Object empty = field("world.level.EmptyBlockGetter", "INSTANCE"), location = make("core.BlockPos", 0, 0, 0);
        Object noQueries = Proxy.newProxyInstance(SculkReference.class.getClassLoader(), new Class<?>[]{type("world.level.BlockGetter")}, (p, m, a) -> {
            throw new IllegalStateException("world-dependent shape queried " + m);
        });
        Object water = field("world.level.material.Fluids", "WATER");
        Object worldgenTag = field("tags.BlockTags", "SCULK_REPLACEABLE_WORLD_GEN"), substrateTag = field("tags.BlockTags", "SCULK_REPLACEABLE"), fireTag = field("tags.BlockTags", "FIRE");
        Method inTag = type("world.level.block.state.BlockState").getMethod("is", type("tags.TagKey"));
        Field cache = type("world.level.block.state.BlockBehaviour$BlockStateBase").getDeclaredField("cache"); cache.setAccessible(true);
        List<int[]> ranges = new ArrayList<>(), veins = new ArrayList<>();
        List<Object> dynamicShapes = new ArrayList<>();
        Map<String, Integer> unsupportedShapes = new TreeMap<>();
        Map<String, Object> blocks = new TreeMap<>(), postprocessing = new TreeMap<>();
        int[] previous = null;
        int count = 0;
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            int id = id(state);
            if (id != count++) throw new IllegalStateException("noncontiguous state registry");
            Object block = call(state, "getBlock"), fluid = call(state, "getFluidState");
            String name = call(field("core.registries.BuiltInRegistries", "BLOCK"), "getKey", block).toString();
            int[] blockRange = (int[]) blocks.get(name);
            if (blockRange == null) { blockRange = new int[]{id, id + 1, id(call(block, "defaultBlockState"))}; blocks.put(name, blockRange); }
            else blockRange[1] = id + 1;
            int flags = (boolean) call(state, "isAir") ? 1 : 0;
            if ((boolean) call(fluid, "is", water)) flags |= 2;
            if (!(boolean) call(fluid, "isEmpty")) flags |= 4;
            if (name.equals("minecraft:water")) flags |= 8;
            if ((boolean) call(state, "canBeReplaced")) flags |= 16;
            if ((boolean) inTag.invoke(state, worldgenTag)) flags |= 32;
            if ((boolean) inTag.invoke(state, substrateTag)) flags |= 64;
            if ((boolean) inTag.invoke(state, fireTag)) flags |= 128;
            if (name.equals("minecraft:sculk")) flags |= 256;
            if (name.equals("minecraft:sculk_vein")) flags |= 512;
            if (name.equals("minecraft:sculk_catalyst")) flags |= 1024;
            if (name.equals("minecraft:moving_piston")) flags |= 2048;
            if (name.equals("minecraft:sculk_sensor") || name.equals("minecraft:sculk_shrieker")) flags |= 4096;
            if (type("world.level.block.SculkBehaviour").isInstance(block)) flags |= 8192;
            int sturdy = 0, attach = 0;
            String shapeClass = block.getClass().getSimpleName();
            boolean uncached = cache.get(state) == null;
            if (!uncached || WORLDGEN_DYNAMIC_SHAPES.containsKey(shapeClass)) {
                flags |= 32768;
                int[] shape = shapePredicates(state, empty, location);
                flags |= shape[0]; sturdy = shape[1]; attach = shape[2];
                if (uncached) {
                    for (Pos point : List.of(new Pos(0, 0, 0), new Pos(-17, -64, 16), new Pos(29999983, 319, -29999984))) {
                        int[] actual = shapePredicates(state, noQueries, pos(point));
                        if (!Arrays.equals(shape, actual)) throw new IllegalStateException("position-dependent sculk shape predicate " + state);
                        dynamicShapes.add(List.of(id, xyz(point), actual));
                    }
                }
            } else unsupportedShapes.merge(name, 1, Integer::sum);
            if (name.equals("minecraft:sculk_vein")) {
                int faces = 0;
                for (int d = 0; d < directions.length; d++) if ((boolean) call(type("world.level.block.MultifaceBlock"), "hasFace", state, directions[d])) faces |= 1 << d;
                veins.add(new int[]{id, faces, (boolean) call(fluid, "isEmpty") ? 0 : 1});
            }
            if (name.startsWith("minecraft:sculk") || name.equals("minecraft:water") || name.equals("minecraft:air")) {
                Object marked = call(state, "getPostProcessPos", empty, location);
                postprocessing.put(Integer.toString(id), marked == null ? null : xyz(Pos.from(marked)));
            }
            int[] row = {id, id + 1, flags, sturdy, attach};
            if (previous != null && Arrays.equals(Arrays.copyOfRange(previous, 2, 5), Arrays.copyOfRange(row, 2, 5))) previous[1]++;
            else { ranges.add(row); previous = row; }
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("state_count", count); result.put("ranges", ranges); result.put("blocks", blocks); result.put("veins", veins);
        result.put("postprocess_offsets", postprocessing);
        result.put("range_encoding", "[start,end,flags,sturdy_faces,attach_directions]; flags: air=1,water_fluid=2,nonempty_fluid=4,water_block=8,replaceable=16,worldgen_tag=32,substrate_tag=64,fire_tag=128,sculk=256,vein=512,catalyst=1024,moving_piston=2048,growth=4096,sculk_behaviour=8192,full_collision=16384,known_worldgen_shape=32768");
        result.put("uncached_shape_reasoning", WORLDGEN_DYNAMIC_SHAPES);
        result.put("uncached_shape_cases", dynamicShapes);
        result.put("unsupported_shape_blocks", unsupportedShapes);
        result.put("directions", Arrays.stream(directions).map(d -> ((Enum<?>) d).name()).toList());
        result.put("tags", Map.of("sculk_replaceable", document("tags/block", "sculk_replaceable"), "sculk_replaceable_world_gen", document("tags/block", "sculk_replaceable_world_gen")));
        return result;
    }

    static List<Object> blockEntityTemplates() throws Exception {
        List<Object> result = new ArrayList<>();
        for (String name : List.of("SCULK_SENSOR", "SCULK_CATALYST", "SCULK_SHRIEKER")) {
            Object state = blockState(name), entity = call(call(state, "getBlock"), "newBlockEntity", make("core.BlockPos", 0, 0, 0), state);
            Object saved = call(entity, "saveWithFullMetadata", registries), update = call(entity, "getUpdateTag", registries);
            Map<String, Object> row = new LinkedHashMap<>(Map.of("block", "minecraft:" + name.toLowerCase(Locale.ROOT), "state", id(state),
                "type_id", call(field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE"), "getId", call(entity, "getType")),
                "nbt", jsonNbt(saved), "typed_nbt", typedNbt(saved), "update", jsonNbt(update), "typed_update", typedNbt(update),
                "update_packet_null", call(entity, "getUpdatePacket") == null));
            List<int[]> ranges = new ArrayList<>();
            int[] range = null;
            for (Object possible : (Iterable<?>) call(call(call(state, "getBlock"), "getStateDefinition"), "getPossibleStates")) {
                int stateId = id(possible);
                if (!(boolean) call(call(entity, "getType"), "isValid", possible)) throw new IllegalStateException("invalid generated entity state " + possible);
                Object alternative = call(call(possible, "getBlock"), "newBlockEntity", make("core.BlockPos", 0, 0, 0), possible);
                if (call(alternative, "getType") != call(entity, "getType") || !call(alternative, "saveWithFullMetadata", registries).equals(saved)
                    || !call(alternative, "getUpdateTag", registries).equals(update)) throw new IllegalStateException("state-dependent generated entity defaults " + possible);
                if (range != null && range[1] == stateId) range[1]++;
                else { range = new int[]{stateId, stateId + 1}; ranges.add(range); }
            }
            row.put("valid_states", ranges);
            result.add(row);
        }
        return result;
    }

    static Map<String, Object> sample(String name, long seed, Scenario scenario, boolean trace) throws Exception {
        return sample(name, seed, scenario, trace, null);
    }

    static Map<String, Object> sample(String name, long seed, Scenario scenario, boolean trace, Object customConfig) throws Exception {
        World world = new World(scenario);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        for (int i = 0; i < scenario.advance(); i++) call(random, "nextLong");
        List<Boolean> results = new ArrayList<>();
        Object feature = configured.get(name);
        Object config = customConfig == null ? call(feature, "config")
            : call(call(field("world.level.levelgen.feature.configurations.SculkPatchConfiguration", "CODEC"), "parse", jsonOps, customConfig), "getOrThrow");
        for (int i = 0; i < scenario.repetitions(); i++) {
            Object context = make("world.level.levelgen.feature.FeaturePlaceContext", Optional.empty(), world.proxy, null, random, pos(scenario.origin()), config);
            results.add((boolean) call(call(feature, "feature"), "place", context));
        }
        Map<String, Object> result = world.result(trace);
        result.put("name", name + "/" + seed + "/" + scenario.name()); result.put("kind", name); result.put("seed", seed);
        result.put("terrain", scenario.terrain()); result.put("origin", xyz(scenario.origin()));
        result.put("write_radius", scenario.writeRadius()); result.put("advance", scenario.advance());
        result.put("placed", results); result.put("next_i64", call(random, "nextLong"));
        if (customConfig != null) result.put("config", customConfig);
        return result;
    }

    static Object customConfig(Map<String, Object> changes) throws Exception {
        Object config = call(call(document("worldgen/configured_feature", "sculk_patch_deep_dark"), "getAsJsonObject"), "get", "config");
        for (var entry : changes.entrySet()) call(config, "add", entry.getKey(), call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", entry.getValue())));
        return config;
    }

    static List<Object> cursorSnapshot(Object spreader) throws Exception {
        List<Object> result = new ArrayList<>();
        for (Object cursor : (List<?>) call(spreader, "getCursors")) {
            Map<String, Object> row = new LinkedHashMap<>();
            row.put("pos", xyz(Pos.from(call(cursor, "getPos")))); row.put("charge", call(cursor, "getCharge"));
            row.put("update_delay", member(cursor, "updateDelay")); row.put("decay_delay", call(cursor, "getDecayDelay"));
            Object facings = call(cursor, "getFacingData");
            row.put("facings", facings == null ? null : ((Number) call(type("world.level.block.MultifaceBlock"), "pack", facings)).intValue());
            result.add(row);
        }
        return result;
    }

    static Map<String, Object> kernel(String name, String terrain, long seed, List<int[]> additions,
            Map<Pos, Object> initial, List<Boolean> updates, Object replacement, boolean trace) throws Exception {
        Pos origin = new Pos(8, -32, 8);
        World world = new World(new Scenario(name, terrain, origin, 1, 0, 1), initial);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        Object spreader = call(type("world.level.block.SculkSpreader"), "createWorldGenSpreader");
        for (int[] addition : additions) call(spreader, "addCursors", make("core.BlockPos", addition[0], addition[1], addition[2]), addition[3]);
        List<Object> cursors = new ArrayList<>(), actions = new ArrayList<>();
        cursors.add(cursorSnapshot(spreader));
        for (int step = 0; step < updates.size(); step++) {
            if (step == 2 && replacement != null) {
                call(world.proxy, "setBlock", pos(origin), replacement, 3);
                actions.add(Map.of("before_update", step, "pos", xyz(origin), "state", id(replacement)));
            }
            call(spreader, "updateCursors", world.proxy, pos(origin), random, updates.get(step));
            cursors.add(cursorSnapshot(spreader));
        }
        Map<String, Object> result = world.result(trace);
        result.put("name", name); result.put("terrain", terrain); result.put("origin", xyz(origin)); result.put("write_radius", 1);
        result.put("seed", seed); result.put("additions", additions); result.put("updates", updates); result.put("actions", actions);
        List<Object> blocks = new ArrayList<>();
        for (Pos p : initial.keySet().stream().sorted(POS_ORDER).toList()) blocks.add(List.of(p.x(), p.y(), p.z(), id(initial.get(p))));
        result.put("initial_blocks", blocks); result.put("cursors", cursors); result.put("next_i64", call(random, "nextLong"));
        return result;
    }

    static List<Object> kernels(boolean trace) throws Exception {
        List<Object> result = new ArrayList<>();
        Pos origin = new Pos(8, -32, 8);
        List<int[]> start = List.of(new int[]{8, -32, 8, 33});
        List<Boolean> five = List.of(false, false, true, true, true);
        Object down = call(type("world.level.block.MultifaceBlock"), "getFaceProperty", directions[0]);
        Object vein = call(blockState("SCULK_VEIN"), "setValue", down, true);
        Object wetVein = call(vein, "setValue", field("world.level.block.state.properties.BlockStateProperties", "WATERLOGGED"), true);
        result.add(kernel("split_and_cap", "air", 0, List.of(new int[]{8, -32, 8, 33001}, new int[]{8, -32, 8, -1}), Map.of(), List.of(false), null, trace));
        result.add(kernel("unreasonable_cursor", "air", 0, List.of(new int[]{1033, -32, 8, 1}), Map.of(), List.of(true), null, trace));
        result.add(kernel("default_decay_delay", "air", 17, start, Map.of(), Collections.nCopies(3, false), null, trace));
        result.add(kernel("stationary_faces", "origin_sculk", 17, start, Map.of(), Collections.nCopies(5, false), null, trace));
        result.add(kernel("coincident_cursors", "origin_sculk", 1, List.of(new int[]{8, -32, 8, 33}, new int[]{8, -32, 8, 33}), Map.of(), Collections.nCopies(4, false), null, trace));
        result.add(kernel("dry_growth", "sculk_floor", 17, List.of(new int[]{8, -33, 8, 160}), Map.of(), Collections.nCopies(64, false), null, trace));
        result.add(kernel("waterlogged_growth", "sculk_water", 0, List.of(new int[]{8, -33, 8, 160}), Map.of(), Collections.nCopies(64, false), null, trace));
        result.add(kernel("horizontal_radius", "sculk_floor", 0, List.of(new int[]{22, -33, 8, 1000}), Map.of(), Collections.nCopies(16, false), null, trace));
        result.add(kernel("distance_decay", "air", 17, List.of(new int[]{22, -32, 8, 1000}),
            Map.of(new Pos(22, -32, 8), blockState("SCULK"), new Pos(22, -31, 8), blockState("DEEPSLATE")), Collections.nCopies(256, false), null, trace));
        result.add(kernel("regrow_dry", "bedrock", 17, start, Map.of(origin, vein), five, blockState("AIR"), trace));
        result.add(kernel("regrow_wet", "bedrock", 17, start, Map.of(origin, wetVein), five, blockState("WATER"), trace));
        result.add(kernel("empty_facings_spread", "origin_sculk", 17, start, Map.of(), five, blockState("AIR"), trace));
        result.add(kernel("flowing_water_regrow_rejected", "bedrock", 17, start, Map.of(origin, wetVein), five,
            call(blockState("WATER"), "setValue", field("world.level.block.LiquidBlock", "LEVEL"), 1), trace));
        return result;
    }

    static Map<String, Object> stream(List<String> roots, long seed, String biomeName, boolean trace) throws Exception {
        World world = new World(new Scenario("placed_stream", "layers", new Pos(8, -32, 8), 1, 0, 1));
        world.biome = biomes.get(biomeName);
        Object generator = make("world.level.levelgen.NoiseBasedChunkGenerator", make("world.level.biome.FixedBiomeSource", world.biome), noiseSettings);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        Pos origin = new Pos(0, -64, 0);
        List<Boolean> results = new ArrayList<>();
        for (String name : roots) results.add((boolean) call(placed.get(name), "placeWithBiomeCheck", world.proxy, generator, random, pos(origin)));
        Map<String, Object> result = world.result(trace);
        result.put("name", String.join("+", roots) + "/" + seed + "/" + biomeName);
        result.put("roots", roots); result.put("seed", seed); result.put("terrain", "layers");
        result.put("origin", xyz(world.scenario.origin())); result.put("stream_origin", xyz(origin)); result.put("write_radius", 1);
        result.put("biome", biomeName); result.put("placed", results); result.put("feature_origins", world.featureOrigins);
        result.put("next_i64", call(random, "nextLong"));
        return result;
    }

    static List<Object> streams(boolean trace) throws Exception {
        List<Object> result = new ArrayList<>();
        List<String> roots = List.of("sculk_patch_deep_dark", "sculk_vein");
        for (String name : roots) for (long seed : new long[]{0, 17}) result.add(stream(List.of(name), seed, "deep_dark", trace));
        result.add(stream(roots, 0, "deep_dark", trace));
        result.add(stream(roots, 17, "plains", trace));
        return result;
    }

    static void bootstrap() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion"); call(type("server.Bootstrap"), "bootStrap");
        registries = NativeWorldgenRegistries.load();
        jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        gson = call(call(Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance(), "serializeNulls"), "create");
        getId = type("world.level.block.Block").getMethod("getId", type("world.level.block.state.BlockState"));
        sectionSet = type("world.level.chunk.LevelChunkSection").getMethod("setBlockState", int.class, int.class, int.class, type("world.level.block.state.BlockState"), boolean.class);
        directions = type("core.Direction").getEnumConstants();
        factory = call(type("world.level.chunk.PalettedContainerFactory"), "create", registries);
        height = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        serverLevel = TreeEffectReference.probeLevel();
        Object dimensions = call(registries, "lookupOrThrow", field("core.registries.Registries", "DIMENSION_TYPE"));
        dimension = call(call(dimensions, "getOrThrow", field("world.level.dimension.BuiltinDimensionTypes", "OVERWORLD")), "value");
        Object features = call(registries, "lookupOrThrow", field("core.registries.Registries", "CONFIGURED_FEATURE"));
        Object placedFeatures = call(registries, "lookupOrThrow", field("core.registries.Registries", "PLACED_FEATURE"));
        for (String name : CONFIGS) {
            Object key = call(type("resources.ResourceKey"), "create", field("core.registries.Registries", "CONFIGURED_FEATURE"), call(type("resources.Identifier"), "withDefaultNamespace", name));
            configured.put(name, call(call(features, "getOrThrow", key), "value"));
            Object placedKey = call(type("resources.ResourceKey"), "create", field("core.registries.Registries", "PLACED_FEATURE"), call(type("resources.Identifier"), "withDefaultNamespace", name));
            placed.put(name, call(call(placedFeatures, "getOrThrow", placedKey), "value"));
        }
        Object biomeRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        for (String name : List.of("deep_dark", "plains")) {
            Object key = call(type("resources.ResourceKey"), "create", field("core.registries.Registries", "BIOME"), call(type("resources.Identifier"), "withDefaultNamespace", name));
            biomes.put(name, call(biomeRegistry, "getOrThrow", key));
        }
        noiseSettings = call(call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS")), "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD"));
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        boolean trace = Arrays.asList(args).contains("--trace"), dataOnly = Arrays.asList(args).contains("--data-only");
        Map<String, Object> data = predicates();
        Map<String, Object> configs = new TreeMap<>(), placements = new TreeMap<>();
        for (String name : CONFIGS) { configs.put(name, document("worldgen/configured_feature", name)); placements.put(name, document("worldgen/placed_feature", name)); }
        data.put("configurations", configs); data.put("placements", placements); data.put("block_entities", blockEntityTemplates());
        Map<String, Object> biomeIds = new TreeMap<>();
        Object biomeRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        for (var entry : biomes.entrySet()) biomeIds.put(entry.getKey(), call(biomeRegistry, "getId", call(entry.getValue(), "value")));
        data.put("biome_ids", biomeIds);
        Map<String, Object> placementStates = new TreeMap<>();
        Object wet = field("world.level.block.state.properties.BlockStateProperties", "WATERLOGGED");
        Object sensor = blockState("SCULK_SENSOR");
        Object shrieker = call(blockState("SCULK_SHRIEKER"), "setValue", field("world.level.block.SculkShriekerBlock", "CAN_SUMMON"), true);
        placementStates.put("sensor", List.of(id(sensor), id(call(sensor, "setValue", wet, true))));
        placementStates.put("shrieker", List.of(id(shrieker), id(call(shrieker, "setValue", wet, true))));
        placementStates.put("catalyst", id(blockState("SCULK_CATALYST")));
        placementStates.put("sculk", id(blockState("SCULK")));
        data.put("placement_states", placementStates);
        data.put("test_states", Map.of("flowing_water", id(call(blockState("WATER"), "setValue", field("world.level.block.LiquidBlock", "LEVEL"), 1)),
            "vein_down", id(call(blockState("SCULK_VEIN"), "setValue", call(type("world.level.block.MultifaceBlock"), "getFaceProperty", directions[0]), true))));
        data.put("vein_directions", ((List<?>) member(call(configured.get("sculk_vein"), "config"), "validDirections")).stream().map(d -> ((Enum<?>) d).ordinal()).toList());
        Field neighbours = type("world.level.block.SculkSpreader$ChargeCursor").getDeclaredField("NON_CORNER_NEIGHBOURS");
        neighbours.setAccessible(true);
        List<Object> offsets = new ArrayList<>();
        for (Object offset : (List<?>) neighbours.get(null)) offsets.add(xyz(Pos.from(offset)));
        data.put("non_corner_neighbours", offsets);
        Object spreader = call(type("world.level.block.SculkSpreader"), "createWorldGenSpreader");
        Map<String, Object> parameters = new TreeMap<>();
        for (String name : List.of("growthSpawnCost", "noGrowthRadius", "chargeDecayRate", "additionalDecayRate", "isWorldGeneration")) parameters.put(name, call(spreader, name));
        data.put("worldgen_spreader", parameters);
        List<Object> samples = new ArrayList<>();
        if (!dataOnly) {
            Pos origin = new Pos(8, -32, 8);
            for (String name : CONFIGS) for (long seed : new long[]{0, 1, 17, 42})
                for (String terrain : List.of("flat", "cave", "water"))
                    samples.add(sample(name, seed, new Scenario(terrain, terrain, origin, 1, 0, 1), trace));
            for (String terrain : List.of("air", "solid", "lava", "flowing_water", "bedrock", "mixed", "ceiling", "walls", "origin_sculk", "origin_vein", "sculk_floor", "search"))
                for (String name : List.of("sculk_patch_deep_dark", "sculk_vein"))
                    samples.add(sample(name, 17, new Scenario(terrain, terrain, origin, 1, 0, 1), trace));
            for (String name : CONFIGS) {
                samples.add(sample(name, -1, new Scenario("negative_chunk_edge", "flat", new Pos(-1, -32, 16), 1, 3, 1), trace));
                samples.add(sample(name, 42, new Scenario("write_radius_zero", "flat", new Pos(15, -32, 15), 0, 0, 1), trace));
                samples.add(sample(name, 1, new Scenario("min_y", "flat", new Pos(8, -64, 8), 1, 0, 1), trace));
                samples.add(sample(name, 0, new Scenario("max_y", "flat", new Pos(8, 319, 8), 1, 0, 1), trace));
                samples.add(sample(name, 17, new Scenario("repeated", "cave", origin, 1, 0, 2), trace));
            }
            for (long seed : new long[]{0, 17}) samples.add(sample("sculk_patch_ancient_city", seed, new Scenario("city", "city", origin, 1, 0, 1), trace));
            for (String name : List.of("sculk_patch_deep_dark", "sculk_vein")) samples.add(sample(name, 17, new Scenario("dynamic", "dynamic", origin, 1, 0, 1), trace));
            samples.add(sample("sculk_patch_deep_dark", 17, new Scenario("spread_growth_rounds", "cave", origin, 1, 0, 1), trace,
                customConfig(Map.of("charge_count", 4, "amount_per_charge", 200, "spread_attempts", 32, "spread_rounds", 2, "growth_rounds", 2, "catalyst_chance", 0.0))));
            samples.add(sample("sculk_patch_deep_dark", 1, new Scenario("growth_only", "sculk_floor", origin, 1, 0, 1), trace,
                customConfig(Map.of("charge_count", 3, "amount_per_charge", 75, "spread_attempts", 32, "spread_rounds", 0, "growth_rounds", 2, "catalyst_chance", 1.0))));
            samples.add(sample("sculk_patch_deep_dark", 42, new Scenario("minimal_charge", "flat", origin, 1, 0, 1), trace,
                customConfig(Map.of("charge_count", 1, "amount_per_charge", 1, "spread_attempts", 1, "catalyst_chance", 0.0))));
            samples.add(sample("sculk_patch_deep_dark", 17, new Scenario("rare_uniform_singleton", "flat", origin, 1, 0, 1), trace,
                customConfig(Map.of("spread_rounds", 0, "catalyst_chance", 1.0, "extra_rare_growths", Map.of("type", "minecraft:uniform", "min_inclusive", 3, "max_inclusive", 3)))));
            samples.add(sample("sculk_patch_deep_dark", 0, new Scenario("negative_extra", "flat", origin, 1, 0, 1), trace,
                customConfig(Map.of("spread_rounds", 0, "catalyst_chance", 0.0, "extra_rare_growths", -1))));
            samples.add(sample("sculk_patch_deep_dark", 1, new Scenario("many_cursors", "cave", origin, 1, 0, 1), trace,
                customConfig(Map.of("charge_count", 32, "amount_per_charge", 500, "spread_attempts", 64, "spread_rounds", 2, "growth_rounds", 1))));
        }
        System.out.println("SCULK_REFERENCE=" + call(gson, "toJson", Map.of("data", data, "samples", samples, "kernels", dataOnly ? List.of() : kernels(trace), "streams", dataOnly ? List.of() : streams(trace),
            "setup", "Native configured features and worldgen spreader; constructed CARVERS ProtoChunks; native WorldGenRegion writes with explicit read radius 2 and recorded write radius; no ticks or postprocessing executed")));
    }
}
