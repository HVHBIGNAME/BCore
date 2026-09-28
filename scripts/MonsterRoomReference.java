import java.lang.reflect.*;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.Predicate;

/** Execute native MonsterRoomFeature and save real chest/spawner block entities. */
public class MonsterRoomReference extends TreeReference {
    static Object stone, bedrock, chest, spawner, lookup;
    static Map<Pos, Object> entities;
    static Pos origin;
    static String scenario;

    static Object initial(Pos p) {
        if (p.y() < -64 || p.y() >= 320) return air;
        int x = p.x() - origin.x(), y = p.y() - origin.y(), z = p.z() - origin.z();
        if (scenario.equals("floor_hole") && x == 0 && z == 0 && y == -1) return air;
        if (scenario.equals("roof_hole") && x == 0 && z == 0 && y == 4) return air;
        if (scenario.equals("unsupported_floor") && y == -2) return air;
        if (scenario.equals("protected") && x == 0 && z == 0 && y >= -1 && y <= 1) return bedrock;
        if (scenario.equals("existing_chest") && x == 1 && y == 0 && z == 1) return chest;
        if (!scenario.equals("closed") && (y == 0 || y == 1) && Math.abs(z) <= (scenario.equals("wide") ? 2 : 0)) return air;
        return stone;
    }

    static Object read(Pos p) { return blocks.getOrDefault(p, initial(p)); }

    static Object entity(Pos p) throws Exception {
        Object state = read(p);
        if (!(boolean) call(state, "hasBlockEntity")) return null;
        if (!entities.containsKey(p)) entities.put(p, call(call(state, "getBlock"), "newBlockEntity", make("core.BlockPos", p.x(), p.y(), p.z()), state));
        return entities.get(p);
    }

    static Object world() throws Exception {
        return Proxy.newProxyInstance(MonsterRoomReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> switch (m.getName()) {
            case "getMinY" -> -64;
            case "getMaxY" -> 319;
            case "getHeight" -> 384;
            case "getBlockState" -> read(Pos.from(a[0]));
            case "isEmptyBlock" -> call(read(Pos.from(a[0])), "isAir");
            case "isStateAtPosition" -> Predicate.class.getMethod("test", Object.class).invoke(a[1], read(Pos.from(a[0])));
            case "getBlockEntity" -> entity(Pos.from(a[0]));
            case "setBlock" -> {
                Pos pos = Pos.from(a[0]);
                if (pos.y() < -64 || pos.y() >= 320) yield false;
                if (call(read(pos), "getBlock") != call(a[1], "getBlock")) entities.remove(pos);
                blocks.put(pos, a[1]);
                entity(pos);
                yield true;
            }
            default -> throw new UnsupportedOperationException(m.toString());
        });
    }

    static Map<String, Object> probe(Object feature, long seed, String terrain, Pos pos) throws Exception {
        origin = pos; scenario = terrain; blocks = new HashMap<>(); entities = new HashMap<>();
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        Object context = make("world.level.levelgen.feature.FeaturePlaceContext", Optional.empty(), world(), null, random,
            make("core.BlockPos", pos.x(), pos.y(), pos.z()), field("world.level.levelgen.feature.configurations.NoneFeatureConfiguration", "INSTANCE"));
        boolean placed = (boolean) call(feature, "place", context);
        MessageDigest digest = MessageDigest.getInstance("MD5");
        int count = 0;
        List<Pos> sorted = new ArrayList<>(blocks.keySet());
        sorted.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        for (Pos p : sorted) {
            Object state = read(p);
            if (state == initial(p)) continue;
            int id = (int) call(type("world.level.block.Block"), "getId", state);
            digest.update(ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN).putInt(p.x()).putInt(p.y()).putInt(p.z()).putInt(id).array());
            count++;
        }
        List<Object> tags = new ArrayList<>();
        List<Pos> positions = new ArrayList<>(entities.keySet());
        positions.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
        Object ops = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        for (Pos p : positions) {
            Object be = entities.get(p);
            Object nbt = call(be, "saveWithFullMetadata", lookup);
            Object update = call(be, "getUpdateTag", lookup);
            tags.add(Map.of("pos", List.of(p.x(), p.y(), p.z()), "nbt", call(field("nbt.NbtOps", "INSTANCE"), "convertTo", ops, nbt),
                "update", call(field("nbt.NbtOps", "INSTANCE"), "convertTo", ops, update), "snbt", nbt.toString(),
                "type", call(field("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE"), "getId", call(be, "getType"))));
        }
        return Map.of("seed", seed, "terrain", terrain, "origin", List.of(pos.x(), pos.y(), pos.z()), "placed", placed,
            "changed_blocks", count, "writes_md5", HexFormat.of().formatHex(digest.digest()), "next_i64", call(random, "nextLong"), "block_entities", tags);
    }

    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion"); call(type("server.Bootstrap"), "bootStrap");
        lookup = call(type("data.registries.VanillaRegistries"), "createLookup");
        Object protectedTag = field("tags.BlockTags", "FEATURES_CANNOT_REPLACE");
        for (Object block : tagBlocks("features_cannot_replace")) call(call(block, "builtInRegistryHolder"), "bindTags", Set.of(protectedTag));
        air = state("AIR"); stone = state("STONE"); bedrock = state("BEDROCK"); chest = state("CHEST"); spawner = state("SPAWNER");
        List<Object> ranges = new ArrayList<>(); int start = 0, last = -1, id = 0;
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            int flags = ((boolean) call(state, "isAir") ? 1 : 0) | ((boolean) call(state, "isSolid") ? 2 : 0)
                | ((boolean) call(state, "isSolidRender") ? 4 : 0) | ((boolean) state.getClass().getMethod("is", type("tags.TagKey")).invoke(state, protectedTag) ? 8 : 0)
                | (call(state, "getBlock") == field("world.level.block.Blocks", "CHEST") ? 16 : 0)
                | (call(state, "getBlock") == field("world.level.block.Blocks", "SPAWNER") ? 32 : 0);
            if (flags != last) { if (id > 0) ranges.add(List.of(start, id, last)); start = id; last = flags; }
            id++;
        }
        ranges.add(List.of(start, id, last));
        Map<String, Integer> states = new TreeMap<>();
        for (String name : List.of("CAVE_AIR", "COBBLESTONE", "MOSSY_COBBLESTONE", "SPAWNER", "CHEST", "BEDROCK")) states.put(name, (int) call(type("world.level.block.Block"), "getId", state(name)));
        for (String dir : List.of("NORTH", "SOUTH", "WEST", "EAST")) {
            Object state = call(chest, "setValue", field("world.level.block.HorizontalDirectionalBlock", "FACING"), field("core.Direction", dir));
            states.put("CHEST_" + dir, (int) call(type("world.level.block.Block"), "getId", state));
        }
        Object feature = make("world.level.levelgen.feature.MonsterRoomFeature", field("world.level.levelgen.feature.configurations.NoneFeatureConfiguration", "CODEC"));
        List<Object> samples = new ArrayList<>();
        for (long seed : new long[]{0,1,17,42,-1,Long.MIN_VALUE}) for (String terrain : List.of("tunnel", "closed", "wide", "floor_hole", "roof_hole", "unsupported_floor", "protected", "existing_chest"))
            for (Pos pos : List.of(new Pos(8,32,8), new Pos(-1,-63,16), new Pos(15,315,15))) samples.add(probe(feature, seed, terrain, pos));
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("MONSTER_ROOM_REFERENCE=" + call(gson, "toJson", Map.of("states", states, "ranges", ranges, "state_count", id, "samples", samples)));
    }
}
