import java.lang.reflect.*;
import java.io.*;
import java.util.*;
import java.util.function.Function;

/** Calls the unmodified configured feature and native region/proto storage.
 * GeodeReference supplies only constructed native chunks and bootstrap plumbing.
 * All input terrain and rejection policies are explicit; no Rust is consulted.
 */
public class DesertWellReference extends GeodeReference {
    static Object typed(Object tag) throws Exception {
        if (tag == null) return null;
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        call(type("nbt.NbtIo"), "writeAnyTag", tag, new DataOutputStream(bytes));
        DataInputStream in = new DataInputStream(new ByteArrayInputStream(bytes.toByteArray()));
        int id = in.readUnsignedByte();
        Object result = List.of(id, payload(in, id));
        if (in.available() != 0) throw new IllegalStateException("trailing NBT");
        return result;
    }

    static Object payload(DataInputStream in, int id) throws Exception {
        return switch (id) {
            case 1 -> in.readByte(); case 2 -> in.readShort(); case 3 -> in.readInt();
            case 4 -> in.readLong(); case 5 -> in.readFloat(); case 6 -> in.readDouble();
            case 8 -> in.readUTF();
            case 7, 11, 12 -> {
                List<Object> values = new ArrayList<>();
                int n = in.readInt();
                for (int i = 0; i < n; i++) values.add(payload(in, id == 7 ? 1 : id == 11 ? 3 : 4));
                yield values;
            }
            case 9 -> {
                int element = in.readUnsignedByte(), n = in.readInt();
                List<Object> values = new ArrayList<>();
                for (int i = 0; i < n; i++) values.add(payload(in, element));
                yield Map.of("element_type", element, "values", values);
            }
            case 10 -> {
                Map<String,Object> fields = new TreeMap<>();
                for (int kind; (kind = in.readUnsignedByte()) != 0;) fields.put(in.readUTF(), List.of(kind, payload(in, kind)));
                yield fields;
            }
            default -> throw new IllegalArgumentException("NBT " + id);
        };
    }

    static Map<String, Object> entity(World w, Pos p) throws Exception {
        Object chunk = call(w.region, "getChunk", at(p));
        Object be = call(chunk, "getBlockEntity", at(p));
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("pending", typed(call(chunk, "getBlockEntityNbt", at(p))));
        out.put("full", be == null ? null : typed(call(be, "saveWithFullMetadata", registries)));
        out.put("update", be == null ? null : typed(call(be, "getUpdateTag", registries)));
        return out;
    }

    static Map<String, Object> sample(String name, long seed, Pos origin, int top, String soil, String defect, int radius, String reject, int repeats) throws Exception {
        Map<Pos, Object> initial = new TreeMap<>(ORDER);
        for (int x = -3; x <= 3; x++) for (int z = -3; z <= 3; z++) for (int y = top - 3; y <= top; y++)
            initial.put(new Pos(origin.x() + x, y, origin.z() + z), block(y == top ? soil : "STONE"));
        Pos hole = new Pos(origin.x() - 2, top - 1, origin.z() - 2);
        if (defect.equals("one_air") || defect.equals("two_air")) initial.put(hole, block("AIR"));
        if (defect.equals("two_air")) initial.put(new Pos(hole.x(), hole.y() - 1, hole.z()), block("CAVE_AIR"));
        if (defect.equals("water_support")) {
            initial.put(hole, block("WATER"));
            initial.put(new Pos(hole.x(), hole.y() - 1, hole.z()), block("AIR"));
        }
        if (defect.equals("obstruction")) initial.put(new Pos(origin.x(), origin.y() + 1, origin.z()), block("STONE"));
        Setup setup = new Setup(origin, block("AIR"), initial, radius, 2, false, false, 1, false);
        World w = new World(setup, 42L);
        List<Object> initialEntities = new ArrayList<>();
        if (defect.startsWith("existing_")) {
            for (int depth = 1; depth <= 2; depth++) for (int[] d : new int[][]{{0,0},{1,0},{0,1},{-1,0},{0,-1}}) {
                Pos p = new Pos(origin.x() + d[0], top - depth, origin.z() + d[1]);
                Object s = block(defect.equals("existing_wrong") ? "CHEST" : "SUSPICIOUS_GRAVEL");
                call(w.region, "setBlock", at(p), s, 2, 512);
                initial.put(p, s);
                if (defect.equals("existing_live")) {
                    Object be = call(w.region, "getBlockEntity", at(p));
                    call(be, "setLootTable", field("world.level.storage.loot.BuiltInLootTables", "DESERT_PYRAMID_ARCHAEOLOGY"), 17L);
                }
                initialEntities.add(Map.of("pos", xyz(p), "data", entity(w, p)));
            }
        }
        List<Object> events = new ArrayList<>();
        Object proxy = Proxy.newProxyInstance(DesertWellReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            try {
                switch (m.getName()) {
                    case "isEmptyBlock", "getBlockState": {
                        Pos pos = Pos.from(a[0]);
                        Object s = call(w.region, "getBlockState", a[0]);
                        events.add(Map.of("op", "read", "pos", xyz(pos), "state", id(s)));
                        return m.getName().equals("isEmptyBlock") ? call(s, "isAir") : s;
                    }
                    case "getMinY": return call(w.region, "getMinY");
                    case "ensureCanWrite": {
                        boolean ok = (boolean) call(w.region, "ensureCanWrite", a[0]);
                        events.add(Map.of("op", "guard", "pos", xyz(Pos.from(a[0])), "accepted", ok));
                        return ok;
                    }
                    case "setBlock": {
                        Pos pos = Pos.from(a[0]);
                        boolean denied = reject.equals("all") || (reject.equals("archaeology") && id(a[1]) == id(block("SUSPICIOUS_SAND")));
                        boolean ok = !denied && (boolean) call(w.region, "setBlock", a[0], a[1], a[2], 512);
                        w.touched.add(pos);
                        events.add(Map.of("op", "write", "pos", xyz(pos), "state", id(a[1]), "flags", a[2], "accepted", ok));
                        return ok;
                    }
                    case "getBlockEntity": {
                        Pos pos = Pos.from(a[0]);
                        Map<String, Object> before = entity(w, pos);
                        Object result = m.invoke(w.region, a);
                        events.add(Map.of("op", "lookup", "pos", xyz(pos), "present", ((Optional<?>) result).isPresent(), "before", before, "after", entity(w, pos)));
                        return result;
                    }
                    default: throw new UnsupportedOperationException(m.toString());
                }
            } catch (InvocationTargetException e) { throw e.getCause(); }
        });
        Object configured = decode(resource("worldgen/configured_feature/desert_well"));
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        List<Object> results = new ArrayList<>();
        for (int i = 0; i < repeats; i++) results.add(call(configured, "place", proxy, generator, random, at(origin)));
        List<Object> entities = new ArrayList<>();
        for (Pos p : w.touched) {
            Map<String, Object> data = entity(w, p);
            if (data.get("pending") != null || data.get("full") != null) entities.add(Map.of("pos", xyz(p), "data", data));
        }
        List<int[]> states = new ArrayList<>();
        for (var e : initial.entrySet()) states.add(new int[]{e.getKey().x(), e.getKey().y(), e.getKey().z(), id(e.getValue())});
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("name", name); out.put("seed", seed); out.put("origin", xyz(origin));
        out.put("write_radius", radius); out.put("reject", reject); out.put("repeats", repeats);
        out.put("initial", states); out.put("initial_entities", initialEntities);
        out.put("placed", results); out.put("events", events); out.put("entities", entities);
        out.put("next_i64", call(random, "nextLong")); out.put("world", w.result());
        return out;
    }

    static Object locate() throws Exception {
        Object biomes = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        Object params = make("world.level.biome.MultiNoiseBiomeSourceParameterList", field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomes);
        Object source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", params));
        List<?> possible = new ArrayList<>((Set<?>) call(source, "possibleBiomes"));
        Function<Object,Object> features = h -> {
            try { return call(call(call(h, "value"), "getGenerationSettings"), "features"); }
            catch (Exception e) { throw new RuntimeException(e); }
        };
        List<?> steps = (List<?>) call(type("world.level.biome.FeatureSorter"), "buildFeaturesPerStep", possible, features, true);
        Object placed = call(holder(call(registries, "lookupOrThrow", field("core.registries.Registries", "PLACED_FEATURE")), "desert_well"), "value");
        int step = -1, index = -1;
        for (int i = 0; i < steps.size(); i++) {
            int j = ((List<?>) call(steps.get(i), "features")).indexOf(placed);
            if (j >= 0) { step = i; index = j; }
        }
        if (step < 0) throw new IllegalStateException("native sorter lacks well");
        long seed = 42L;
        Object settings = call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS"));
        Object randomState = call(type("world.level.levelgen.RandomState"), "create", registries, field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD"), seed);
        Object sampler = call(randomState, "sampler");
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", 0L));
        List<Object> candidates = new ArrayList<>();
        // Only native RNG and biome calls select inputs; original-server history is the oracle.
        for (int cz = -256; cz <= 256 && candidates.size() < 8; cz++) for (int cx = -256; cx <= 256 && candidates.size() < 8; cx++) {
            long decoration = (long) call(random, "setDecorationSeed", seed, cx * 16, cz * 16);
            call(random, "setFeatureSeed", decoration, index, step);
            float f = ((Number) call(random, "nextFloat")).floatValue();
            if (f >= 1.0F / 1000.0F) continue;
            int x = cx * 16 + (int) call(random, "nextInt", 16), z = cz * 16 + (int) call(random, "nextInt", 16);
            Object biome = call(source, "getNoiseBiome", x >> 2, 16, z >> 2, sampler);
            String name = call(call(biome, "key"), "identifier").toString();
            if (name.equals("minecraft:desert")) candidates.add(Map.of("chunk", new int[]{cx,cz}, "xz", new int[]{x,z}, "biome", name, "rarity_float", f));
        }
        return Map.of("scope", "Native input selection only; not parity evidence", "seed", seed, "step", step, "index", index, "candidates", candidates);
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        Object result;
        if (args.length > 0 && args[0].equals("locate")) result = locate();
        else {
            List<Object> samples = new ArrayList<>();
            for (long seed : new long[]{0, 1, 2, 17, 42, -1, Long.MIN_VALUE, Long.MAX_VALUE})
                samples.add(sample("flat_" + seed, seed, new Pos(8,65,8), 64, "SAND", "none", 1, "none", 1));
            samples.add(sample("descending", 42, new Pos(8,90,8), 64, "SAND", "none", 1, "none", 1));
            for (String soil : List.of("RED_SAND", "SANDSTONE", "WATER", "AIR"))
                samples.add(sample("soil_" + soil, 42, new Pos(8,65,8), 64, soil, "none", 1, "none", 1));
            for (String defect : List.of("one_air", "two_air", "water_support", "obstruction"))
                samples.add(sample(defect, 17, new Pos(8,65,8), 64, "SAND", defect, 1, "none", 1));
            for (int top : new int[]{-64,-63,-62,-61,316,319})
                samples.add(sample("height_" + top, 42, new Pos(8,top+1,8), top, "SAND", "none", 1, "none", 1));
            for (Pos p : List.of(new Pos(0,65,0), new Pos(-1,65,-16), new Pos(-17,65,15)))
                for (int radius : new int[]{0,1}) samples.add(sample("edge_" + p + "_" + radius, 42, p, 64, "SAND", "none", radius, "none", 1));
            for (String reject : List.of("all", "archaeology"))
                samples.add(sample("reject_" + reject, 42, new Pos(8,65,8), 64, "SAND", "none", 1, reject, 1));
            for (String defect : List.of("existing_pending", "existing_live", "existing_wrong"))
                samples.add(sample(defect, 42, new Pos(0,65,0), 64, "SAND", defect, 1, "all", 1));
            samples.add(sample("repeat", 42, new Pos(8,65,8), 64, "SAND", "none", 1, "none", 2));
            result = Map.of("scope", "Native ConfiguredFeature.place + WorldGenRegion/ProtoChunk, constructed explicit terrain; no gameplay ticks; POI callback record-only", "samples", samples);
        }
        System.out.println("DESERT_WELL_REFERENCE=" + call(gson, "toJson", result));
    }
}
