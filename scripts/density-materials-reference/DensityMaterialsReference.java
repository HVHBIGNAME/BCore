import java.lang.reflect.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.Predicate;

/** Calls 26.1 OreVeinifier, Beardifier and NoiseChunk; no copied native kernel. */
public class DensityMaterialsReference extends JigsawSupport {
    static Object settings, noises, registryOps, gson, picker;
    static Class<?> densityClass, contextClass, randomClass;
    static Method compute, createOre, calculateOre;
    static final Map<Long, Object> randomStates = new HashMap<>();

    static Method method(String owner, String name, Class<?>... parameters) throws Exception {
        Method m = type(owner).getDeclaredMethod(name, parameters);
        m.setAccessible(true);
        return m;
    }

    static Object json(Object value) throws Exception {
        return call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", value));
    }

    static Object decode(Object codec, Object value) throws Exception {
        return call(call(codec, "parse", registryOps, json(value)), "getOrThrow");
    }

    static Object replaceRecord(Object record, String name, Object replacement) throws Exception {
        RecordComponent[] components = record.getClass().getRecordComponents();
        Class<?>[] types = new Class<?>[components.length];
        Object[] values = new Object[components.length];
        boolean found = false;
        for (int i = 0; i < components.length; i++) {
            types[i] = components[i].getType();
            values[i] = components[i].getName().equals(name) ? replacement : components[i].getAccessor().invoke(record);
            found |= components[i].getName().equals(name);
        }
        if (!found) throw new IllegalArgumentException(name);
        return record.getClass().getConstructor(types).newInstance(values);
    }

    static Object randomState(long seed) throws Exception {
        Object value = randomStates.get(seed);
        if (value == null) {
            value = call(type("world.level.levelgen.RandomState"), "create", settings, noises, seed);
            randomStates.put(seed, value);
        }
        return value;
    }

    static Object context(int x, int y, int z) throws Exception {
        return make("world.level.levelgen.DensityFunction$SinglePointContext", x, y, z);
    }

    static String bits(double value) { return HexFormat.of().toHexDigits(Double.doubleToRawLongBits(value)); }
    static String bits(float value) { return HexFormat.of().toHexDigits(Float.floatToRawIntBits(value)); }

    static Object constant(double value, String label, List<String> events) {
        return Proxy.newProxyInstance(DensityMaterialsReference.class.getClassLoader(), new Class<?>[]{densityClass}, (p, m, a) -> {
            if (m.getName().equals("compute")) { events.add(label); return value; }
            throw new UnsupportedOperationException(m.toString());
        });
    }

    static Map<String, Object> materialCase(String id, long seed, int x, int y, int z,
                                           double toggle, double ridged, double gap, float[] scripted) throws Exception {
        List<String> events = new ArrayList<>();
        Object nativeFactory = call(randomState(seed), "oreRandom");
        Object[] randomUsed = {null};
        int[] draws = {0};
        Object factory = Proxy.newProxyInstance(DensityMaterialsReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.PositionalRandomFactory")}, (p, m, a) -> {
                if (!m.getName().equals("at")) throw new UnsupportedOperationException(m.toString());
                if (!Arrays.equals(a, new Object[]{x, y, z})) throw new AssertionError("wrong RNG position");
                events.add("at");
                randomUsed[0] = call(nativeFactory, "at", a);
                return Proxy.newProxyInstance(DensityMaterialsReference.class.getClassLoader(), new Class<?>[]{randomClass}, (rp, rm, ra) -> {
                    if (!rm.getName().equals("nextFloat")) throw new UnsupportedOperationException(rm.toString());
                    events.add("float");
                    int index = draws[0]++;
                    return scripted == null ? (float) call(randomUsed[0], "nextFloat") : scripted[index];
                });
            });
        Object filler = createOre.invoke(null, constant(toggle, "toggle", events),
            constant(ridged, "ridged", events), constant(gap, "gap", events), factory);
        Object state = calculateOre.invoke(filler, context(x, y, z));
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("id", id); result.put("seed", seed); result.put("pos", List.of(x, y, z));
        result.put("inputs", List.of(bits(toggle), bits(ridged), bits(gap)));
        result.put("state", state == null ? -1 : stateId(state));
        result.put("events", events); result.put("draws", draws[0]);
        if (scripted != null) {
            List<String> values = new ArrayList<>();
            for (float f : scripted) values.add(bits(f));
            result.put("scripted", values);
        } else if (randomUsed[0] != null) result.put("next_i64", call(randomUsed[0], "nextLong"));
        return result;
    }

    static Object materials() throws Exception {
        List<Object> cases = new ArrayList<>();
        int n = 0;
        for (double toggle : new double[]{0.0, -0.0, 0.4, (double) 0.4f, Math.nextDown((double) 0.4f),
                Math.nextUp((double) 0.4f), 0.5, (double) 0.6f, 0.8, -0.4, -(double) 0.4f, -0.5, -(double) 0.6f, -0.8}) {
            for (int y : new int[]{Integer.MIN_VALUE, -65, -61, -60, -59, -41, -40, -34, -28, -9, -8, -7,
                    -1, 0, 1, 19, 20, 25, 30, 49, 50, 51, 320, Integer.MAX_VALUE}) {
                cases.add(materialCase("height-threshold/" + n++, 0, -17, y, 16, toggle, -0.01, 0.0, new float[]{0, 0, 0}));
            }
        }
        for (float solid : new float[]{0, Math.nextDown(0.7f), 0.7f, Math.nextUp(0.7f), Math.nextDown(1f)})
            for (double ridged : new double[]{-1, -Double.MIN_VALUE, -0.0, 0.0, Double.MIN_VALUE, 1})
                cases.add(materialCase("solid-ridge/" + n++, 0, 16, 25, -1, 0.8, ridged, 0, new float[]{solid, 0, 0}));
        for (double sign : new double[]{1, -1}) {
            for (float rich : new float[]{0, Math.nextDown(0.1f), 0.1f, 0.2f, Math.nextDown(0.3f), 0.3f, Math.nextUp(0.3f)})
                for (double gap : new double[]{-1, -0.3, -(double) 0.3f, Math.nextDown(-(double) 0.3f), Math.nextUp(-(double) 0.3f), 0})
                    for (float raw : new float[]{0, Math.nextDown(0.02f), 0.02f, Math.nextUp(0.02f)})
                        cases.add(materialCase("rich-gap-raw/" + n++, 0, 0, sign > 0 ? 25 : -34, 0,
                            sign * 0.8, -0.01, gap, new float[]{0, rich, raw}));
        }
        for (long seed : new long[]{0, 1, -1, 1234, 846692123413862008L, Long.MIN_VALUE, Long.MAX_VALUE}) {
            for (int[] xz : new int[][]{{0, 0}, {15, 16}, {-1, -1}, {-17, 31}, {-2000, 3000},
                    {29999999, -29999999}, {-29999999, 29999999}, {Integer.MIN_VALUE, Integer.MAX_VALUE}}) {
                for (int y : new int[]{-61, -60, -59, -40, -34, -20, -9, -8, -7, 0, 1, 20, 25, 49, 50, 51}) {
                    cases.add(materialCase("seeded/" + n++, seed, xz[0], y, xz[1], y < 0 ? -0.8 : 0.8, -0.01, 0, null));
                }
            }
        }
        return Map.of("cases", cases, "states", describeStates());
    }

    static Map<String, Integer> describeStates() throws Exception {
        Map<String, Integer> states = new TreeMap<>();
        for (String name : List.of("AIR", "STONE", "WATER", "LAVA", "COPPER_ORE", "RAW_COPPER_BLOCK", "GRANITE",
                "DEEPSLATE_IRON_ORE", "RAW_IRON_BLOCK", "TUFF")) states.put(name, stateId(state(name)));
        return states;
    }

    static Object protoFactory, heightAccessor, generator;

    static Object proto(int x, int z) throws Exception {
        if (protoFactory == null) {
            Object blockStrategy = call(type("world.level.chunk.Strategy"), "createForBlockStates", field("world.level.block.Block", "BLOCK_STATE_REGISTRY"));
            Object biome = call(registry("BIOME"), "getOrThrow", key("BIOME", "minecraft:plains"));
            Object biomeIds = make("core.IdMapper"); call(biomeIds, "add", biome);
            Object biomeStrategy = call(type("world.level.chunk.Strategy"), "createForBiomes", biomeIds);
            protoFactory = make("world.level.chunk.PalettedContainerFactory", blockStrategy, state("AIR"), null, biomeStrategy, biome, null);
            heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
            generator = make("world.level.levelgen.NoiseBasedChunkGenerator", make("world.level.biome.FixedBiomeSource", biome),
                call(type("core.Holder"), "direct", settings));
        }
        return make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", x, z), field("world.level.chunk.UpgradeData", "EMPTY"),
            null, make("world.ticks.ProtoChunkTicks"), make("world.ticks.ProtoChunkTicks"), heightAccessor, protoFactory, null);
    }

    static Object structure(String adaptation) throws Exception {
        // A real decoded structure with an explicit terrain adaptation. Piece
        // generation is supplied separately; this tests admission/filtering.
        return decode(field("world.level.levelgen.structure.Structure", "DIRECT_CODEC"), Map.of(
            "type", "minecraft:mineshaft", "biomes", "#minecraft:has_structure/mineshaft",
            "spawn_overrides", Map.of(), "step", "underground_structures", "terrain_adaptation", adaptation,
            "mineshaft_type", "normal"));
    }

    static Object poolPiece(String projection, int delta, int[] bb, int[][] junctions) throws Exception {
        Object element = decode(field("world.level.levelgen.structure.pools.StructurePoolElement", "CODEC"), Map.of(
            "element_type", "minecraft:single_pool_element", "location", "minecraft:empty",
            "processors", "minecraft:empty", "projection", projection));
        Object box = make("world.level.levelgen.structure.BoundingBox", bb[0], bb[1], bb[2], bb[3], bb[4], bb[5]);
        Object piece = make("world.level.levelgen.structure.PoolElementStructurePiece", templateManager, element,
            make("core.BlockPos", bb[0], bb[1], bb[2]), delta, field("world.level.block.Rotation", "NONE"), box,
            field("world.level.levelgen.structure.templatesystem.LiquidSettings", "APPLY_WATERLOGGING"));
        for (int[] j : junctions) call(piece, "addJunction", make("world.level.levelgen.structure.pools.JigsawJunction",
            j[0], j[1], j[2], j[3], field("world.level.levelgen.structure.pools.StructureTemplatePool$Projection", j[4] == 0 ? "RIGID" : "TERRAIN_MATCHING")));
        return piece;
    }

    static Object start(String adaptation, List<Object> pieces, int cx, int cz) throws Exception {
        return make("world.level.levelgen.structure.StructureStart", structure(adaptation), make("world.level.ChunkPos", cx, cz), 0,
            make("world.level.levelgen.structure.pieces.PiecesContainer", pieces));
    }

    record Selected(Object manager, Object chunk, Object beard, List<?> starts) {}

    static Selected select(List<?> starts, int cx, int cz) throws Exception {
        Map<Pos, Object> chunks = new HashMap<>();
        Object target = proto(cx, cz);
        chunks.put(new Pos(cx, 0, cz), target);
        // An ordered caller-supplied reference map is part of this fixture's
        // input. Actual native StructureManager and LongOpenHashSet iteration
        // select and order the starts; no Beardifier selection is reimplemented.
        Map<Object, Object> references = new LinkedHashMap<>();
        for (Object s : starts) {
            Object source = call(s, "getChunkPos");
            int sx = (int) call(source, "x"), sz = (int) call(source, "z");
            Pos key = new Pos(sx, 0, sz);
            Object c = chunks.get(key);
            if (c == null) { c = proto(sx, sz); chunks.put(key, c); }
            Object structure = call(s, "getStructure");
            call(c, "setStartForStructure", structure, s);
            Object set = references.get(structure);
            if (set == null) { set = Class.forName("it.unimi.dsi.fastutil.longs.LongOpenHashSet").getConstructor().newInstance(); references.put(structure, set); }
            call(set, "add", ((long) sx & 0xffffffffL) | ((long) sz << 32));
        }
        Field refs = type("world.level.chunk.ChunkAccess").getDeclaredField("structuresRefences");
        refs.setAccessible(true); refs.set(target, references);
        Object level = Proxy.newProxyInstance(DensityMaterialsReference.class.getClassLoader(), new Class<?>[]{type("world.level.LevelAccessor")}, (p, m, a) -> {
            return switch (m.getName()) {
                case "getChunk" -> {
                    Object c = chunks.get(new Pos((int) a[0], 0, (int) a[1]));
                    if (c == null) throw new AssertionError("unavailable referenced chunk");
                    yield c;
                }
                case "getMinSectionY" -> -4;
                default -> throw new UnsupportedOperationException(m.toString());
            };
        });
        Object manager = make("world.level.StructureManager", level, make("world.level.levelgen.WorldOptions", 0L, true, false), null);
        List<?> selected = (List<?>) call(manager, "startsForStructure", make("world.level.ChunkPos", cx, cz), (Predicate<Object>) s -> true);
        Object beard = call(type("world.level.levelgen.Beardifier"), "forStructuresInChunk", manager, make("world.level.ChunkPos", cx, cz));
        return new Selected(manager, target, beard, selected);
    }

    static Map<String, Object> junctionData(Object j) throws Exception {
        return Map.of("source", List.of(call(j, "getSourceX"), call(j, "getSourceGroundY"), call(j, "getSourceZ")),
            "delta_y", call(j, "getDeltaY"), "destination_projection", call(call(j, "getDestProjection"), "getSerializedName"));
    }

    static List<Object> startData(List<?> starts) throws Exception {
        List<Object> result = new ArrayList<>();
        for (Object s : starts) {
            List<Object> pieces = new ArrayList<>();
            for (Object p : (List<?>) call(s, "getPieces")) {
                List<Object> junctions = new ArrayList<>();
                boolean pool = type("world.level.levelgen.structure.PoolElementStructurePiece").isInstance(p);
                if (pool) for (Object j : (List<?>) call(p, "getJunctions")) junctions.add(junctionData(j));
                pieces.add(Map.of("bounds", bounds(call(p, "getBoundingBox")), "ground_level_delta", pool ? call(p, "getGroundLevelDelta") : 0,
                    "projection", pool ? call(call(call(p, "getElement"), "getProjection"), "getSerializedName") : "non_pool",
                    "junctions", junctions));
            }
            result.add(Map.of("terrain_adaptation", call(call(call(s, "getStructure"), "terrainAdaptation"), "getSerializedName"),
                "pieces", pieces, "reference_bounds", bounds(call(s, "getBoundingBox"))));
        }
        return result;
    }

    static Map<String, Object> selectedData(Object beard) throws Exception {
        List<Object> pieces = new ArrayList<>(), junctions = new ArrayList<>();
        for (Object p : (List<?>) member(beard, "pieces")) pieces.add(Map.of("bounds", bounds(call(p, "box")),
            "terrain_adaptation", call(call(p, "terrainAdjustment"), "getSerializedName"), "ground_level_delta", call(p, "groundLevelDelta")));
        for (Object j : (List<?>) member(beard, "junctions")) junctions.add(junctionData(j));
        Object affected = member(beard, "affectedBox");
        return Map.of("pieces", pieces, "junctions", junctions, "affected_bounds", affected == null ? List.of() : bounds(affected));
    }

    static Object assembledStart(String name, long seed, int cx, int cz) throws Exception {
        proto(cx, cz); // Initialize the real noise generator and height accessor.
        Object context = make("world.level.levelgen.structure.Structure$GenerationContext", registries, generator,
            call(generator, "getBiomeSource"), randomState(seed), templateManager, seed, make("world.level.ChunkPos", cx, cz),
            heightAccessor, (Predicate<Object>) h -> true);
        Object structure = call(call(registry("STRUCTURE"), "getOrThrow", key("STRUCTURE", name)), "value");
        Object stub = ((Optional<?>) call(structure, "findGenerationPoint", context)).orElseThrow();
        return make("world.level.levelgen.structure.StructureStart", structure, make("world.level.ChunkPos", cx, cz), 0,
            call(call(stub, "getPiecesBuilder"), "build"));
    }

    static List<Object> syntheticStarts(int x, int z, int y) throws Exception {
        List<Object> starts = new ArrayList<>();
        int i = 0;
        for (String adjustment : List.of("beard_thin", "beard_box", "bury", "encapsulate", "none")) {
            starts.add(start(adjustment, List.of(
                poolPiece("rigid", i - 2, new int[]{x - 3, y + i, z - 2, x + 6, y + i + 8, z + 7},
                    new int[][]{{x + 3, y + 1, z + 1, -9, 1}, {x - 1, y + 4, z + 2, 6, 0}}),
                poolPiece("terrain_matching", -5, new int[]{x + 7, y - 4, z, x + 19, y + 1, z + 4},
                    new int[][]{{x + 7, y + 2, z + 1, 12, 0}, {x + 7, y + 2, z + 1, 12, 1}})), (x >> 4) + i, z >> 4));
            i++;
        }
        return starts;
    }

    static Map<String, Object> beardCase(String id, List<?> starts, int cx, int cz, int y) throws Exception {
        Selected selected = select(starts, cx, cz);
        List<List<Integer>> points = new ArrayList<>(); List<String> values = new ArrayList<>();
        for (int dz = -16; dz <= 31; dz += 3) for (int dy = -27; dy <= 36; dy += 3) for (int dx = -16; dx <= 31; dx += 3) {
            int x = cx * 16 + dx, yy = y + dy, z = cz * 16 + dz;
            points.add(List.of(x, yy, z)); values.add(bits((double) compute.invoke(selected.beard, context(x, yy, z))));
        }
        return Map.of("id", id, "chunk", List.of(cx, cz), "starts", startData(selected.starts), "selected", selectedData(selected.beard),
            "points", points, "bits", values);
    }

    static Object beard() throws Exception {
        Field table = type("world.level.levelgen.Beardifier").getDeclaredField("BEARD_KERNEL"); table.setAccessible(true);
        List<String> kernel = new ArrayList<>();
        for (float v : (float[]) table.get(null)) kernel.add(bits(v));
        Method beard = method("world.level.levelgen.Beardifier", "getBeardContribution", int.class, int.class, int.class, int.class);
        Method bury = method("world.level.levelgen.Beardifier", "getBuryContribution", double.class, double.class, double.class);
        List<Object> contributions = new ArrayList<>(), buries = new ArrayList<>(), cases = new ArrayList<>();
        for (int x : new int[]{Integer.MIN_VALUE, -13, -12, -11, -1, 0, 1, 11, 12, Integer.MAX_VALUE})
            for (int y : new int[]{-13, -12, -1, 0, 1, 11, 12}) for (int z : new int[]{-13, -12, -1, 0, 1, 11, 12})
                for (int ground : new int[]{Integer.MIN_VALUE, -100, -12, -1, 0, 11, 100, Integer.MAX_VALUE})
                    contributions.add(Map.of("input", List.of(x, y, z, ground), "bits", bits((double) beard.invoke(null, x, y, z, ground))));
        for (double x : new double[]{-12, -6, -3, -0.0, 0, 0.5, 3, 6, 12})
            for (double y : new double[]{-6, -3.5, -0.5, 0, 0.5, 3.5, 6}) for (double z : new double[]{-6, -1, 0, 1, 6})
                buries.add(Map.of("input", List.of(bits(x), bits(y), bits(z)), "bits", bits((double) bury.invoke(null, x, y, z))));
        for (int[] origin : new int[][]{{0, 0, 63}, {-16, -16, -37}, {29999984, -29999984, 255}}) {
            List<Object> starts = syntheticStarts(origin[0], origin[1], origin[2]);
            for (int dx = -1; dx <= 1; dx++) cases.add(beardCase("ordered/" + Arrays.toString(origin) + "/" + dx,
                starts, (origin[0] >> 4) + dx, origin[1] >> 4, origin[2]));
            Collections.reverse(starts);
            cases.add(beardCase("reversed/" + Arrays.toString(origin), starts, origin[0] >> 4, origin[1] >> 4, origin[2]));
        }
        List<Object> edges = new ArrayList<>();
        for (int x : new int[]{-13, -12, -11, 26, 27, 28, 108}) edges.add(poolPiece("rigid", 0, new int[]{x, 63, 0, x, 65, 0}, new int[][]{{0, 64, 0, 0, 0}}));
        edges.add(poolPiece("terrain_matching", 0, new int[]{0, 63, 0, 15, 65, 15}, new int[][]{
            {-12, 63, 0, 1, 0}, {-11, 63, 0, 2, 1}, {26, 63, 0, 3, 0}, {27, 63, 0, 4, 1},
            {0, 63, -12, 5, 0}, {0, 63, -11, 6, 1}, {0, 63, 26, 7, 0}, {0, 63, 27, 8, 1}}));
        cases.add(beardCase("strict-junction-inclusive-piece-margins", List.of(start("beard_thin", edges, 0, 0)), 0, 0, 63));
        cases.add(beardCase("affected-guard-ground-delta", List.of(start("beard_box", List.of(
            poolPiece("rigid", 32, new int[]{0, 63, 0, 4, 64, 4}, new int[0][])), 0, 0)), 0, 0, 90));
        cases.add(beardCase("terrain-matching-no-junction", List.of(start("encapsulate", List.of(
            poolPiece("terrain_matching", 0, new int[]{0, 63, 0, 15, 70, 15}, new int[0][])), 0, 0)), 0, 0, 63));
        cases.add(beardCase("none-ignores-junction", List.of(start("none", edges, 0, 0)), 0, 0, 63));
        cases.add(beardCase("empty", List.of(), 0, 0, 63));
        Object ordinary = make("world.level.levelgen.structure.structures.BuriedTreasurePieces$BuriedTreasurePiece", make("core.BlockPos", -1, 64, 15));
        cases.add(beardCase("ordinary-piece", List.of(start("bury", List.of(ordinary), -1, 0)), 0, 0, 64));
        for (String name : List.of("minecraft:village_plains", "minecraft:ancient_city")) {
            Object actual = assembledStart(name, 846692123413862008L, -2, 1);
            cases.add(beardCase(name, List.of(actual), -2, 1, name.endsWith("ancient_city") ? -40 : 70));
        }
        return Map.of("kernel", kernel, "contributions", contributions, "buries", buries, "cases", cases);
    }

    static int[] findVeinChunk(long seed, boolean copper, int baseX, int baseZ) throws Exception {
        Object toggle = call(call(randomState(seed), "router"), "veinToggle");
        for (int z = 0; z < 96; z++) for (int x = 0; x < 96; x++) {
            int wx = baseX + x * 16, wz = baseZ + z * 16;
            double value = (double) compute.invoke(toggle, context(wx, copper ? 24 : -32, wz));
            if (copper ? value > 0.62 : value < -0.62) return new int[]{wx >> 4, wz >> 4};
        }
        throw new AssertionError("no native vein region found " + seed + " / " + copper);
    }

    static String md5(byte[] bytes) throws Exception { return HexFormat.of().formatHex(MessageDigest.getInstance("MD5").digest(bytes)); }

    static Map<String, Object> noiseCase(String id, long seed, int cx, int cz, List<?> starts, boolean ores) throws Exception {
        Selected selected = select(starts, cx, cz);
        Object config = replaceRecord(settings, "oreVeinsEnabled", ores);
        Object rs = call(type("world.level.levelgen.RandomState"), "create", config, noises, seed);
        Object localGenerator = make("world.level.levelgen.NoiseBasedChunkGenerator", call(generator, "getBiomeSource"), call(type("core.Holder"), "direct", config));
        Object emptyBlender = call(type("world.level.levelgen.blending.Blender"), "empty");
        // The entire native generator NOISE stage is the voxel oracle.
        ((java.util.concurrent.CompletableFuture<?>) call(localGenerator, "fillFromNoise", emptyBlender, rs, selected.manager, selected.chunk)).join();
        int[] states = new int[384 * 256];
        Method getState = type("world.level.chunk.LevelChunkSection").getMethod("getBlockState", int.class, int.class, int.class);
        Map<Object, Integer> stateIds = new IdentityHashMap<>();
        Object[] sections = (Object[]) call(selected.chunk, "getSections");
        for (int y = -64; y < 320; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
            Object state = getState.invoke(sections[(y + 64) >> 4], x, y & 15, z);
            Integer sid = stateIds.get(state);
            if (sid == null) { sid = stateId(state); stateIds.put(state, sid); }
            states[(y + 64) * 256 + z * 16 + x] = sid;
        }
        // A second, explicitly traversed NoiseChunk checks router bits and the
        // global chronological mark order against the native generator's output.
        Object nc = make("world.level.levelgen.NoiseChunk", 4, rs, cx * 16, cz * 16, call(config, "noiseSettings"), selected.beard,
            config, picker, emptyBlender);
        Object visitor = Proxy.newProxyInstance(DensityMaterialsReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.DensityFunction$Visitor")}, (p, m, a) -> {
                if (m.getName().equals("apply")) return call(nc, "wrap", a[0]);
                if (m.getName().equals("visitNoise")) return a[0];
                throw new UnsupportedOperationException(m.toString());
            });
        List<Object> functions = new ArrayList<>();
        for (String name : List.of("veinToggle", "veinRidged", "veinGap")) functions.add(call(call(call(rs, "router"), name), "mapAll", visitor));
        Method updateY = method("world.level.levelgen.NoiseChunk", "updateForY", int.class, double.class);
        Method updateX = method("world.level.levelgen.NoiseChunk", "updateForX", int.class, double.class);
        Method updateZ = method("world.level.levelgen.NoiseChunk", "updateForZ", int.class, double.class);
        Method getDensity = method("world.level.levelgen.NoiseChunk", "getInterpolatedDensity");
        Method getMaterial = method("world.level.levelgen.NoiseChunk", "getInterpolatedState");
        Method updateFlag = type("world.level.levelgen.Aquifer").getMethod("shouldScheduleFluidUpdate");
        Object aquifer = call(nc, "aquifer");
        ByteBuffer[] signalBytes = new ByteBuffer[4];
        for (int i = 0; i < 4; i++) signalBytes[i] = ByteBuffer.allocate(states.length * 8).order(ByteOrder.LITTLE_ENDIAN);
        List<List<Integer>> marks = new ArrayList<>(); List<Object> samples = new ArrayList<>();
        int nonpositiveVeins = 0;
        call(nc, "initializeForFirstCellX");
        for (int cellX = 0; cellX < 4; cellX++) {
            call(nc, "advanceCellX", cellX);
            for (int cellZ = 0; cellZ < 4; cellZ++) for (int cellY = 47; cellY >= 0; cellY--) {
                call(nc, "selectCellYZ", cellY, cellZ);
                for (int dy = 7; dy >= 0; dy--) {
                    int y = -64 + cellY * 8 + dy; updateY.invoke(nc, y, dy / 8.0);
                    for (int dx = 0; dx < 4; dx++) {
                        int x = cx * 16 + cellX * 4 + dx; updateX.invoke(nc, x, dx / 4.0);
                        for (int dz = 0; dz < 4; dz++) {
                            int z = cz * 16 + cellZ * 4 + dz; updateZ.invoke(nc, z, dz / 4.0);
                            int index = (y + 64) * 256 + (z & 15) * 16 + (x & 15);
                            double density = (double) getDensity.invoke(nc);
                            Object material = getMaterial.invoke(nc);
                            int sid = material == null ? 1 : stateId(material);
                            if (sid != states[index]) throw new AssertionError("NOISE traversal mismatch " + id + " at " + x + "," + y + "," + z);
                            if ((sid == 86 || sid == 102) && (boolean) updateFlag.invoke(aquifer)) marks.add(List.of(x & 15, y, z & 15));
                            if (sid != 0 && sid != 1 && sid != 86 && sid != 102 && density <= 0) nonpositiveVeins++;
                            List<String> values = new ArrayList<>(); values.add(bits(density));
                            signalBytes[0].putLong(index * 8, Double.doubleToRawLongBits(density));
                            for (int i = 0; i < 3; i++) {
                                double value = (double) compute.invoke(functions.get(i), nc);
                                signalBytes[i + 1].putLong(index * 8, Double.doubleToRawLongBits(value));
                                values.add(bits(value));
                            }
                            if ((x & 15) == (z & 15) && Set.of(-64, -61, -60, -59, -56, -55, -54, -9, -8, -7, -1, 0, 1, 49, 50, 51, 55, 56, 63, 64, 319).contains(y))
                                samples.add(Map.of("pos", List.of(x, y, z), "bits", values, "state", sid));
                        }
                    }
                }
            }
            call(nc, "swapSlices");
        }
        call(nc, "stopInterpolation");
        Object[] nativeMarks = (Object[]) call(selected.chunk, "getPostProcessing");
        for (int s = 0; s < nativeMarks.length; s++) {
            List<Integer> expected = new ArrayList<>(), actual = new ArrayList<>();
            if (nativeMarks[s] != null) for (Object packed : (Iterable<?>) nativeMarks[s]) expected.add(((Short) packed).intValue() & 0xffff);
            for (List<Integer> mark : marks) if ((mark.get(1) + 64) >> 4 == s) actual.add(mark.get(0) | ((mark.get(1) & 15) << 4) | (mark.get(2) << 8));
            if (!expected.equals(actual)) throw new AssertionError("native postprocessing order " + id + " section " + s);
        }
        List<List<Integer>> runs = new ArrayList<>();
        for (int i = 0; i < states.length;) {
            int end = i + 1; while (end < states.length && states[end] == states[i]) end++;
            runs.add(List.of(states[i], end - i)); i = end;
        }
        List<String> hashes = new ArrayList<>(); for (ByteBuffer b : signalBytes) hashes.add(md5(b.array()));
        List<Integer> heights = new ArrayList<>();
        for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) heights.add((int) call(selected.chunk, "getHeight",
            field("world.level.levelgen.Heightmap$Types", "WORLD_SURFACE_WG"), x, z) + 1);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("id", id); result.put("seed", seed); result.put("chunk", List.of(cx, cz)); result.put("ore_veins_enabled", ores);
        result.put("starts", startData(selected.starts)); result.put("selected", selectedData(selected.beard));
        result.put("runs", runs); result.put("marks", marks); result.put("signal_md5", hashes); result.put("samples", samples);
        result.put("world_surface_wg", heights); result.put("nonpositive_density_veins", nonpositiveVeins);
        return result;
    }

    static Object noise() throws Exception {
        List<Object> cases = new ArrayList<>();
        for (long seed : new long[]{0, -1, 846692123413862008L}) {
            int[] c = findVeinChunk(seed, true, -1024, -512);
            int[] i = findVeinChunk(seed, false, -1024, -512);
            cases.add(noiseCase("copper/" + seed, seed, c[0], c[1], List.of(), true));
            cases.add(noiseCase("copper-boundary/" + seed, seed, c[0] - 1, c[1], List.of(), true));
            cases.add(noiseCase("iron/" + seed, seed, i[0], i[1], List.of(), true));
        }
        for (long seed : new long[]{Long.MIN_VALUE, Long.MAX_VALUE})
            cases.add(noiseCase("extreme/" + seed, seed, 1874999, -1874999, syntheticStarts(29999984, -29999984, 62), true));
        List<Object> overlap = syntheticStarts(-16, -16, 63);
        cases.add(noiseCase("overlap", 1234, -1, -1, overlap, true));
        cases.add(noiseCase("overlap-no-ores", 1234, -1, -1, overlap, false));
        for (String name : List.of("minecraft:village_plains", "minecraft:ancient_city")) {
            Object actual = assembledStart(name, 846692123413862008L, -2, 1);
            for (int dx = 0; dx <= 1; dx++) cases.add(noiseCase(name + "/" + dx, 846692123413862008L, -2 + dx, 1, List.of(actual), true));
        }
        return Map.of("cases", cases, "states", describeStates(), "signal_order", List.of("full_density", "vein_toggle", "vein_ridged", "vein_gap"));
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        settings = call(call(registry("NOISE_SETTINGS"), "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD")), "value");
        noises = registry("NOISE");
        registryOps = call(type("resources.RegistryOps"), "create", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), registries);
        densityClass = type("world.level.levelgen.DensityFunction");
        contextClass = type("world.level.levelgen.DensityFunction$FunctionContext");
        randomClass = type("util.RandomSource");
        compute = densityClass.getMethod("compute", contextClass);
        createOre = method("world.level.levelgen.OreVeinifier", "create", densityClass, densityClass, densityClass,
            type("world.level.levelgen.PositionalRandomFactory"));
        calculateOre = type("world.level.levelgen.NoiseChunk$BlockStateFiller").getMethod("calculate", contextClass);
        Object lava = make("world.level.levelgen.Aquifer$FluidStatus", -54, state("LAVA"));
        Object water = make("world.level.levelgen.Aquifer$FluidStatus", 63, state("WATER"));
        picker = Proxy.newProxyInstance(DensityMaterialsReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.Aquifer$FluidPicker")}, (p, m, a) -> {
                if (m.getName().equals("computeFluid")) return (int) a[1] < -54 ? lava : water;
                throw new UnsupportedOperationException(m.toString());
            });
        Object result = switch (args[0]) {
            case "describe" -> Map.of("states", describeStates());
            case "materials" -> materials();
            case "beard" -> beard();
            case "noise" -> noise();
            default -> throw new IllegalArgumentException(args[0]);
        };
        output("DENSITY_MATERIALS_REFERENCE", result);
        call(resources, "close");
    }
}
