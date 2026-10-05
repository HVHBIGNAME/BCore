import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.function.Predicate;
import java.util.jar.JarFile;

/** Calls the pinned JAR's admission, StructureStart and StructurePiece methods. */
public class ScatteredStructureReference extends StructureRuntimeReference {
    static final Map<String,String> SETS = Map.of(
        "buried_treasure", "buried_treasures", "swamp_hut", "swamp_huts", "jungle_pyramid", "jungle_temples");
    static final Map<String,String> BIOMES = Map.of("buried_treasure", "beach", "swamp_hut", "swamp", "jungle_pyramid", "jungle");
    static final List<String> KINDS = List.of("buried_treasure", "swamp_hut", "jungle_pyramid");
    static final Comparator<Pos> POS_ORDER = Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z);
    static Map<String,Object> stateCache = new HashMap<>();
    static Map<Pos,Object> overrides;
    static Set<Pos> denied;
    static List<Object> effects, heightQueries;
    static Map<Pos,String> observedLoot;
    static Map<String,Integer> observedMarks;
    static Object activePiece;
    static Object disabledEntityLevel;
    static boolean suppressBlockEntities;
    static int worldMinY = -64;

    static Object cached(String name) throws Exception {
        Object result = stateCache.get(name);
        if (result == null) { result = state(name); stateCache.put(name, result); }
        return result;
    }

    static Object blockAt(Pos p) throws Exception {
        if (p.y() < worldMinY || p.y() > 319) return cached("AIR");
        Object result = placed.get(p);
        if (result != null) return result;
        result = overrides.get(p);
        if (result != null) return result;
        return switch (terrain) {
            case "beach" -> cached(p.y() < 53 ? "STONE" : p.y() < 63 ? "SAND" : p.y() < 65 ? "WATER" : "AIR");
            case "water" -> cached(p.y() < 58 ? "STONE" : p.y() < 64 ? "WATER" : "AIR");
            case "slope" -> cached(p.y() < 60 + Math.floorMod(p.x() + 2 * p.z(), 9) ? "STONE" : "AIR");
            case "dirt" -> cached(p.y() < 64 ? "DIRT" : "AIR");
            case "deepslate" -> cached(p.y() < 64 ? "DEEPSLATE" : "AIR");
            case "void" -> cached("AIR");
            default -> cached(p.y() < 64 ? "STONE" : "AIR");
        };
    }

    static int heightAt(Object kind, int x, int z) throws Exception {
        Predicate<Object> predicate = (Predicate<Object>)call(kind, "isOpaque");
        int result = worldMinY;
        for (int y = 319; y >= worldMinY; y--) {
            if (predicate.test(blockAt(new Pos(x, y, z)))) { result = y + 1; break; }
        }
        heightQueries.add(List.of(((Enum<?>)kind).name(), x, z, result));
        return result;
    }

    static void observeEffects() throws Exception {
        // Read actual container mutations before the next world callback; this
        // preserves loot assignment order without substituting a Java algorithm.
        List<Pos> positions = new ArrayList<>(blockEntities.keySet()); positions.sort(POS_ORDER);
        for (Pos p : positions) {
            Object entity = blockEntities.get(p);
            if (!type("world.RandomizableContainer").isInstance(entity)) continue;
            Object table = call(entity, "getLootTable");
            if (table == null) continue;
            long seed = (long)call(entity, "getLootTableSeed");
            String name = call(table, "identifier").toString();
            String signature = name + "/" + seed;
            if (!signature.equals(observedLoot.put(p, signature))) {
                String entityId = call(field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE"), "getKey", call(entity, "getType")).toString();
                effects.add(List.of("loot", p.x(), p.y(), p.z(), entityId, name, seed));
            }
        }
        Method unpack = type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates", short.class, int.class, type("world.level.ChunkPos"));
        for (var entry : markChunks.entrySet()) {
            Object chunk = entry.getValue(); Object[] sections = (Object[])call(chunk, "getPostProcessing");
            for (int s = 0; s < sections.length; s++) if (sections[s] != null) {
                String key = entry.getKey() + "/" + s;
                int seen = observedMarks.getOrDefault(key, 0), index = 0;
                for (Object packed : (Iterable<?>)sections[s]) {
                    if (index++ < seen) continue;
                    Pos p = Pos.from(unpack.invoke(null, packed, s + (worldMinY >> 4), call(chunk, "getPos")));
                    effects.add(List.of("mark", p.x(), p.y(), p.z()));
                }
                observedMarks.put(key, index);
            }
        }
    }

    static boolean allowed(Pos p) {
        return p.y() >= worldMinY && p.y() <= 319
            && Math.abs((p.x() >> 4) - writeSourceX) <= 1 && Math.abs((p.z() >> 4) - writeSourceZ) <= 1
            && !denied.contains(p);
    }

    static Object scatteredWorld() throws Exception {
        return Proxy.newProxyInstance(ScatteredStructureReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            try {
                observeEffects();
                return switch (m.getName()) {
                    case "getMinY" -> worldMinY;
                    case "getMaxY" -> 319;
                    case "getMinSectionY" -> worldMinY >> 4;
                    case "getSeed" -> worldSeed;
                    case "getHeight" -> a == null || a.length == 0 ? 320 - worldMinY : heightAt(a[0], (int)a[1], (int)a[2]);
                    case "getHeightmapPos" -> { Pos at = Pos.from(a[1]); yield make("core.BlockPos", at.x(), heightAt(a[0], at.x(), at.z()), at.z()); }
                    case "getBlockState" -> blockAt(Pos.from(a[0]));
                    case "getFluidState" -> call(blockAt(Pos.from(a[0])), "getFluidState");
                    case "isEmptyBlock" -> call(blockAt(Pos.from(a[0])), "isAir");
                    case "isStateAtPosition" -> ((Predicate<Object>)a[1]).test(blockAt(Pos.from(a[0])));
                    case "isFluidAtPosition" -> ((Predicate<Object>)a[1]).test(call(blockAt(Pos.from(a[0])), "getFluidState"));
                    case "isOutsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : Pos.from(a[0]).y(); yield y < worldMinY || y > 319; }
                    case "ensureCanWrite" -> allowed(Pos.from(a[0]));
                    case "registryAccess" -> registries;
                    case "holderLookup" -> call(registries, "lookupOrThrow", a[0]);
                    case "isClientSide" -> false;
                    case "getChunk" -> {
                        if (a.length == 1) { Pos at = Pos.from(a[0]); yield markChunk(at.x() >> 4, at.z() >> 4); }
                        yield markChunk((int)a[0], (int)a[1]);
                    }
                    case "setBlock" -> {
                        Pos at = Pos.from(a[0]); boolean accept = allowed(at);
                        effects.add(List.of("block", at.x(), at.y(), at.z(), stateId(a[1]), a[2], accept));
                        if (!accept) yield false;
                        Object previous = blockAt(at), block = call(a[1], "getBlock");
                        placed.put(at, a[1]); writes.add(List.of(at.x(), at.y(), at.z(), stateId(a[1]), (int)a[2]));
                        if (!(boolean)call(a[1], "hasBlockEntity")) { blockEntities.remove(at); observedLoot.remove(at); }
                        else if (!suppressBlockEntities && (!blockEntities.containsKey(at) || call(previous, "getBlock") != block)) {
                            Object entity = call(block, "newBlockEntity", a[0], a[1]);
                            if (entity != null) { blockEntities.put(at, entity); observedLoot.remove(at); }
                        }
                        // WorldGenRegion records this native state-selected mark
                        // after setBlock whenever UPDATE_KNOWN_SHAPE is absent.
                        if (((int)a[2] & 16) == 0) {
                            Object post = call(a[1], "getPostProcessPos", p, a[0]);
                            if (post != null) call(markChunk(at.x() >> 4, at.z() >> 4), "markPosForPostprocessing", post);
                        }
                        yield true;
                    }
                    case "getBlockEntity" -> {
                        Object entity = blockEntities.get(Pos.from(a[0]));
                        if (a.length == 2) yield entity != null && call(entity, "getType") == a[1] ? Optional.of(entity) : Optional.empty();
                        yield entity;
                    }
                    case "scheduleTick" -> {
                        boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                        Object registry = fluid ? field("core.registries.BuiltInRegistries", "FLUID") : blockRegistry;
                        Pos at = Pos.from(a[0]); int id = (int)call(registry, "getId", a[1]);
                        effects.add(List.of("tick", at.x(), at.y(), at.z(), fluid ? "fluid" : "block", id, a[2]));
                        ticks.add(List.of(xyz(a[0]), fluid ? "fluid" : "block", id, a[2])); yield null;
                    }
                    case "getLevel" -> {
                        String caller = StackWalker.getInstance().walk(s -> s.filter(f -> f.getClassName().endsWith("SwampHutPiece"))
                            .map(StackWalker.StackFrame::getMethodName).findFirst().orElseThrow());
                        String mob = caller.equals("spawnCat") ? "minecraft:cat" : "minecraft:witch";
                        Object at = call(activePiece, "getWorldPos", 2, 2, 5);
                        effects.add(List.of("mob_request", mob, xyz(at)));
                        // The native feature gate returns null before the entity
                        // constructor. The piece still executes both request paths.
                        yield disabledEntityLevel;
                    }
                    case "addFreshEntityWithPassengers" -> throw new UnsupportedOperationException("unexpected successful entity factory");
                    case "toString" -> "ScatteredStructureReference controlled native world";
                    default -> throw new UnsupportedOperationException(m.toString());
                };
            } catch (InvocationTargetException e) { throw e.getCause(); }
        });
    }

    static Object structure(String kind) throws Exception { return call(holder("STRUCTURE", kind), "value"); }

    static void prepareDisabledEntityLevel() throws Exception {
        Object flags = call(type("world.flag.FeatureFlagSet"), "of");
        Object data = Proxy.newProxyInstance(ScatteredStructureReference.class.getClassLoader(), new Class<?>[]{type("world.level.storage.WorldData")}, (p, m, a) -> {
            if (m.getName().equals("enabledFeatures")) return flags;
            throw new UnsupportedOperationException(m.toString());
        });
        Object server = allocate(type("server.dedicated.DedicatedServer"));
        Field f = type("server.MinecraftServer").getDeclaredField("worldData"); f.setAccessible(true); f.set(server, data);
        disabledEntityLevel = allocate(type("server.level.ServerLevel"));
        setMember(disabledEntityLevel, "server", server);
        for (String mob : List.of("WITCH", "CAT")) {
            if (call(field("world.entity.EntityType", mob), "create", disabledEntityLevel, field("world.entity.EntitySpawnReason", "STRUCTURE")) != null)
                throw new IllegalStateException("expected disabled native entity factory");
        }
    }

    static Map<String,Object> jarJson(JarFile jar, String path) throws Exception {
        byte[] bytes = jar.getInputStream(jar.getJarEntry(path)).readAllBytes();
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        return Map.of("sha256", HexFormat.of().formatHex(java.security.MessageDigest.getInstance("SHA-256").digest(bytes)),
            "value", call(gson, "fromJson", new String(bytes, StandardCharsets.UTF_8), Object.class));
    }

    static List<Object> catalog(String jarPath) throws Exception {
        List<Object> rows = new ArrayList<>();
        try (JarFile jar = new JarFile(jarPath)) {
            for (String kind : KINDS) {
                Object value = structure(kind); List<Object> biomes = new ArrayList<>();
                for (Object h : (Iterable<?>)call(value, "biomes")) {
                    Object b = call(h, "value"); biomes.add(List.of(call(registry("BIOME"), "getId", b), call(registry("BIOME"), "getKey", b).toString()));
                }
                Map<?,?> meta = (Map<?,?>)metadata().stream().filter(r -> ((Map<?,?>)r).get("structure").equals("minecraft:" + kind)).findFirst().orElseThrow();
                rows.add(Map.of("kind", kind, "set", SETS.get(kind), "biomes", biomes,
                    "structure_id", call(registry("STRUCTURE"), "getId", value), "step", meta.get("step"), "index", meta.get("index"),
                    "config", jarJson(jar, "data/minecraft/worldgen/structure/" + kind + ".json"),
                    "placement", jarJson(jar, "data/minecraft/worldgen/structure_set/" + SETS.get(kind) + ".json")));
            }
        }
        return rows;
    }

    static Map<String,Object> blockProperties() throws Exception {
        Object piece = make("world.level.levelgen.structure.structures.SwampHutPiece", make("world.level.levelgen.LegacyRandomSource", 0L), 0, 0);
        Field shapeField = type("world.level.levelgen.structure.StructurePiece").getDeclaredField("SHAPE_CHECK_BLOCKS"); shapeField.setAccessible(true);
        Set<?> shapeChecks = (Set<?>)shapeField.get(null);
        Predicate<Object> floor = (Predicate<Object>)call(field("world.level.levelgen.Heightmap$Types", "OCEAN_FLOOR_WG"), "isOpaque");
        Predicate<Object> motion = (Predicate<Object>)call(field("world.level.levelgen.Heightmap$Types", "MOTION_BLOCKING_NO_LEAVES"), "isOpaque");
        List<Object> ranges = new ArrayList<>(); int start = 0, lastFlags = -1, lastFluid = -1, count = 0;
        for (Object state : (Iterable<?>)field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            int id = stateId(state), flags = 0;
            if ((boolean)call(state, "isAir")) flags |= 1;
            if ((boolean)call(state, "liquid")) flags |= 2;
            if ((boolean)call(state, "isSolidRender")) flags |= 4;
            if ((boolean)call(piece, "isReplaceableByStructures", state)) flags |= 8;
            if (floor.test(state)) flags |= 16;
            if (motion.test(state)) flags |= 32;
            if (shapeChecks.contains(call(state, "getBlock"))) flags |= 64;
            Object fluid = call(state, "getFluidState");
            int fluidId = (int)call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", call(fluid, "getType"));
            if (id != count++) throw new IllegalStateException("block registry order");
            if (id > 0 && (flags != lastFlags || fluidId != lastFluid)) { ranges.add(List.of(start, id, lastFlags, lastFluid)); start = id; }
            lastFlags = flags; lastFluid = fluidId;
        }
        ranges.add(List.of(start, count, lastFlags, lastFluid));
        return Map.of("state_count", count, "ranges", ranges);
    }

    static List<Object> chestOrientations() throws Exception {
        List<Object> rows = new ArrayList<>();
        List<Object> directions = new ArrayList<>();
        for (Object direction : (Iterable<?>)field("core.Direction$Plane", "HORIZONTAL")) directions.add(direction);
        Object position = make("core.BlockPos", -1, 64, -17);
        for (int mask = 0; mask < 16; mask++) for (int chest = -1; chest < 4; chest++) {
            freshWorld("flat", 0L);
            for (int d = 0; d < 4; d++) {
                Pos p = Pos.from(call(position, "relative", directions.get(d)));
                overrides.put(p, cached(chest == d ? "CHEST" : (mask & (1 << d)) != 0 ? "STONE" : "GLASS"));
            }
            Object result = call(type("world.level.levelgen.structure.StructurePiece"), "reorient", scatteredWorld(), position, cached("CHEST"));
            rows.add(Map.of("mask", mask, "neighbour_chest", chest, "position", xyz(position), "overrides", overrideRows(), "state", stateId(result)));
        }
        return rows;
    }

    static Map<String,Object> admissionCase(String kind, String biome, int x, int z) throws Exception {
        Map<String,Object> result = new LinkedHashMap<>(admission(SETS.get(kind), biome, x, z));
        Object value = structure(kind), place = call(call(holder("STRUCTURE_SET", SETS.get(kind)), "value"), "placement");
        result.put("kind", kind);
        result.put("candidate", call(place, "isStructureChunk", structureState, x, z));
        Object potential = call(place, "getPotentialStructureChunk", worldSeed, x, z);
        result.put("potential", List.of(call(potential, "x"), call(potential, "z")));
        Object context = make("world.level.levelgen.structure.Structure$GenerationContext", registries, generator,
            call(generator, "getBiomeSource"), randomState, templateManager, worldSeed, make("world.level.ChunkPos", x, z),
            heightAccessor, (Predicate<Object>)h -> { try { return (boolean)call(call(value, "biomes"), "contains", h); } catch(Exception e) { throw new RuntimeException(e); } });
        Object hm = field("world.level.levelgen.Heightmap$Types", kind.equals("buried_treasure") ? "OCEAN_FLOOR_WG" : "WORLD_SURFACE_WG");
        int y = (int)call(generator, "getBaseHeight", x * 16 + 8, z * 16 + 8, hm, heightAccessor, randomState);
        Object noiseBiome = call(call(generator, "getBiomeSource"), "getNoiseBiome", (x * 16 + 8) >> 2, (y - 1) >> 2, (z * 16 + 8) >> 2, call(randomState, "sampler"));
        result.put("first_free", y);
        result.put("sea_level", call(generator, "getSeaLevel"));
        if (kind.equals("jungle_pyramid")) {
            List<Object> corners = new ArrayList<>();
            for (int[] delta : new int[][]{{0,0},{0,15},{12,0},{12,15}}) {
                int bx = x * 16 + delta[0], bz = z * 16 + delta[1];
                corners.add(List.of(bx, bz, call(generator, "getBaseHeight", bx, bz, hm, heightAccessor, randomState)));
            }
            result.put("corner_first_free", corners);
            result.put("native_lowest_y", call(type("world.level.levelgen.structure.Structure"), "getLowestY", context, 12, 15));
        }
        result.put("noise_biome", call(registry("BIOME"), "getId", call(noiseBiome, "value")));
        Optional<?> stub = (Optional<?>)call(value, "findValidGenerationPoint", context);
        result.put("biome_admitted", stub.isPresent());
        if (stub.isPresent()) {
            Object s = stub.orElseThrow(); result.put("generation_point", xyz(call(s, "position")));
            Object start = make("world.level.levelgen.structure.StructureStart", value, make("world.level.ChunkPos", x, z), 0, call(call(s, "getPiecesBuilder"), "build"));
            result.put("assembled", Map.of("nbt", nbt64(call(start, "createTag", serialContext, make("world.level.ChunkPos", x, z)))));
        }
        result.put("next_i64", call(call(context, "random"), "nextLong"));
        return result;
    }

    static List<Object> scatteredAdmissions() throws Exception {
        List<Object> rows = new ArrayList<>();
        for (String kind : KINDS) for (long seed : new long[]{0, 1, 42, -17}) {
            for (String biome : List.of(BIOMES.get(kind), "plains")) {
                setup(seed, biome, SETS.get(kind));
                if (kind.equals("buried_treasure")) {
                    Object place = call(call(holder("STRUCTURE_SET", SETS.get(kind)), "value"), "placement");
                    int accepted = 0, rejected = 0;
                    for (int x = -32; x <= 32 && accepted < 2; x++) for (int z = -32; z <= 32 && accepted < 2; z++) {
                        boolean candidate = (boolean)call(place, "isStructureChunk", structureState, x, z);
                        if (candidate || rejected++ < 2) rows.add(admissionCase(kind, biome, x, z));
                        if (candidate) accepted++;
                    }
                    if (accepted != 2) throw new IllegalStateException("treasure frequency search");
                } else {
                    for (int rx : new int[]{-1, 0}) {
                        Object pos = candidate(SETS.get(kind), rx, -1);
                        int x = (int)call(pos, "x"), z = (int)call(pos, "z");
                        rows.add(admissionCase(kind, biome, x, z)); rows.add(admissionCase(kind, biome, x + 1, z));
                    }
                }
            }
        }
        for (String kind : KINDS) {
            setup(846692123413862008L, "overworld", SETS.get(kind));
            Object place = call(call(holder("STRUCTURE_SET", SETS.get(kind)), "value"), "placement");
            int accepted = 0, rejected = 0, max = kind.equals("buried_treasure") ? 160 : 14;
            for (int radius = 0; radius <= max && accepted < 2; radius++) {
                for (int rx = -radius; rx <= radius && accepted < 2; rx++) for (int rz = -radius; rz <= radius && accepted < 2; rz++) {
                    if (Math.max(Math.abs(rx), Math.abs(rz)) != radius) continue;
                    Object pos = candidate(SETS.get(kind), rx, rz);
                    int x = (int)call(pos, "x"), z = (int)call(pos, "z");
                    if (!(boolean)call(place, "isStructureChunk", structureState, x, z)) continue;
                    Map<String,Object> row = admissionCase(kind, "overworld", x, z);
                    if (!((List<?>)row.get("starts")).isEmpty()) { rows.add(row); accepted++; }
                    else if (rejected++ < 3) rows.add(row);
                }
            }
            if (accepted < 2) throw new IllegalStateException("no two real overworld starts: " + kind);
        }
        return rows;
    }

    static List<Object> jungleAdmissions() throws Exception {
        List<Object> rows = new ArrayList<>();
        String kind = "jungle_pyramid", set = SETS.get(kind);
        for (long seed : new long[]{0, 1, 42, -17, Long.MIN_VALUE, Long.MAX_VALUE}) {
            for (String biome : List.of("jungle", "bamboo_jungle", "plains")) {
                setup(seed, biome, set);
                for (int rx : new int[]{-1, 0}) {
                    Object p = candidate(set, rx, -1); int x = (int)call(p,"x"), z = (int)call(p,"z");
                    rows.add(admissionCase(kind, biome, x, z)); rows.add(admissionCase(kind, biome, x + 1, z));
                }
            }
        }
        // Unmodified flat generators below, at, and above their native sea level.
        for (int layers : new int[]{0, 1, 2, 3, 126, 127, 128, 129}) {
            setup(42, "jungle", set);
            Object settings = make("world.level.levelgen.flat.FlatLevelGeneratorSettings", Optional.empty(), holder("BIOME", "jungle"), List.of());
            if (layers > 0) ((List<Object>)call(settings, "getLayersInfo")).add(make("world.level.levelgen.flat.FlatLayerInfo", layers, field("world.level.block.Blocks", "STONE")));
            call(settings,"updateLayers"); generator = make("world.level.levelgen.FlatLevelSource", settings);
            Object p = candidate(set, -1, 0);
            Map<String,Object> row = admissionCase(kind, "jungle", (int)call(p,"x"), (int)call(p,"z"));
            row.put("flat_layers", layers); rows.add(row);
        }
        setup(846692123413862008L, "overworld", set);
        int accepted = 0, rejected = 0;
        for (int radius = 0; radius <= 18 && accepted < 2; radius++) {
            for (int rx = -radius; rx <= radius && accepted < 2; rx++) for (int rz = -radius; rz <= radius && accepted < 2; rz++) {
                if (Math.max(Math.abs(rx),Math.abs(rz)) != radius) continue;
                Object p = candidate(set, rx, rz);
                Map<String,Object> row = admissionCase(kind, "overworld", (int)call(p,"x"), (int)call(p,"z"));
                if (!((List<?>)row.get("starts")).isEmpty()) { rows.add(row); accepted++; }
                else if (rejected++ < 4) rows.add(row);
            }
        }
        if (accepted != 2) throw new IllegalStateException("two native overworld jungle starts required");
        return rows;
    }

    static void freshWorld(String mode, long seed) {
        terrain = mode; worldSeed = seed;
        placed = new HashMap<>(); blockEntities = new HashMap<>(); overrides = new HashMap<>(); denied = new HashSet<>();
        writes = new ArrayList<>(); ticks = new ArrayList<>(); markChunks = new HashMap<>();
        effects = new ArrayList<>(); heightQueries = new ArrayList<>(); observedLoot = new HashMap<>(); observedMarks = new HashMap<>();
        suppressBlockEntities = false; worldMinY = -64;
    }

    static List<List<Integer>> overrideRows() throws Exception {
        List<Pos> positions = new ArrayList<>(overrides.keySet()); positions.sort(POS_ORDER);
        List<List<Integer>> rows = new ArrayList<>();
        for (Pos p : positions) rows.add(List.of(p.x(), p.y(), p.z(), stateId(overrides.get(p))));
        return rows;
    }

    static Map<String,Object> placementCase(String name, String kind, long seed, int x, int z,
        String mode, int shiftX, int shiftZ, int[][] sources, String special) throws Exception {
        configureFlat(); freshWorld(mode, seed);
        Object start = nativeStart("minecraft:" + kind, seed, x, z), cp = make("world.level.ChunkPos", x, z);
        activePiece = ((List<?>)call(start, "getPieces")).get(0);
        if (shiftX != 0 || shiftZ != 0) call(activePiece, "move", shiftX, 0, shiftZ);
        Map<String,Object> row = new LinkedHashMap<>();
        row.put("name", name); row.put("kind", kind); row.put("seed", seed); row.put("chunk", List.of(x, z));
        Object direction = call(activePiece, "getOrientation");
        row.put("orientation", direction == null ? "none" : direction.toString());
        row.put("shift", List.of(shiftX, 0, shiftZ)); row.put("terrain", mode);
        row.put("initial", Map.of("nbt", nbt64(call(start, "createTag", serialContext, cp))));
        row.put("reference_bounds", bounds(call(start, "getBoundingBox")));
        if (special.equals("missing_entity")) suppressBlockEntities = true;
        if (special.equals("deny_containers")) {
            for (int[] local : new int[][]{{3,-2,1},{9,-2,3},{8,-3,3},{9,-3,10}})
                denied.add(Pos.from(call(activePiece, "getWorldPos", local[0],local[1],local[2])));
        }
        if (special.equals("deny_fluid")) {
            for (int y = 64; y <= 69; y++) for (int dx = -1; dx <= 10; dx++) for (int dz = -1; dz <= 10; dz++) {
                Pos p = new Pos(x * 16 + shiftX + dx, y, z * 16 + shiftZ + dz);
                overrides.put(p, cached("WATER")); denied.add(p);
            }
            // Retain the native starting HPos, avoiding terrain feedback here.
            Field h = type("world.level.levelgen.structure.ScatteredFeaturePiece").getDeclaredField("heightPosition"); h.setAccessible(true); h.setInt(activePiece, 64);
            row.put("initial", Map.of("nbt", nbt64(call(start, "createTag", serialContext, cp))));
        }
        if (special.equals("plants")) {
            int i = 0;
            for (int dz : new int[]{2, 7}) for (int dx : new int[]{1, 5}) {
                Pos p = Pos.from(call(activePiece, "getWorldPos", dx, -1, dz));
                overrides.put(p, cached(new String[]{"SEAGRASS", "TALL_SEAGRASS", "GLOW_LICHEN", "KELP"}[i++]));
            }
        }
        if (kind.equals("buried_treasure")) {
            List<Integer> bb = bounds(call(activePiece, "getBoundingBox")); int bx = bb.get(0), bz = bb.get(2);
            if (special.equals("pocket")) {
                for (int dx = -1; dx <= 1; dx++) for (int dz = -1; dz <= 1; dz++) {
                    if (dx == 0 && dz == 0) continue;
                    overrides.put(new Pos(bx + dx, 52, bz + dz), cached("WATER"));
                    overrides.put(new Pos(bx + dx, 53, bz + dz), cached("WATER"));
                }
                overrides.put(new Pos(bx, 54, bz), cached("AIR"));
            }
            if (special.equals("waterlogged")) {
                Object fence = call(cached("OAK_FENCE"), "setValue", field("world.level.block.state.properties.BlockStateProperties", "WATERLOGGED"), true);
                overrides.put(new Pos(bx + 1, 64, bz), fence);
            }
            if (special.equals("already_chest")) overrides.put(new Pos(bx, 64, bz), cached("CHEST"));
            if (special.equals("missing_entity")) suppressBlockEntities = true;
            if (special.startsWith("substrate_")) overrides.put(new Pos(bx, 63, bz), cached(special.substring(10)));
            if (special.equals("lava")) {
                Object flowing = call(cached("LAVA"), "setValue", field("world.level.block.state.properties.BlockStateProperties", "LEVEL"), 3);
                for (int dx = -1; dx <= 1; dx++) for (int dz = -1; dz <= 1; dz++) overrides.put(new Pos(bx + dx, 64, bz + dz), flowing);
            }
            if (special.equals("bottom")) {
                worldMinY = 60;
                overrides.put(new Pos(bx, 60, bz), cached("STONE"));
                for (int y = 61; y < 64; y++) overrides.put(new Pos(bx, y, bz), cached("DIRT"));
            }
        }
        row.put("overrides", overrideRows());
        List<Pos> denial = new ArrayList<>(denied); denial.sort(POS_ORDER);
        row.put("denied", denial.stream().map(p -> List.of(p.x(), p.y(), p.z())).toList());
        row.put("suppress_block_entities", suppressBlockEntities); row.put("min_y", worldMinY);
        List<Object> passes = new ArrayList<>();
        Map<?,?> meta = (Map<?,?>)metadata().stream().filter(r -> ((Map<?,?>)r).get("structure").equals("minecraft:" + kind)).findFirst().orElseThrow();
        for (int[] source : sources) {
            writeSourceX = source[0]; writeSourceZ = source[1];
            writes = new ArrayList<>(); ticks = new ArrayList<>(); effects = new ArrayList<>(); heightQueries = new ArrayList<>();
            markChunks = new HashMap<>(); observedMarks = new HashMap<>();
            Object clip = make("world.level.levelgen.structure.BoundingBox", source[0] * 16, worldMinY + 1, source[1] * 16, source[0] * 16 + 15, 319, source[1] * 16 + 15);
            if (source.length == 4) clip = make("world.level.levelgen.structure.BoundingBox", source[0] * 16, source[2], source[1] * 16, source[0] * 16 + 15, source[3], source[1] * 16 + 15);
            Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", 0L));
            long decorationSeed = (long)call(random, "setDecorationSeed", seed, source[0] * 16, source[1] * 16);
            call(random, "setFeatureSeed", decorationSeed, meta.get("index"), meta.get("step"));
            call(start, "placeInChunk", scatteredWorld(), null, generator, random, clip, make("world.level.ChunkPos", source[0], source[1]));
            observeEffects();
            Map<String,Object> pass = new LinkedHashMap<>(runtimeSnapshot());
            pass.put("source", List.of(source[0], source[1])); pass.put("clip", bounds(clip));
            pass.put("effects", new ArrayList<>(effects)); pass.put("height_queries", new ArrayList<>(heightQueries));
            pass.put("decoration_seed", decorationSeed); pass.put("next_i64", call(random, "nextLong"));
            Object tag = call(start, "createTag", serialContext, cp);
            pass.put("after", Map.of("nbt", nbt64(tag))); pass.put("cached_reference_bounds", bounds(call(start, "getBoundingBox")));
            Object reloaded = call(type("world.level.levelgen.structure.StructureStart"), "loadStaticStart", serialContext, tag, seed);
            pass.put("reloaded_reference_bounds", bounds(call(reloaded, "getBoundingBox")));
            passes.add(pass);
        }
        row.put("passes", passes);
        return row;
    }

    static List<Object> scatteredPlacements() throws Exception {
        List<Object> rows = new ArrayList<>();
        for (long seed : new long[]{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 42, -17}) {
            rows.add(placementCase("hut_" + seed, "swamp_hut", seed, -2, 3, "flat", 0, 0, new int[][]{{-2, 3}}, ""));
        }
        for (String mode : List.of("slope", "water", "void")) {
            rows.add(placementCase("hut_" + mode, "swamp_hut", 42, -1, -1, mode, 13, 13,
                new int[][]{{-1, -1}, {0, -1}, {-1, 0}, {0, 0}, {-1, -1}}, ""));
        }
        rows.add(placementCase("hut_reverse", "swamp_hut", 42, -1, -1, "slope", 13, 13,
            new int[][]{{0, 0}, {-1, 0}, {0, -1}, {-1, -1}, {0, 0}}, ""));
        rows.add(placementCase("hut_denied_fluid", "swamp_hut", 1, 0, 0, "flat", 0, 0, new int[][]{{0, 0}}, "deny_fluid"));
        rows.add(placementCase("hut_vertical_clip", "swamp_hut", 0, 0, 0, "flat", 0, 0, new int[][]{{0, 0, 65, 319}}, ""));
        rows.add(placementCase("hut_plants", "swamp_hut", 4, -2, 3, "water", 0, 0, new int[][]{{-2, 3}}, "plants"));
        for (String mode : List.of("flat", "beach", "water", "dirt", "deepslate", "void")) {
            rows.add(placementCase("treasure_" + mode, "buried_treasure", 42, -2, 3, mode, 0, 0, new int[][]{{-2, 3}, {-2, 3}}, ""));
        }
        for (String special : List.of("pocket", "waterlogged", "already_chest", "missing_entity", "bottom", "lava",
            "substrate_SANDSTONE", "substrate_ANDESITE", "substrate_GRANITE", "substrate_DIORITE")) {
            rows.add(placementCase("treasure_" + special, "buried_treasure", -17, -1, -1, special.equals("pocket") ? "beach" : "flat", 0, 0, new int[][]{{-1, -1}}, special));
        }
        rows.add(placementCase("treasure_clip_edge", "buried_treasure", 0, -1, -1, "flat", 6, 6, new int[][]{{-1, -1}}, ""));
        return rows;
    }

    static List<Object> junglePlacements() throws Exception {
        List<Object> rows = new ArrayList<>();
        for (long seed : new long[]{0, 1, 4, 6, 7, 9, 42, -17}) {
            rows.add(placementCase("jungle_" + seed, "jungle_pyramid", seed, -2, 3, "flat", 0, 0,
                new int[][]{{-2,3},{-2,3},{-1,3}}, ""));
        }
        for (String mode : List.of("slope", "water", "void")) {
            rows.add(placementCase("jungle_" + mode, "jungle_pyramid", 42, -1, -1, mode, 13, 13,
                new int[][]{{-1,-1},{0,-1},{-1,0},{0,0},{-1,-1}}, ""));
        }
        rows.add(placementCase("jungle_reverse", "jungle_pyramid", 42, -1, -1, "slope", 13, 13,
            new int[][]{{0,0},{-1,0},{0,-1},{-1,-1},{0,0}}, ""));
        for (String special : List.of("missing_entity", "deny_containers")) {
            rows.add(placementCase("jungle_" + special, "jungle_pyramid", -17, -2, 3, "flat", 0, 0,
                new int[][]{{-2,3},{-2,3}}, special));
        }
        rows.add(placementCase("jungle_vertical_clip", "jungle_pyramid", 0, 0, 0, "flat", 0, 0,
            new int[][]{{0,0,65,319}}, ""));
        return rows;
    }

    static List<Object> scatteredReferences() throws Exception {
        List<Object> rows = new ArrayList<>();
        for (String kind : KINDS) for (int shift : new int[]{0, 13}) {
            configureFlat(); freshWorld("flat", 42L);
            Object cp = make("world.level.ChunkPos", -1, -1), value = structure(kind);
            Object start = nativeStart("minecraft:" + kind, 42L, -1, -1);
            Object piece = ((List<?>)call(start, "getPieces")).get(0);
            if (shift != 0) call(piece, "move", shift, 0, shift);
            call(markChunk(-1, -1), "setStartForStructure", value, start);
            structureManager = make("world.level.StructureManager", referenceWorld(), make("world.level.levelgen.WorldOptions", 42L, true, false), null);
            List<Object> targets = new ArrayList<>();
            for (int x = -3; x <= 2; x++) for (int z = -3; z <= 2; z++) {
                Object target = markChunk(x, z);
                call(generator, "createReferences", referenceWorld(), structureManager, target);
                Object found = ((Map<?,?>)call(target, "getAllReferences")).get(value);
                List<Object> references = new ArrayList<>();
                if (found != null) for (Object packed : (Iterable<?>)found) {
                    Object source = call(type("world.level.ChunkPos"), "unpack", packed);
                    references.add(List.of(call(source, "x"), call(source, "z")));
                }
                targets.add(Map.of("chunk", List.of(x, z), "sources", references));
            }
            rows.add(Map.of("kind", kind, "seed", 42L, "shift", shift,
                "start", Map.of("nbt", nbt64(call(start, "createTag", serialContext, cp))),
                "reference_bounds", bounds(call(start, "getBoundingBox")), "targets", targets));
        }
        return rows;
    }

    static Map<String,Object> scatteredContainers() throws Exception {
        Map<String,Object> defaults = new TreeMap<>(); List<Object> cases = new ArrayList<>();
        for (String name : List.of("CHEST", "DISPENSER")) {
            Object block = field("world.level.block.Blocks", name), state = call(block, "defaultBlockState");
            Object entity = call(block, "newBlockEntity", make("core.BlockPos", 0, 0, 0), state);
            Object type = call(entity, "getType"), types = field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE");
            defaults.put(call(blockRegistry, "getKey", block).toString(), Map.of("id", call(types, "getKey", type).toString(), "type_id", call(types, "getId", type),
                "full", Map.of("nbt", nbt64(call(entity,"saveWithFullMetadata", registries))), "update", Map.of("nbt", nbt64(call(entity,"getUpdateTag",registries)))));
            for (long seed : new long[]{0, 1, -1, Long.MIN_VALUE, Long.MAX_VALUE}) {
                Object p = make("core.BlockPos", -17, 61, 32);
                entity = call(block, "newBlockEntity", p, state);
                Object table = key("LOOT_TABLE", "minecraft:chests/" + (name.equals("CHEST") ? "buried_treasure" : "jungle_temple_dispenser"));
                call(entity,"setLootTable",table,seed);
                cases.add(Map.of("state",stateId(state),"pos",xyz(p),"table",call(table,"identifier").toString(),"seed",seed,
                    "full",Map.of("nbt",nbt64(call(entity,"saveWithFullMetadata",registries))),"update",Map.of("nbt",nbt64(call(entity,"getUpdateTag",registries)))));
            }
        }
        return Map.of("defaults",defaults,"cases",cases);
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat(); prepareDisabledEntityLevel();
        Map<String,Object> result = new LinkedHashMap<>(); String section = args.length > 1 ? args[1] : "all";
        result.put("scope", "Native 26.1 scattered admission and StructureStart.placeInChunk on controlled WorldGenLevel. Exact ordered writes, loot mutations, requested fluid ticks, ProtoChunk marks, retained piece tags and RNG. Swamp mob factory requests are observed using native EntityType.create with an empty enabled-feature set; entities/finalizeSpawn are explicitly not instantiated or verified.");
        result.put("catalog", catalog(args[0]));
        result.put("block_properties", blockProperties());
        if (section.equals("containers")) result.put("containers", scatteredContainers());
        if (section.equals("jungle")) {
            result.put("admission", jungleAdmissions()); result.put("placements", junglePlacements());
            result.put("references", scatteredReferences());
        }
        if (section.equals("all") || section.equals("placement")) result.put("chest_orientations", chestOrientations());
        if (section.equals("all") || section.equals("admission")) result.put("admission", scatteredAdmissions());
        if (section.equals("all") || section.equals("placement")) result.put("placements", scatteredPlacements());
        output("SCATTEREDSTRUCTUREREFERENCE", result); call(resources, "close");
    }
}
