// Reflection keeps this probe compilable with JDK 21; run it on the JAR's Java version.
// Invokes vanilla tree shapes and leaf updates, without a server or copied implementation.
import java.lang.reflect.*;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.*;

public class TreeReference {
    static final String MC = "net.minecraft.";
    static Class<?> type(String name) throws Exception { return Class.forName(MC + name); }
    static Object field(String type, String name) throws Exception { return type(type).getField(name).get(null); }
    static boolean matches(Class<?>[] types, Object[] args) {
        if (types.length != args.length) return false;
        for (int i = 0; i < types.length; i++) {
            Class<?> t = types[i];
            if (t == int.class) t = Integer.class;
            if (t == long.class) t = Long.class;
            if (t == boolean.class) t = Boolean.class;
            if (args[i] != null && !t.isInstance(args[i])) return false;
        }
        return true;
    }
    static Object call(Object target, String name, Object... args) throws Exception {
        Class<?> owner = target instanceof Class<?> c ? c : target.getClass();
        for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
            for (Method m : c.getDeclaredMethods()) {
                if (m.getName().equals(name) && matches(m.getParameterTypes(), args)) {
                    m.setAccessible(true);
                    return m.invoke(target instanceof Class<?> ? null : target, args);
                }
            }
        }
        for (Method m : owner.getMethods()) {
            if (m.getName().equals(name) && matches(m.getParameterTypes(), args)) {
                return m.invoke(target instanceof Class<?> ? null : target, args);
            }
        }
        throw new NoSuchMethodException(owner + "." + name);
    }
    static Object make(String name, Object... args) throws Exception {
        for (Constructor<?> c : type(name).getConstructors()) {
            if (matches(c.getParameterTypes(), args)) return c.newInstance(args);
        }
        throw new NoSuchMethodException(name + " constructor");
    }
    static Object state(String block) throws Exception { return call(field("world.level.block.Blocks", block), "defaultBlockState"); }

    static Set<Object> tagBlocks(String name) throws Exception {
        String path = "/data/minecraft/tags/block/" + name.replace("minecraft:", "") + ".json";
        String json;
        try (var stream = TreeReference.class.getResourceAsStream(path)) {
            if (stream == null) throw new IllegalArgumentException("missing tag " + path);
            json = new String(stream.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        Map<?, ?> data = (Map<?, ?>) call(gson, "fromJson", json, Map.class);
        Set<Object> result = new HashSet<>();
        for (Object entry : (List<?>) data.get("values")) {
            String id = entry instanceof String s ? s : (String) ((Map<?, ?>) entry).get("id");
            if (id.startsWith("#")) result.addAll(tagBlocks(id.substring(1)));
            else result.add(field("world.level.block.Blocks", id.replace("minecraft:", "").toUpperCase(Locale.ROOT)));
        }
        return result;
    }
    static void bindTreeTags() throws Exception {
        Map<Object, Set<Object>> tags = new HashMap<>();
        for (String name : List.of("dirt", "logs", "leaves", "replaceable_by_trees", "prevents_nearby_leaf_decay", "cannot_replace_below_tree_trunk")) {
            Object key = call(type("tags.TagKey"), "create", field("core.registries.Registries", "BLOCK"), call(type("resources.Identifier"), "withDefaultNamespace", name));
            for (Object block : tagBlocks(name)) tags.computeIfAbsent(block, b -> new HashSet<>()).add(key);
        }
        for (var entry : tags.entrySet()) call(call(entry.getKey(), "builtInRegistryHolder"), "bindTags", entry.getValue());
    }

    record Pos(int x, int y, int z) {
        static Pos from(Object p) throws Exception {
            return new Pos((int) call(p, "getX"), (int) call(p, "getY"), (int) call(p, "getZ"));
        }
    }
    static Map<Pos, Object> blocks;
    static Object air, grass;
    static Object get(Object pos) throws Exception {
        Pos p = Pos.from(pos);
        return blocks.getOrDefault(p, p.y == 64 ? grass : air);
    }
    static void put(Object pos, Object state, Set<Object> positions) {
        try {
            Object immutable = call(pos, "immutable");
            positions.add(immutable);
            blocks.put(Pos.from(immutable), state);
        } catch (Exception e) { throw new RuntimeException(e); }
    }
    @SuppressWarnings("unchecked")
    static Object level() throws Exception {
        return Proxy.newProxyInstance(TreeReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            return switch (m.getName()) {
                case "getMinY" -> -64;
                case "getMaxY" -> 319;
                case "getHeight" -> 384;
                case "getHeightmapPos" -> {
                    Predicate<Object> predicate = (Predicate<Object>) call(a[0], "isOpaque");
                    Pos pos = Pos.from(a[1]);
                    int top = -64;
                    for (int y = 319; y >= -64; y--) {
                        if (predicate.test(get(make("core.BlockPos", pos.x(), y, pos.z())))) { top = y + 1; break; }
                    }
                    yield make("core.BlockPos", pos.x(), top, pos.z());
                }
                case "getBlockState" -> get(a[0]);
                case "isStateAtPosition" -> ((Predicate<Object>) a[1]).test(get(a[0]));
                case "isFluidAtPosition" -> ((Predicate<Object>) a[1]).test(call(get(a[0]), "getFluidState"));
                case "setBlock" -> { blocks.put(Pos.from(a[0]), a[1]); yield true; }
                default -> throw new UnsupportedOperationException(m.toString());
            };
        });
    }
    static Object config(String kind) throws Exception {
        String path = "/data/minecraft/worldgen/configured_feature/" + kind + ".json";
        String json;
        try (var stream = TreeReference.class.getResourceAsStream(path)) {
            if (stream == null) throw new IllegalArgumentException("missing feature " + path);
            json = new String(stream.readAllBytes(), java.nio.charset.StandardCharsets.UTF_8);
        }
        Object document = call(Class.forName("com.google.gson.JsonParser"), "parseString", json);
        Object configuration = call(call(document, "getAsJsonObject"), "get", "config");
        Object ops = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        return call(call(field("world.level.levelgen.feature.configurations.TreeConfiguration", "CODEC"), "parse", ops, configuration), "getOrThrow");
    }
    static String snapshotHash() throws Exception {
        Map<Object, Integer> ids = new HashMap<>();
        for (Object state : blocks.values()) ids.put(state, (int) call(type("world.level.block.Block"), "getId", state));
        int grassId = (int) call(type("world.level.block.Block"), "getId", grass);
        ByteBuffer buffer = ByteBuffer.allocate(384 * 256 * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int y = -64; y < 320; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
            Object state = blocks.get(new Pos(x, y, z));
            buffer.putInt(state == null ? (y == 64 ? grassId : 0) : ids.get(state));
        }
        return HexFormat.of().formatHex(MessageDigest.getInstance("MD5").digest(buffer.array()));
    }
    public static void main(String[] args) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        bindTreeTags();
        air = state("AIR"); grass = state("GRASS_BLOCK");
        Object tree = make("world.level.levelgen.feature.TreeFeature", field("world.level.levelgen.feature.configurations.TreeConfiguration", "CODEC"));
        List<String> records = new ArrayList<>();
        for (String kind : List.of("oak", "birch", "spruce", "pine", "fancy_oak", "oak_bees_0002_leaf_litter", "fancy_oak_bees_0002_leaf_litter")) for (long seed : new long[]{0, 1, 17, 42}) {
            records.add(probe(tree, kind, seed, "GRASS_BLOCK"));
        }
        for (String soil : List.of("DIRT", "COARSE_DIRT", "PODZOL", "MOSS_BLOCK", "MUD", "ROOTED_DIRT", "FARMLAND", "STONE", "AIR")) {
            records.add(probe(tree, "oak", 0, soil));
        }
        System.out.println("TREE_REFERENCE=[" + String.join(",", records) + "]");
    }

    static String probe(Object tree, String kind, long seed, String soil) throws Exception {
            grass = state(soil);
            blocks = new HashMap<>();
            Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
            Set<Object> roots = new HashSet<>(), logs = new HashSet<>(), leaves = new HashSet<>();
            Object world = level();
            BiConsumer<Object, Object> rootSetter = (pos, state) -> put(pos, state, roots);
            BiConsumer<Object, Object> logSetter = (pos, state) -> put(pos, state, logs);
            Object leafSetter = Proxy.newProxyInstance(TreeReference.class.getClassLoader(), new Class<?>[]{type("world.level.levelgen.feature.foliageplacers.FoliagePlacer$FoliageSetter")}, (p, m, a) -> {
                if (m.getName().equals("set")) { put(a[0], a[1], leaves); return null; }
                if (m.getName().equals("isSet")) return leaves.contains(a[0]);
                throw new UnsupportedOperationException(m.toString());
            });
            Object config = config(kind);
            boolean placed = (boolean) call(tree, "doPlace", world, random, make("core.BlockPos", 8, 65, 8), rootSetter, logSetter, leafSetter, config);
            String shapeHash = snapshotHash();
            Set<Object> decorations = new HashSet<>();
            BiConsumer<Object, Object> decorationSetter = (pos, state) -> put(pos, state, decorations);
            Object context = make("world.level.levelgen.feature.treedecorators.TreeDecorator$Context", world, decorationSetter, random, logs, leaves, roots);
            for (Object decorator : (List<?>) config.getClass().getField("decorators").get(config)) {
                call(decorator, "place", context);
            }
            Set<Object> all = new HashSet<>(roots); all.addAll(logs); all.addAll(leaves);
            all.addAll(decorations);
            Object bounds = ((Optional<?>) call(type("world.level.levelgen.structure.BoundingBox"), "encapsulatingPositions", all)).orElseThrow();
            call(tree, "updateLeaves", world, bounds, logs, decorations, roots);
            String soilField = soil.equals("GRASS_BLOCK") ? "" : ",\"soil\":" + call(type("world.level.block.Block"), "getId", grass);
            return "{\"kind\":\"" + kind + "\",\"seed\":" + seed + soilField + ",\"placed\":" + placed + ",\"shape_md5\":\"" + shapeHash + "\",\"states_md5\":\"" + snapshotHash() + "\",\"next_i64\":" + call(random, "nextLong") + "}";
    }
}
