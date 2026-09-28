import java.lang.reflect.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;

/** Native OreFeature using real ProtoChunk sections in an in-memory region. */
public class OreReference extends TreeReference {
    static Object height, airFactory, baseFactory, base;
    static Object stone, deepslate, water, bedrock, caveAir;
    static Method sectionSet, chunkSection, sectionGet;
    static Map<Pos, Object> chunks;
    static Set<Pos> visited;
    static String terrain;
    static int surface;

    static Object factory(Object defaultState) throws Exception {
        Object blockStrategy = call(type("world.level.chunk.Strategy"), "createForBlockStates", field("world.level.block.Block", "BLOCK_STATE_REGISTRY"));
        // OreFeature never queries biomes. The native containers still require a
        // palette entry; this opaque holder is used only by that unused palette.
        Object biome = call(type("core.Holder"), "direct", "unused by ore probe");
        Object biomeIds = make("core.IdMapper");
        call(biomeIds, "add", biome);
        Object biomeStrategy = call(type("world.level.chunk.Strategy"), "createForBiomes", biomeIds);
        return make("world.level.chunk.PalettedContainerFactory", blockStrategy, defaultState, null, biomeStrategy, biome, null);
    }

    static Object chunk(int cx, int cz) throws Exception {
        Pos key = new Pos(cx, 0, cz);
        Object result = chunks.get(key);
        if (result != null) return result;
        Object sections = Array.newInstance(type("world.level.chunk.LevelChunkSection"), 24);
        for (int sy = 0; sy < 24; sy++) {
            int bottom = -64 + sy * 16;
            Object factory = bottom >= surface || terrain.equals("air") ? airFactory : baseFactory;
            Object section = make("world.level.chunk.LevelChunkSection", factory);
            if (bottom < surface && cx == 0 && Set.of("cave", "water", "bedrock", "cave_air").contains(terrain)) {
                Object plane = switch (terrain) {
                    case "water" -> water;
                    case "bedrock" -> bedrock;
                    case "cave_air" -> caveAir;
                    default -> air;
                };
                for (int y = 0; y < 16; y++) for (int z = 0; z < 16; z++) sectionSet.invoke(section, 8, y, z, plane, false);
            }
            Array.set(sections, sy, section);
        }
        result = make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", cx, cz),
            field("world.level.chunk.UpgradeData", "EMPTY"), sections,
            make("world.ticks.ProtoChunkTicks"), make("world.ticks.ProtoChunkTicks"), height, airFactory, null);
        chunks.put(key, result);
        return result;
    }

    static Object read(Pos pos) throws Exception {
        if (pos.y() < -64 || pos.y() >= 320) return air;
        Object section = chunkSection.invoke(chunk(pos.x() >> 4, pos.z() >> 4), (pos.y() + 64) >> 4);
        return sectionGet.invoke(section, pos.x() & 15, pos.y() & 15, pos.z() & 15);
    }

    static Object initial(Pos pos) {
        if (pos.y() < -64 || pos.y() >= surface || terrain.equals("air")) return air;
        if (pos.x() == 8) {
            switch (terrain) {
                case "cave": return air;
                case "cave_air": return caveAir;
                case "water": return water;
                case "bedrock": return bedrock;
            }
        }
        return base;
    }

    static Object world() throws Exception {
        return Proxy.newProxyInstance(OreReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            return switch (m.getName()) {
                case "getHeight" -> {
                    if (a == null || a.length == 0) yield 384;
                    boolean empty = terrain.equals("air") || ((int) a[1] == 8 && Set.of("cave", "cave_air", "water").contains(terrain));
                    yield empty ? -64 : surface;
                }
                case "getMinY" -> -64;
                case "getMaxY" -> 319;
                case "getSectionsCount" -> 24;
                case "getSectionIndex" -> (((int) a[0]) + 64) >> 4;
                case "isOutsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : Pos.from(a[0]).y(); yield y < -64 || y >= 320; }
                case "ensureCanWrite" -> { visited.add(Pos.from(a[0])); yield true; }
                case "getChunk" -> chunk((int) a[0], (int) a[1]);
                case "getBlockState" -> read(Pos.from(a[0]));
                default -> throw new UnsupportedOperationException(m.toString());
            };
        });
    }

    static Object configuration(String name) throws Exception {
        String path = "/data/minecraft/worldgen/configured_feature/" + name + ".json";
        try (var stream = OreReference.class.getResourceAsStream(path)) {
            if (stream == null) throw new IllegalArgumentException("missing " + path);
            Object doc = call(Class.forName("com.google.gson.JsonParser"), "parseString", new String(stream.readAllBytes(), StandardCharsets.UTF_8));
            Object value = call(call(doc, "getAsJsonObject"), "get", "config");
            Object ops = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
            return call(call(field("world.level.levelgen.feature.configurations.OreConfiguration", "CODEC"), "parse", ops, value), "getOrThrow");
        }
    }

    static Map<String, Object> probe(Object feature, Object config, String name, long seed, String scenario, Pos origin, int attempts) throws Exception {
        terrain = scenario;
        base = switch (scenario) {
            case "deepslate" -> deepslate;
            case "deepslate_x" -> call(type("world.level.block.Block"), "stateById", 27923);
            case "deepslate_z" -> call(type("world.level.block.Block"), "stateById", 27925);
            case "tuff" -> state("TUFF");
            default -> stone;
        };
        baseFactory = factory(base);
        surface = scenario.equals("ceiling") ? 320 : 64;
        chunks = new HashMap<>();
        visited = new HashSet<>();
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        Object world = world();
        List<Boolean> placed = new ArrayList<>();
        for (int i = 0; i < attempts; i++) {
            Object context = make("world.level.levelgen.feature.FeaturePlaceContext", Optional.empty(), world, null, random,
                make("core.BlockPos", origin.x(), origin.y(), origin.z()), config);
            placed.add((boolean) call(feature, "place", context));
        }
        MessageDigest digest = MessageDigest.getInstance("MD5");
        int count = 0;
        List<Pos> sorted = new ArrayList<>(visited);
        sorted.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        for (Pos pos : sorted) {
            Object state = read(pos);
            if (state == initial(pos)) continue;
            int id = (int) call(type("world.level.block.Block"), "getId", state);
            digest.update(ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN).putInt(pos.x()).putInt(pos.y()).putInt(pos.z()).putInt(id).array());
            count++;
        }
        return Map.of("kind", name, "seed", seed, "terrain", scenario, "origin", List.of(origin.x(), origin.y(), origin.z()),
            "attempts", attempts, "placed", placed, "changed_blocks", count, "writes_md5", HexFormat.of().formatHex(digest.digest()), "next_i64", call(random, "nextLong"));
    }

    static Map<String, List<Integer>> bootstrapOre() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Map<String, List<Integer>> tagStates = new TreeMap<>();
        Map<Object, Set<Object>> bindings = new HashMap<>();
        for (String tag : List.of("base_stone_overworld", "stone_ore_replaceables", "deepslate_ore_replaceables")) {
            Object key = call(type("tags.TagKey"), "create", field("core.registries.Registries", "BLOCK"), call(type("resources.Identifier"), "withDefaultNamespace", tag));
            List<Integer> ids = new ArrayList<>();
            for (Object block : tagBlocks(tag)) {
                bindings.computeIfAbsent(block, b -> new HashSet<>()).add(key);
                for (Object state : (Iterable<?>) call(call(block, "getStateDefinition"), "getPossibleStates")) ids.add((int) call(type("world.level.block.Block"), "getId", state));
            }
            Collections.sort(ids);
            tagStates.put(tag, ids);
        }
        for (var entry : bindings.entrySet()) call(call(entry.getKey(), "builtInRegistryHolder"), "bindTags", entry.getValue());
        air = state("AIR"); stone = state("STONE"); deepslate = state("DEEPSLATE");
        water = state("WATER"); bedrock = state("BEDROCK"); caveAir = state("CAVE_AIR");
        height = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        airFactory = factory(air);
        sectionSet = type("world.level.chunk.LevelChunkSection").getMethod("setBlockState", int.class, int.class, int.class, type("world.level.block.state.BlockState"), boolean.class);
        sectionGet = type("world.level.chunk.LevelChunkSection").getMethod("getBlockState", int.class, int.class, int.class);
        chunkSection = type("world.level.chunk.ChunkAccess").getMethod("getSection", int.class);
        return tagStates;
    }

    public static void main(String[] args) throws Exception {
        Map<String, List<Integer>> tagStates = bootstrapOre();
        Object feature = make("world.level.levelgen.feature.OreFeature", field("world.level.levelgen.feature.configurations.OreConfiguration", "CODEC"));
        List<Object> samples = new ArrayList<>();
        Map<String, Object> configs = new TreeMap<>();
        for (String name : List.of("ore_coal", "ore_coal_buried", "ore_diamond_buried", "ore_iron", "ore_copper_large", "ore_granite", "ore_dirt")) {
            Object config = configuration(name);
            configs.put(name, Map.of("size", config.getClass().getField("size").get(config),
                "discard_chance_on_air_exposure", config.getClass().getField("discardChanceOnAirExposure").get(config)));
            for (long seed : new long[]{0, 1, 17, 42}) {
                for (String scenario : List.of("stone", "deepslate", "deepslate_x", "deepslate_z", "tuff", "cave", "cave_air", "water", "bedrock", "air")) {
                    samples.add(probe(feature, config, name, seed, scenario, new Pos(8, 32, 8), 1));
                }
                samples.add(probe(feature, config, name, seed, "stone", new Pos(-1, -63, 16), 1));
                samples.add(probe(feature, config, name, seed, "stone", new Pos(8, 90, 8), 1));
                samples.add(probe(feature, config, name, seed, "ceiling", new Pos(8, 319, 8), 1));
                samples.add(probe(feature, config, name, seed, "cave", new Pos(8, 32, 8), 3));
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        int caveAirId = (int) call(type("world.level.block.Block"), "getId", caveAir);
        System.out.println("ORE_REFERENCE=" + call(gson, "toJson", Map.of("tags", tagStates, "configurations", configs, "samples", samples, "cave_air_state", caveAirId)));
    }
}
