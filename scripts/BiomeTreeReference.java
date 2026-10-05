// SPDX-License-Identifier: MIT
// Compile with javac 21+, run with Java 25+. No other BCore probe sources are used.
import java.io.ByteArrayOutputStream;
import java.lang.reflect.*;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.*;

/** Captures the target JAR's climate tree, including its observable search history. */
public final class BiomeTreeReference {
    static final String JAR_SHA256 = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52";
    static final List<String> SOURCE_HASH_ORDER = List.of("scripts/BiomeTreeReference.java");
    static final String[] AXES = {"temperature", "humidity", "continentalness", "erosion", "depth", "weirdness"};
    static final long[] SEEDS = {0, 1, -1, 846692123413862008L, Long.MIN_VALUE, Long.MAX_VALUE};
    static ClassLoader loader;

    static Class<?> type(String name) throws ClassNotFoundException {
        return Class.forName(name.startsWith("net.") || name.startsWith("com.") ? name : "net.minecraft." + name, true, loader);
    }

    static Class<?> boxed(Class<?> type) {
        if (type == int.class) return Integer.class;
        if (type == long.class) return Long.class;
        if (type == float.class) return Float.class;
        if (type == double.class) return Double.class;
        if (type == boolean.class) return Boolean.class;
        return type;
    }

    static boolean accepts(Class<?>[] types, Object[] args) {
        if (types.length != args.length) return false;
        for (int i = 0; i < types.length; i++) {
            if (args[i] == null ? types[i].isPrimitive() : !boxed(types[i]).isInstance(args[i])) return false;
        }
        return true;
    }

    static Object call(Object target, String name, Object... args) throws Exception {
        Class<?> owner = target instanceof Class<?> c ? c : target.getClass();
        for (Class<?> current = owner; current != null; current = current.getSuperclass()) {
            Method selected = null;
            for (Method method : current.getDeclaredMethods()) {
                if (method.isBridge() || !method.getName().equals(name) || !accepts(method.getParameterTypes(), args)) continue;
                if (selected != null) throw new IllegalArgumentException("ambiguous method " + owner + "." + name);
                selected = method;
            }
            if (selected != null) {
                selected.setAccessible(true);
                return selected.invoke(target instanceof Class<?> ? null : target, args);
            }
        }
        for (Method method : owner.getMethods()) {
            if (!method.isBridge() && method.getName().equals(name) && accepts(method.getParameterTypes(), args)) {
                return method.invoke(target instanceof Class<?> ? null : target, args);
            }
        }
        throw new NoSuchMethodException(owner + "." + name);
    }

    static Object make(String name, Object... args) throws Exception {
        for (Constructor<?> constructor : type(name).getConstructors()) {
            if (accepts(constructor.getParameterTypes(), args)) return constructor.newInstance(args);
        }
        throw new NoSuchMethodException(name + " constructor");
    }

    static Object constant(String owner, String name) throws Exception {
        return type(owner).getField(name).get(null);
    }

    static Field privateField(String owner, String name) throws Exception {
        Field field = type(owner).getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    static String hash(byte[] bytes) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }

    static Object registries() throws Exception {
        Object pack = call(type("server.packs.repository.ServerPacksSource"), "createVanillaPackSource");
        Object resources = make("server.packs.resources.MultiPackResourceManager", constant("server.packs.PackType", "SERVER_DATA"), List.of(pack));
        try {
            Object root = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", constant("core.registries.BuiltInRegistries", "REGISTRY"));
            List<?> tags = (List<?>) call(type("tags.TagLoader"), "loadTagsForExistingRegistries", resources, root);
            Object lookups = call(type("tags.TagLoader"), "buildUpdatedLookups", root, tags);
            Object loading = call(type("resources.RegistryDataLoader"), "load", resources, lookups,
                constant("resources.RegistryDataLoader", "WORLDGEN_REGISTRIES"), ForkJoinPool.commonPool());
            Object result = ((CompletableFuture<?>) loading).join();
            for (Object pending : tags) call(pending, "apply");
            return result;
        } finally {
            call(resources, "close");
        }
    }

    record Row(String biome, Object point, long[][] ranges, long offset) {
        static Row read(String biome, Object point) throws Exception {
            long[][] ranges = new long[6][2];
            for (int d = 0; d < 6; d++) {
                Object range = call(point, AXES[d]);
                ranges[d][0] = (long) call(range, "min");
                ranges[d][1] = (long) call(range, "max");
            }
            return new Row(biome, point, ranges, (long) call(point, "offset"));
        }

        Map<String, Object> json() {
            return Map.of("biome", biome, "ranges", ranges, "offset", offset);
        }

        long[] center() {
            long[] result = new long[6];
            for (int d = 0; d < 6; d++) result[d] = (ranges[d][0] + ranges[d][1]) / 2;
            return result;
        }
    }

    static Row row(int label, long[][] ranges, long offset) throws Exception {
        Object[] arguments = new Object[7];
        for (int d = 0; d < 6; d++) arguments[d] = make("world.level.biome.Climate$Parameter", ranges[d][0], ranges[d][1]);
        arguments[6] = offset;
        return Row.read("test:biome_" + label, make("world.level.biome.Climate$ParameterPoint", arguments));
    }

    static Row temperature(int label, long min, long max, long offset) throws Exception {
        long[][] ranges = new long[6][2];
        ranges[0] = new long[]{min, max};
        return row(label, ranges, offset);
    }

    static final class Tree {
        final String id;
        final List<Row> rows;
        final Object parameterList;
        final ThreadLocal<?> last;
        final Object root;
        final Method find;
        final Method brute;
        final Constructor<?> target;
        final Field space;
        final Field children;
        final Field value;
        final Method distance;
        int nodes;

        Tree(String id, List<Row> rows) throws Exception {
            this.id = id;
            this.rows = rows;
            List<Object> pairs = new ArrayList<>();
            for (int i = 0; i < rows.size(); i++) pairs.add(call(type("com.mojang.datafixers.util.Pair"), "of", rows.get(i).point, i));
            parameterList = make("world.level.biome.Climate$ParameterList", pairs);
            Object index = privateField("world.level.biome.Climate$ParameterList", "index").get(parameterList);
            last = (ThreadLocal<?>) privateField("world.level.biome.Climate$RTree", "lastResult").get(index);
            root = privateField("world.level.biome.Climate$RTree", "root").get(index);
            Class<?> targetType = type("world.level.biome.Climate$TargetPoint");
            find = parameterList.getClass().getMethod("findValue", targetType);
            brute = parameterList.getClass().getMethod("findValueBruteForce", targetType);
            target = targetType.getConstructor(long.class, long.class, long.class, long.class, long.class, long.class);
            space = privateField("world.level.biome.Climate$RTree$Node", "parameterSpace");
            children = privateField("world.level.biome.Climate$RTree$SubTree", "children");
            value = privateField("world.level.biome.Climate$RTree$Leaf", "value");
            distance = type("world.level.biome.Climate$RTree$Node").getDeclaredMethod("distance", long[].class);
            distance.setAccessible(true);
        }

        Object target(long[] point) throws Exception {
            return target.newInstance(point[0], point[1], point[2], point[3], point[4], point[5]);
        }

        int query(Object point) throws Exception {
            int row = (int) find.invoke(parameterList, point);
            if ((int) value.get(last.get()) != row) throw new IllegalStateException("native cache did not retain returned leaf");
            return row;
        }

        void snapshot(Object node, ByteArrayOutputStream out) throws Exception {
            nodes++;
            boolean leaf = value.getDeclaringClass().isInstance(node);
            out.write(leaf ? 0 : 1);
            ByteBuffer record = ByteBuffer.allocate(7 * 16 + 4).order(ByteOrder.LITTLE_ENDIAN);
            for (Object range : (Object[]) space.get(node)) {
                record.putLong((long) call(range, "min")).putLong((long) call(range, "max"));
            }
            Object[] descendants = leaf ? new Object[0] : (Object[]) children.get(node);
            record.putInt(leaf ? (int) value.get(node) : descendants.length);
            out.writeBytes(record.array());
            for (Object descendant : descendants) snapshot(descendant, out);
        }

        Map<String, Object> json() throws Exception {
            ByteArrayOutputStream snapshot = new ByteArrayOutputStream();
            nodes = 0;
            snapshot(root, snapshot);
            return Map.of("id", id, "rows", rows.stream().map(Row::json).toList(), "nodes", nodes,
                "preorder_sha256_le", hash(snapshot.toByteArray()));
        }
    }

    static int[] order(int size, String name) {
        int[] result = new int[size];
        for (int i = 0; i < size; i++) result[i] = i;
        if (name.equals("reverse")) {
            for (int i = 0; i < size / 2; i++) {
                int other = result[size - i - 1];
                result[size - i - 1] = result[i];
                result[i] = other;
            }
        } else if (name.equals("shuffled")) {
            Random random = new Random(0xBC0261L);
            for (int i = size - 1; i > 0; i--) {
                int j = random.nextInt(i + 1), other = result[j];
                result[j] = result[i];
                result[i] = other;
            }
        } else if (!name.equals("forward")) throw new IllegalArgumentException(name);
        return result;
    }

    static int[] stream(Tree tree, Object[] targets, int[] order, boolean cold) throws Exception {
        tree.last.remove();
        int[] results = new int[targets.length];
        for (int index : order) {
            if (cold) tree.last.remove();
            results[index] = tree.query(targets[index]);
        }
        tree.last.remove();
        return results;
    }

    static Map<String, Object> group(Tree tree, String id, List<long[]> points, Map<String, Object> context) throws Exception {
        Object[] targets = new Object[points.size()];
        for (int i = 0; i < targets.length; i++) targets[i] = tree.target(points.get(i));
        Map<String, Object> result = new LinkedHashMap<>(context);
        result.put("id", id);
        result.put("tree", tree.id);
        result.put("targets", points);
        int[] brute = new int[targets.length];
        for (int i = 0; i < targets.length; i++) brute[i] = (int) tree.brute.invoke(tree.parameterList, targets[i]);
        result.put("linear", brute);
        result.put("cold", stream(tree, targets, order(targets.length, "forward"), true));
        Map<String, Object> streams = new LinkedHashMap<>();
        for (String name : List.of("forward", "reverse", "shuffled")) {
            int[] indices = order(targets.length, name);
            streams.put(name, Map.of("order", indices, "rows", stream(tree, targets, indices, false)));
        }
        result.put("streams", streams);
        return result;
    }

    static List<long[]> boundaryPoints(List<Row> rows) {
        List<long[]> points = new ArrayList<>();
        for (Row row : rows) points.add(row.center());
        for (int d = 0; d < 6; d++) {
            Set<Long> edges = new TreeSet<>();
            for (Row row : rows) for (long edge : row.ranges[d]) edges.add(edge);
            for (long edge : edges) for (long delta : new long[]{-1, 0, 1}) {
                for (int anchor : new int[]{0, rows.size() / 3, rows.size() * 2 / 3, rows.size() - 1}) {
                    long[] point = rows.get(anchor).center();
                    point[d] = edge + delta;
                    points.add(point);
                }
            }
        }
        Random random = new Random(261);
        for (int i = 0; i < 4096; i++) {
            long[] point = new long[6];
            for (int d = 0; d < 6; d++) point[d] = random.nextInt(40001) - 20000;
            points.add(point);
        }
        return points;
    }

    static List<int[]> positions(boolean fullChunks) {
        List<int[]> result = new ArrayList<>();
        if (fullChunks) {
            for (int[] chunk : new int[][]{{0, 0}, {-126, 93}, {62, -63}}) {
                // Native fillBiomesFromNoise: section, quart x, quart y, quart z.
                for (int section = -4; section < 20; section++) for (int x = 0; x < 4; x++)
                    for (int y = 0; y < 4; y++) for (int z = 0; z < 4; z++) {
                        result.add(new int[]{chunk[0] * 4 + x, section * 4 + y, chunk[1] * 4 + z});
                    }
            }
        }
        for (int x : new int[]{-7500000, -1025, -17, -4, -1, 0, 1, 3, 4, 17, 1025, 7499999})
            for (int y : new int[]{-16, -15, -1, 0, 15, 16, 63, 79}) {
                result.add(new int[]{x, y, -x - 1});
            }
        return result;
    }

    static Map<String, Object> routerGroup(Tree tree, Object registries, String settings, long seed) throws Exception {
        Object key = call(type("resources.ResourceKey"), "create", constant("core.registries.Registries", "NOISE_SETTINGS"),
            call(type("resources.Identifier"), "parse", "minecraft:" + settings));
        Object state = call(type("world.level.levelgen.RandomState"), "create", registries, key, seed);
        Object sampler = call(state, "sampler");
        Method sample = sampler.getClass().getMethod("sample", int.class, int.class, int.class);
        Method compute = type("world.level.levelgen.DensityFunction").getMethod("compute", type("world.level.levelgen.DensityFunction$FunctionContext"));
        Object[] fields = new Object[6];
        for (int d = 0; d < 6; d++) fields[d] = call(sampler, AXES[d]);
        List<int[]> positions = positions(settings.equals("overworld"));
        List<long[]> points = new ArrayList<>();
        List<String[]> bits = new ArrayList<>();
        for (int[] p : positions) {
            Object target = sample.invoke(sampler, p[0], p[1], p[2]);
            Object context = make("world.level.levelgen.DensityFunction$SinglePointContext", p[0] << 2, p[1] << 2, p[2] << 2);
            long[] point = new long[6];
            String[] values = new String[6];
            for (int d = 0; d < 6; d++) {
                point[d] = (long) call(target, AXES[d]);
                values[d] = HexFormat.of().toHexDigits(Double.doubleToRawLongBits((double) compute.invoke(fields[d], context)));
            }
            points.add(point);
            bits.add(values);
        }
        return group(tree, "router/" + settings + "/" + seed, points,
            Map.of("seed", seed, "settings", settings, "quart_positions", positions, "climate_f64_bits", bits));
    }

    static List<Row> synthetic(int size) throws Exception {
        Random random = new Random(0xB10CE261L + size);
        List<Row> rows = new ArrayList<>();
        for (int i = 0; i < size; i++) {
            long[][] ranges = new long[6][2];
            for (int d = 0; d < 6; d++) {
                // Repeated centers, asymmetric widths and odd negative sums exercise stable sorts.
                long center = (random.nextInt(7) - 3) * 17L;
                long width = random.nextInt(19);
                ranges[d] = new long[]{center - width, center + width + (i % 3 == 0 ? -1 : 0)};
                if (ranges[d][0] > ranges[d][1]) ranges[d][1] = ranges[d][0];
            }
            rows.add(row(i, ranges, i % 4 == 0 ? 0 : random.nextInt(31)));
        }
        return rows;
    }

    static List<long[]> syntheticPoints(List<Row> rows) {
        List<long[]> points = new ArrayList<>();
        points.add(new long[6]);
        for (Row row : rows) {
            points.add(row.center());
            for (int d = 0; d < 6; d++) for (long edge : row.ranges[d]) {
                long[] point = row.center();
                point[d] = edge;
                points.add(point);
            }
        }
        Random random = new Random(62649);
        for (int i = 0; i < 128; i++) {
            long[] point = new long[6];
            for (int d = 0; d < 6; d++) point[d] = random.nextInt(151) - 75;
            points.add(point);
        }
        return points;
    }

    static Map<String, Object> concurrent(Tree tree, List<long[]> points) throws Exception {
        Object[] targets = new Object[points.size()];
        for (int i = 0; i < points.size(); i++) targets[i] = tree.target(points.get(i));
        ExecutorService executor = Executors.newFixedThreadPool(4);
        CountDownLatch start = new CountDownLatch(1);
        List<Future<Map<String, Object>>> futures = new ArrayList<>();
        try {
            for (int worker = 0; worker < 4; worker++) {
                final int id = worker;
                futures.add(executor.submit(() -> {
                    start.await();
                    int[] indices = order(points.size(), id % 2 == 0 ? "forward" : "reverse");
                    if (id >= 2) {
                        int[] copy = indices.clone();
                        for (int i = 0; i < indices.length; i++) indices[i] = copy[(i + indices.length / 3) % indices.length];
                    }
                    return Map.of("worker", id, "order", indices, "rows", stream(tree, targets, indices, false));
                }));
            }
            start.countDown();
            List<Object> streams = new ArrayList<>();
            for (Future<?> future : futures) streams.add(future.get());
            return Map.of("tree", tree.id, "targets", points, "streams", streams);
        } finally {
            executor.shutdownNow();
        }
    }

    static Map<String, Object> contextSwitches() throws Exception {
        List<Row> rows = List.of(temperature(0, -10, 0, 0), temperature(1, 0, 10, 0));
        Tree first = new Tree("context/a", rows), same = new Tree("context/b", rows);
        Tree changed = new Tree("context/changed", List.of(temperature(1, -10, 0, 0), temperature(0, 0, 10, 0)));
        List<Tree> trees = List.of(first, same, changed);
        List<Object> steps = new ArrayList<>();
        for (int[] step : new int[][]{{0, 10}, {1, -10}, {2, 10}, {0, 0}, {1, 0}, {2, 0}, {1, 10}, {0, 0}, {1, 0}, {2, -10}, {0, 0}, {2, 0}}) {
            Tree tree = trees.get(step[0]);
            long[] point = {step[1], 0, 0, 0, 0, 0};
            steps.add(Map.of("tree", tree.id, "target", point, "row", tree.query(tree.target(point))));
        }
        List<Object> definitions = new ArrayList<>();
        for (Tree tree : trees) definitions.add(tree.json());
        return Map.of("trees", definitions, "steps", steps);
    }

    static List<Object> quantization() throws Exception {
        Method quantize = type("world.level.biome.Climate").getMethod("quantizeCoord", float.class);
        List<Double> values = new ArrayList<>(List.of(0.0, -0.0, Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY,
            (double) Float.MAX_VALUE, (double) -Float.MAX_VALUE, Double.MAX_VALUE, -Double.MAX_VALUE));
        for (float boundary : new float[]{-2, -1, -0.45f, -0.11f, -0.0001f, -Float.MIN_VALUE, Float.MIN_VALUE, 0.0001f, 0.1f, 0.55f, 1, 2}) {
            for (float neighbor : new float[]{Math.nextDown(boundary), boundary, Math.nextUp(boundary)}) {
                values.add(Math.nextDown((double) neighbor));
                values.add((double) neighbor);
                values.add(Math.nextUp((double) neighbor));
            }
        }
        List<Object> result = new ArrayList<>();
        for (double value : values) result.add(Map.of("f64_bits", HexFormat.of().toHexDigits(Double.doubleToRawLongBits(value)),
            "quantized", quantize.invoke(null, (float) value)));
        return result;
    }

    static Map<String, Object> arithmetic(Tree tree) throws Exception {
        Method rangeDistance = type("world.level.biome.Climate$Parameter").getMethod("distance", long.class);
        List<Object> ranges = new ArrayList<>();
        for (long[] bounds : new long[][]{{-10, 10}, {0, 0}, {Long.MIN_VALUE, Long.MIN_VALUE + 9}, {Long.MAX_VALUE - 9, Long.MAX_VALUE}, {5, -5}}) {
            Object range = make("world.level.biome.Climate$Parameter", bounds[0], bounds[1]);
            for (long point : new long[]{Long.MIN_VALUE, Long.MIN_VALUE + 10, -11, -1, 0, 1, 11, Long.MAX_VALUE}) {
                ranges.add(Map.of("range", bounds, "point", point, "distance", rangeDistance.invoke(range, point)));
            }
        }
        List<Object> nodes = new ArrayList<>();
        for (long[] point : new long[][]{new long[7], {Long.MIN_VALUE, 0, 0, 0, 0, 0, 0}, {Long.MAX_VALUE, 0, 0, 0, 0, 0, 0},
            {3037000500L, -3037000500L, 9, -9, 100, -100, 0}}) {
            nodes.add(Map.of("target", point, "distance", tree.distance.invoke(tree.root, (Object) point)));
        }
        return Map.of("ranges", ranges, "root_tree", tree.id, "nodes", nodes);
    }

    static Map<String, Object> capture(Map<String, Object> provenance, Path canonicalOutput) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Object registries = registries();
        Object biomeRegistry = call(registries, "lookupOrThrow", constant("core.registries.Registries", "BIOME"));
        Object preset = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
            constant("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomeRegistry);
        List<Row> rows = new ArrayList<>();
        for (Object pair : (List<?>) call(call(preset, "parameters"), "values")) {
            Object point = call(pair, "getFirst"), holder = call(pair, "getSecond");
            String name = call(call(holder, "key"), "identifier").toString();
            rows.add(Row.read(name, point));
        }
        List<String> ids = new ArrayList<>();
        for (Object biome : (Iterable<?>) biomeRegistry) {
            if ((int) call(biomeRegistry, "getId", biome) != ids.size()) throw new IllegalStateException("non-contiguous native registry");
            ids.add(call(biomeRegistry, "getKey", biome).toString());
        }
        Map<String, Object> canonical = new LinkedHashMap<>(provenance);
        // Preserve the JAR's integer endpoints directly: no codec unquantize/requantize round trip.
        canonical.put("rows", rows.stream().map(Row::json).toList());
        write(canonicalOutput, canonical);

        Tree overworld = new Tree("overworld", rows);
        List<Tree> trees = new ArrayList<>(List.of(overworld));
        List<Object> groups = new ArrayList<>();
        groups.add(group(overworld, "overworld/boundaries", boundaryPoints(rows), Map.of()));
        for (String settings : List.of("overworld", "large_biomes", "amplified")) for (long seed : SEEDS) {
            groups.add(routerGroup(overworld, registries, settings, seed));
        }
        Tree ties = new Tree("synthetic/ties", List.of(temperature(0, -10, 0, 0), temperature(1, 0, 10, 0), temperature(2, -3, 3, 4)));
        trees.add(ties);
        List<long[]> tiePoints = new ArrayList<>();
        for (long x : new long[]{10, 0, -10, 0, 10, 0, -1, 1, 0, 10, 0}) tiePoints.add(new long[]{x, 0, 0, 0, 0, 0});
        groups.add(group(ties, ties.id, tiePoints, Map.of()));
        Tree origin = new Tree("synthetic/origin_order", List.of(temperature(0, -8000, -2000, 0), temperature(1, -1000, 1000, 0)));
        trees.add(origin);
        groups.add(group(origin, origin.id, List.of(new long[]{-1500, 0, 0, 0, 0, 0}, new long[]{-2000, 0, 0, 0, 0, 0},
            new long[]{-1500, 0, 0, 0, 0, 0}, new long[]{-1000, 0, 0, 0, 0, 0}, new long[]{-1500, 0, 0, 0, 0, 0}, new long[6]), Map.of()));
        for (int size : new int[]{1, 2, 6, 7, 35, 36, 37, 215, 216, 217, 1295, 1296, 1297}) {
            Tree tree = new Tree("synthetic/ranged_" + size, synthetic(size));
            trees.add(tree);
            groups.add(group(tree, tree.id, syntheticPoints(tree.rows), Map.of()));
        }
        List<Row> zeroRows = new ArrayList<>();
        for (Row row : synthetic(217)) zeroRows.add(row(zeroRows.size(), row.ranges, 0));
        Collections.reverse(zeroRows);
        Tree zero = new Tree("synthetic/zero_offset_reversed", zeroRows);
        trees.add(zero);
        groups.add(group(zero, zero.id, syntheticPoints(zero.rows), Map.of()));
        groups.add(group(ties, "synthetic/wrapping", List.of(
            new long[]{Long.MIN_VALUE, 0, 0, 0, 0, 0}, new long[]{Long.MAX_VALUE, 0, 0, 0, 0, 0},
            new long[]{3037000500L, -3037000500L, 9, -9, 100, -100}, new long[6]), Map.of()));
        Map<String, Object> result = new LinkedHashMap<>(provenance);
        result.put("registry", ids);
        result.put("dimensions", List.of("temperature", "humidity", "continentalness", "erosion", "depth", "weirdness", "offset"));
        result.put("tree_hash_encoding", "preorder: u8 leaf=0/branch=1; 7*(i64 min,i64 max) LE; u32 LE row/child_count; children in native order");
        List<Object> definitions = new ArrayList<>();
        for (Tree tree : trees) definitions.add(tree.json());
        result.put("trees", definitions);
        result.put("groups", groups);
        result.put("concurrent", List.of(concurrent(overworld, boundaryPoints(rows).subList(0, 1536)), concurrent(ties, tiePoints)));
        result.put("contexts", contextSwitches());
        result.put("quantization", quantization());
        result.put("arithmetic", arithmetic(ties));
        int queries = 0, linearDifferences = 0, cacheDifferences = 0;
        for (Object entry : groups) {
            Map<?, ?> group = (Map<?, ?>) entry;
            int[] linear = (int[]) group.get("linear"), cold = (int[]) group.get("cold");
            Map<?, ?> streams = (Map<?, ?>) group.get("streams");
            int[] forward = (int[]) ((Map<?, ?>) streams.get("forward")).get("rows");
            queries += linear.length;
            for (int i = 0; i < linear.length; i++) {
                if (linear[i] != forward[i]) linearDifferences++;
                if (cold[i] != forward[i]) cacheDifferences++;
            }
        }
        Map<String, Object> summary = Map.of("canonical_rows", rows.size(), "registry_ids", ids.size(), "trees", trees.size(),
            "groups", groups.size(), "query_inputs", queries, "linear_vs_forward_row_differences", linearDifferences,
            "cold_vs_forward_row_differences", cacheDifferences);
        result.put("summary", summary);
        System.out.println("SUMMARY=" + summary);
        return result;
    }

    static void write(Path path, Object value) throws Exception {
        Object gson = type("com.google.gson.Gson").getConstructor().newInstance();
        String json = (String) call(gson, "toJson", value);
        Files.writeString(path, json + "\n", StandardCharsets.UTF_8);
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 3 || args.length > 4) {
            throw new IllegalArgumentException("usage: BiomeTreeReference REPO OUTPUT.json CANONICAL.json [VERIFY.json]");
        }
        Path repo = Path.of(args[0]).toAbsolutePath();
        Path vanilla = repo.resolve("target/vanilla-775"), jar = vanilla.resolve("versions/26.1/server-26.1.jar");
        if (!hash(Files.readAllBytes(jar)).equals(JAR_SHA256)) throw new IllegalStateException("target JAR pin mismatch");
        List<Path> dependencies = new ArrayList<>(List.of(jar));
        try (var files = Files.walk(vanilla.resolve("libraries"))) {
            dependencies.addAll(files.filter(p -> p.toString().endsWith(".jar"))
                .sorted(Comparator.comparing(p -> vanilla.relativize(p).toString().replace('\\', '/'))).toList());
        }
        List<URL> urls = new ArrayList<>();
        for (Path dependency : dependencies) urls.add(dependency.toUri().toURL());
        Map<String, Object> provenance = new LinkedHashMap<>();
        provenance.put("minecraft", "26.1");
        provenance.put("jar_sha256", JAR_SHA256);
        provenance.put("source_hash_order", SOURCE_HASH_ORDER);
        provenance.put("source_hash_encoding", "UTF-8, CRLF normalized to LF, concatenated in source_hash_order");
        ByteArrayOutputStream sources = new ByteArrayOutputStream();
        for (String source : SOURCE_HASH_ORDER) sources.writeBytes(
            Files.readString(repo.resolve(source), StandardCharsets.UTF_8).replace("\r\n", "\n").getBytes(StandardCharsets.UTF_8));
        provenance.put("probe_sha256", hash(sources.toByteArray()));
        List<Object> runtime = new ArrayList<>();
        for (Path dependency : dependencies) runtime.add(Map.of("path", repo.relativize(dependency).toString().replace('\\', '/'),
            "sha256", hash(Files.readAllBytes(dependency))));
        provenance.put("runtime_dependencies", runtime);
        try (URLClassLoader nativeLoader = new URLClassLoader(urls.toArray(URL[]::new), ClassLoader.getPlatformClassLoader())) {
            loader = nativeLoader;
            Thread.currentThread().setContextClassLoader(loader);
            Map<String, Object> result = capture(provenance, Path.of(args[2]));
            write(Path.of(args[1]), result);
            if (args.length == 4) {
                Object actual = call(type("com.google.gson.JsonParser"), "parseString", Files.readString(Path.of(args[1])));
                Object expected = call(type("com.google.gson.JsonParser"), "parseString", Files.readString(Path.of(args[3])));
                if (!actual.equals(expected)) throw new IllegalStateException("native recapture differs from " + args[3]);
            }
            System.out.println("BIOME_TREE_REFERENCE=" + args[1]);
            System.out.println("SOURCE_HASH_ORDER=" + SOURCE_HASH_ORDER);
        }
    }
}
