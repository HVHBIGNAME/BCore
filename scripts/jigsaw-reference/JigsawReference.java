import java.lang.reflect.*;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.Predicate;

/** Fresh controlled-world calls to real JigsawPlacement, SinglePoolElement and StructureTemplate. */
public class JigsawReference extends JigsawSupport {
    static Object generator, heightAccessor, serialContext;
    static Map<Pos, Object> placed;
    static Map<Pos, Object> blockEntities;
    static List<List<Integer>> writes;
    static List<Object> ticks;
    static Object fill;
    static String terrain;
    static Map<Pos, Object> markChunks;
    static Object markFactory;

    static void includeBuiltInRegistries() throws Exception {
        Object builtin = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", field("core.registries.BuiltInRegistries", "REGISTRY"));
        Map<Object, Object> merged = new HashMap<>();
        for (Object access : List.of(builtin, registries)) {
            for (Object entry : ((java.util.stream.Stream<?>) call(access, "registries")).toList()) merged.put(call(entry, "key"), call(entry, "value"));
        }
        registries = call(make("core.RegistryAccess$ImmutableRegistryAccess", merged), "freeze");
        // 26.1 binds item components after loading datapack registries, in
        // ReloadableServerResources.updateComponentsAndStaticRegistryTags.
        // Bootstrap and an ITEM lookup alone leave inventory codecs unbound.
        for (Object pending : (List<?>) call(field("core.registries.BuiltInRegistries", "DATA_COMPONENT_INITIALIZERS"), "build", registries)) {
            call(pending, "apply");
        }
        for (Object item : (Iterable<?>) registry("ITEM")) call(item, "components");
    }

    static void configureFlat() throws Exception {
        Object biome = call(registry("BIOME"), "getOrThrow", key("BIOME", "minecraft:plains"));
        Object settings = make("world.level.levelgen.flat.FlatLevelGeneratorSettings", Optional.empty(), biome, List.of());
        ((List<Object>) call(settings, "getLayersInfo")).add(make("world.level.levelgen.flat.FlatLayerInfo", 129, field("world.level.block.Blocks", "STONE")));
        call(settings, "updateLayers");
        generator = make("world.level.levelgen.FlatLevelSource", settings);
        heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        serialContext = make("world.level.levelgen.structure.pieces.StructurePieceSerializationContext", resources, registries, templateManager);
    }

    static Map<String, Object> assembly(String structure, long seed, int x, int z) throws Exception {
        Object context = make("world.level.levelgen.structure.Structure$GenerationContext", registries, generator,
            call(generator, "getBiomeSource"), null, templateManager, seed, make("world.level.ChunkPos", x, z),
            heightAccessor, (Predicate<Object>) holder -> true);
        Object value = call(call(registry("STRUCTURE"), "getOrThrow", key("STRUCTURE", structure)), "value");
        Optional<?> optional = (Optional<?>) call(value, "findGenerationPoint", context);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("structure", structure); result.put("seed", seed); result.put("chunk", List.of(x, z));
        result.put("first_free_height", 65);
        if (optional.isEmpty()) {
            result.put("pieces", List.of());
        } else {
            Object stub = optional.orElseThrow();
            result.put("generation_point", xyz(call(stub, "position")));
            List<Object> pieces = new ArrayList<>();
            for (Object piece : (List<?>) call(call(call(stub, "getPiecesBuilder"), "build"), "pieces")) {
                Map<String, Object> entry = new LinkedHashMap<>();
                entry.put("pos", xyz(call(piece, "getPosition")));
                entry.put("bounds", bounds(call(piece, "getBoundingBox")));
                entry.put("rotation", call(piece, "getRotation").toString());
                entry.put("ground_level_delta", call(piece, "getGroundLevelDelta"));
                entry.put("nbt", nbt64(call(piece, "createTag", serialContext)));
                pieces.add(entry);
            }
            result.put("pieces", pieces);
        }
        result.put("next_i64", call(call(context, "random"), "nextLong"));
        return result;
    }

    static Object getBlock(Object pos) throws Exception {
        Pos p = Pos.from(pos);
        if (p.y() < -64 || p.y() > 319) return state("VOID_AIR");
        if (placed.containsKey(p)) return placed.get(p);
        if (terrain.equals("protected") && Math.floorMod(p.x() * 3 + p.y() + p.z() * 5, 17) == 0) return state("BEDROCK");
        if (terrain.equals("slope")) return p.y() < height(p.x(), p.z()) ? state("STONE") : state("AIR");
        if (terrain.equals("city_cave")) return p.y() < -50 || p.y() >= -20 ? state("STONE") : state("AIR");
        return fill;
    }

    static int height(int x, int z) {
        return terrain.equals("slope") ? 64 + Math.floorMod(x + 2 * z, 5) : 65;
    }

    static Object markChunk(int x, int z) throws Exception {
        Pos key = new Pos(x, 0, z);
        if (markChunks.containsKey(key)) return markChunks.get(key);
        if (markFactory == null) {
            Object blockStrategy = call(type("world.level.chunk.Strategy"), "createForBlockStates", field("world.level.block.Block", "BLOCK_STATE_REGISTRY"));
            Object biome = call(registry("BIOME"), "getOrThrow", key("BIOME", "minecraft:plains"));
            Object biomeIds = make("core.IdMapper"); call(biomeIds, "add", biome);
            Object biomeStrategy = call(type("world.level.chunk.Strategy"), "createForBiomes", biomeIds);
            markFactory = make("world.level.chunk.PalettedContainerFactory", blockStrategy, state("AIR"), null, biomeStrategy, biome, null);
        }
        Object chunk = make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", x, z), field("world.level.chunk.UpgradeData", "EMPTY"),
            null, make("world.ticks.ProtoChunkTicks"), make("world.ticks.ProtoChunkTicks"), heightAccessor, markFactory, null);
        markChunks.put(key, chunk);
        return chunk;
    }

    static Object world() throws Exception {
        return Proxy.newProxyInstance(JigsawReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            try {
                return switch (m.getName()) {
                    case "getMinY" -> -64;
                    case "getMaxY" -> 319;
                    case "isOutsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : Pos.from(a[0]).y(); yield y < -64 || y > 319; }
                    case "ensureCanWrite" -> { Pos pos = Pos.from(a[0]); yield pos.y() >= -64 && pos.y() <= 319; }
                    case "getHeight" -> a == null || a.length == 0 ? 384 : height((int) a[1], (int) a[2]);
                    case "getBlockState" -> getBlock(a[0]);
                    case "getChunk" -> {
                        if (a.length == 1) { Pos pos = Pos.from(a[0]); yield markChunk(pos.x() >> 4, pos.z() >> 4); }
                        yield markChunk((int) a[0], (int) a[1]);
                    }
                    case "getFluidState" -> call(getBlock(a[0]), "getFluidState");
                    case "isClientSide" -> false;
                    case "registryAccess" -> registries;
                    case "holderLookup" -> a[0].equals(field("core.registries.Registries", "BLOCK")) ? blockRegistry : call(registries, "lookupOrThrow", a[0]);
                    case "getBlockEntity" -> blockEntities.get(Pos.from(a[0]));
                    case "setBlock" -> {
                        Pos pos = Pos.from(a[0]);
                        if (pos.y() < -64 || pos.y() > 319) yield false;
                        Object old = getBlock(a[0]);
                        placed.put(pos, a[1]);
                        writes.add(List.of(pos.x(), pos.y(), pos.z(), stateId(a[1]), (int) a[2]));
                        Object block = call(a[1], "getBlock");
                        if (!(boolean) call(a[1], "hasBlockEntity")) blockEntities.remove(pos);
                        else if (!blockEntities.containsKey(pos) || call(old, "getBlock") != block) {
                            Object entity = call(block, "newBlockEntity", a[0], a[1]);
                            if (entity != null) blockEntities.put(pos, entity);
                        }
                        yield true;
                    }
                    case "scheduleTick" -> {
                        ticks.add(List.of(xyz(a[0]), call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]), a[2]));
                        yield null;
                    }
                    // These four callbacks are also no-ops in WorldGenRegion.
                    case "playSound", "addParticle", "levelEvent", "gameEvent" -> null;
                    case "getLevel" -> null;
                    case "addFreshEntityWithPassengers" -> throw new UnsupportedOperationException("entity-bearing template requires the entity fixture");
                    case "toString" -> "JigsawReference fresh map world";
                    default -> throw new UnsupportedOperationException(m.toString());
                };
            } catch (InvocationTargetException e) { throw e.getCause(); }
        });
    }

    static String digest(List<List<Integer>> rows) throws Exception {
        MessageDigest md5 = MessageDigest.getInstance("MD5");
        ByteBuffer buffer = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN);
        for (List<Integer> row : rows) for (int v : row) {
            buffer.clear(); buffer.putInt(v); md5.update(buffer.array());
        }
        return HexFormat.of().formatHex(md5.digest());
    }

    static Map<String, Object> placement(String name, String processor, String projection, String rotation, String mirror,
        long seed, String mode, List<Integer> clipping) throws Exception {
        terrain = mode;
        fill = state(mode.equals("water") ? "WATER" : mode.equals("air") ? "AIR" : "STONE");
        placed = new HashMap<>(); blockEntities = new HashMap<>(); writes = new ArrayList<>(); ticks = new ArrayList<>(); markChunks = new HashMap<>();
        Object origin = make("core.BlockPos", -9, mode.equals("slope") ? 64 : -50, 7);
        Object reference = make("core.BlockPos", -2, -48, 12);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        Object t = template(name);
        Object settings = make("world.level.levelgen.structure.templatesystem.StructurePlaceSettings");
        call(settings, "setRotation", field("world.level.block.Rotation", rotation));
        call(settings, "setMirror", field("world.level.block.Mirror", mirror));
        call(settings, "setKnownShape", true);
        call(settings, "setIgnoreEntities", true);
        call(settings, "addProcessor", field("world.level.levelgen.structure.templatesystem.BlockIgnoreProcessor", "STRUCTURE_BLOCK"));
        call(settings, "addProcessor", field("world.level.levelgen.structure.templatesystem.JigsawReplacementProcessor", "INSTANCE"));
        if (!processor.isEmpty()) {
            Object processors = call(call(registry("PROCESSOR_LIST"), "getOrThrow", key("PROCESSOR_LIST", processor)), "value");
            for (Object item : (List<?>) call(processors, "list")) call(settings, "addProcessor", item);
        }
        if (projection.equals("terrain_matching")) call(settings, "addProcessor",
            make("world.level.levelgen.structure.templatesystem.GravityProcessor", field("world.level.levelgen.Heightmap$Types", "WORLD_SURFACE_WG"), -1));
        if (!clipping.isEmpty()) call(settings, "setBoundingBox", make("world.level.levelgen.structure.BoundingBox", clipping.toArray()));
        boolean placedResult = (boolean) call(t, "placeInWorld", world(), origin, reference, settings, random, 18);
        List<Pos> positions = new ArrayList<>(placed.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        List<List<Integer>> states = new ArrayList<>();
        for (Pos p : positions) states.add(List.of(p.x(), p.y(), p.z(), stateId(placed.get(p))));
        List<Object> entities = new ArrayList<>();
        for (Pos p : positions) if (blockEntities.containsKey(p)) {
            entities.add(Map.of("pos", List.of(p.x(), p.y(), p.z()), "nbt", nbt64(call(blockEntities.get(p), "saveWithFullMetadata", registries))));
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("template", name); result.put("processors", processor); result.put("projection", projection);
        result.put("rotation", rotation); result.put("mirror", mirror); result.put("seed", seed); result.put("terrain", mode);
        result.put("origin", xyz(origin)); result.put("reference", xyz(reference)); result.put("clip", clipping);
        result.put("placed", placedResult); result.put("write_count", writes.size()); result.put("writes_md5", digest(writes));
        result.put("state_count", states.size()); result.put("states_md5", digest(states)); result.put("block_entities", entities);
        result.put("ticks", ticks); result.put("next_i64", call(random, "nextLong"));
        return result;
    }

    static Object nativeStart(String name, long seed, int x, int z) throws Exception {
        Object context = make("world.level.levelgen.structure.Structure$GenerationContext", registries, generator,
            call(generator, "getBiomeSource"), null, templateManager, seed, make("world.level.ChunkPos", x, z), heightAccessor, (Predicate<Object>) h -> true);
        Object structure = call(call(registry("STRUCTURE"), "getOrThrow", key("STRUCTURE", name)), "value");
        Object stub = ((Optional<?>) call(structure, "findGenerationPoint", context)).orElseThrow();
        Object pieces = call(call(stub, "getPiecesBuilder"), "build");
        return make("world.level.levelgen.structure.StructureStart", structure, make("world.level.ChunkPos", x, z), 0, pieces);
    }

    static Map<String, Object> fullCity(long seed, int cx, int cz, boolean entire, String environment) throws Exception {
        Object start = nativeStart("minecraft:ancient_city", seed, 0, 0);
        terrain = environment; fill = state("STONE");
        placed = new HashMap<>(); blockEntities = new HashMap<>(); writes = new ArrayList<>(); ticks = new ArrayList<>(); markChunks = new HashMap<>();
        Object clip = entire ? call(start, "getBoundingBox") : make("world.level.levelgen.structure.BoundingBox", cx * 16, -64, cz * 16, cx * 16 + 15, 319, cz * 16 + 15);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", 918273L));
        call(start, "placeInChunk", world(), null, generator, random, clip, make("world.level.ChunkPos", cx, cz));
        List<Pos> positions = new ArrayList<>(placed.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        List<List<Integer>> states = new ArrayList<>();
        List<Object> entities = new ArrayList<>();
        for (Pos p : positions) {
            states.add(List.of(p.x(), p.y(), p.z(), stateId(placed.get(p))));
            if (blockEntities.containsKey(p)) entities.add(Map.of("pos", List.of(p.x(), p.y(), p.z()), "nbt", nbt64(call(blockEntities.get(p), "saveWithFullMetadata", registries))));
        }
        Map<Pos, Integer> marks = new TreeMap<>(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        Method unpack = type("world.level.chunk.ProtoChunk").getMethod("unpackOffsetCoordinates", short.class, int.class, type("world.level.ChunkPos"));
        for (Object chunk : markChunks.values()) {
            Object[] sections = (Object[]) call(chunk, "getPostProcessing");
            for (int s = 0; s < sections.length; s++) if (sections[s] != null) for (Object packed : (Iterable<?>) sections[s]) {
                Pos p = Pos.from(unpack.invoke(null, packed, s - 4, call(chunk, "getPos")));
                marks.merge(p, 1, Integer::sum);
            }
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("seed", seed); result.put("chunk", List.of(cx, cz)); result.put("entire", entire);
        result.put("terrain", environment);
        result.put("placement_seed", 918273L); result.put("clip", bounds(clip));
        result.put("write_count", writes.size()); result.put("writes_md5", digest(writes));
        result.put("state_count", states.size()); result.put("states_md5", digest(states));
        result.put("block_entities", entities); result.put("ticks", ticks);
        result.put("marks", marks.entrySet().stream().map(e -> List.of(e.getKey().x(), e.getKey().y(), e.getKey().z(), e.getValue())).toList());
        result.put("next_i64", call(random, "nextLong"));
        return result;
    }

    static List<Object> transforms() throws Exception {
        List<Object> result = new ArrayList<>();
        for (Object r : type("world.level.block.Rotation").getEnumConstants()) for (Object m : type("world.level.block.Mirror").getEnumConstants()) {
            Object p = make("core.BlockPos", -5, 4, 7), pivot = make("core.BlockPos", 2, 9, -3);
            Object v = type("world.phys.Vec3").getConstructor(double.class, double.class, double.class).newInstance(-4.75, 4.125, 7.75);
            Object transformed = call(type("world.level.levelgen.structure.templatesystem.StructureTemplate"), "transform", v, m, r, pivot);
            result.add(Map.of("rotation", r.toString(), "mirror", m.toString(),
                "block", xyz(call(type("world.level.levelgen.structure.templatesystem.StructureTemplate"), "transform", p, m, r, pivot)),
                "entity", List.of(transformed.getClass().getField("x").get(transformed), transformed.getClass().getField("y").get(transformed), transformed.getClass().getField("z").get(transformed))));
        }
        return result;
    }

    static List<Object> cities() throws Exception {
        List<Object> result = new ArrayList<>();
        result.add(fullCity(0, 0, 0, true, "stone"));
        result.add(fullCity(1, 0, 0, true, "stone"));
        result.add(fullCity(42, 0, 0, false, "stone"));
        result.add(fullCity(42, -2, 1, false, "stone"));
        result.add(fullCity(0, 0, 0, true, "city_cave"));
        result.add(fullCity(42, 0, 0, true, "city_cave"));
        return result;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(); includeBuiltInRegistries(); configureFlat();
        if (args.length > 1 && args[1].equals("cities")) {
            output("JIGSAWREFERENCE", Map.of("cities", cities()));
            call(resources, "close");
            return;
        }
        List<Object> assemblies = new ArrayList<>(), placements = new ArrayList<>();
        for (String name : List.of("ancient_city", "village_plains", "village_desert", "village_savanna", "village_snowy", "village_taiga")) {
            for (long seed : new long[]{0, 1, 42, -17}) assemblies.add(assembly("minecraft:" + name, seed, seed == 42 ? -3 : 0, seed == -17 ? 5 : 0));
        }
        for (String r : List.of("NONE", "CLOCKWISE_90", "CLOCKWISE_180", "COUNTERCLOCKWISE_90")) {
            for (String m : List.of("NONE", "LEFT_RIGHT", "FRONT_BACK")) {
                placements.add(placement("minecraft:ancient_city/city_center/city_center_1", "minecraft:ancient_city_start_degradation", "rigid", r, m, 123, "protected", List.of()));
                placements.add(placement("minecraft:ancient_city/structures/ice_box_1", "", "rigid", r, m, 123, "water", List.of()));
            }
        }
        for (String name : List.of("minecraft:ancient_city/structures/barracks", "minecraft:ancient_city/walls/intact_horizontal_wall_stairs_4")) {
            placements.add(placement(name, "minecraft:ancient_city_generic_degradation", "rigid", "COUNTERCLOCKWISE_90", "NONE", 42, "stone", List.of(-16, -64, 0, -1, 319, 15)));
        }
        placements.add(placement("minecraft:village/plains/streets/straight_01", "minecraft:street_plains", "terrain_matching", "CLOCKWISE_90", "NONE", 42, "slope", List.of()));
        placements.add(placement("minecraft:village/plains/houses/plains_small_house_1", "minecraft:zombie_plains", "rigid", "COUNTERCLOCKWISE_90", "FRONT_BACK", -1, "air", List.of()));
        output("JIGSAWREFERENCE", Map.of("assembly", assemblies, "placement", placements, "transforms", transforms(), "cities", cities(),
            "scope", "Native assembly on a fresh flat generator; actual template+processor placement and StructureStart.placeInChunk including sculk in fresh map-backed WorldGenLevel instances. Known shape; entity spawning/finalization is outside this fixture (no live ServerLevel). Native ProtoChunks retain explicit sculk marks. No server scheduler or gameplay ticks."));
        call(resources, "close");
    }
}
