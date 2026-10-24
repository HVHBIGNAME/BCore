import java.lang.reflect.*;
import java.util.*;

/** Constructor bounds and branch witnesses from the original 26.1 implementations. */
public class AquiferBoundReference extends DensityMaterialsReference {
    static Method substance;
    static Object biomeSource;
    static int cellCount = 4;

    static Object noiseChunk(Object rs, Object config, int cx, int cz) throws Exception {
        return make("world.level.levelgen.NoiseChunk", cellCount, rs, cx * 16, cz * 16,
            call(config, "noiseSettings"), field("world.level.levelgen.DensityFunctions$BeardifierMarker", "INSTANCE"),
            config, picker, call(type("world.level.levelgen.blending.Blender"), "empty"));
    }

    static String biome(Object rs, int cx, int cz, int y) throws Exception {
        Object holder = call(biomeSource, "getNoiseBiome", cx * 4 + 2, y >> 2, cz * 4 + 2, call(rs, "sampler"));
        return call(((Optional<?>) call(holder, "unwrapKey")).orElseThrow(), "identifier").toString();
    }

    static int locations(Object aquifer) throws Exception {
        int count = 0;
        for (long value : (long[]) member(aquifer, "aquiferLocationCache"))
            if (value != Long.MAX_VALUE) count++;
        return count;
    }

    static Map<String, Object> sample(Object aquifer, int x, int y, int z, double density) throws Exception {
        int before = locations(aquifer);
        Object state = substance.invoke(aquifer, context(x, y, z), density);
        return Map.of("pos", List.of(x, y, z), "density_bits", bits(density),
            "state", state == null ? 1 : stateId(state),
            "update", call(aquifer, "shouldScheduleFluidUpdate"),
            "new_locations", locations(aquifer) - before);
    }

    static Map<String, Object> boundCase(String id, long seed, int cx, int cz, Object config,
                                        boolean boundaries) throws Exception {
        Object rs = config == settings ? randomState(seed) :
            call(type("world.level.levelgen.RandomState"), "create", config, noises, seed);
        Object nc = noiseChunk(rs, config, cx, cz);
        Object aquifer = call(nc, "aquifer");
        int bound = (int) member(aquifer, "skipSamplingAboveY");
        int cacheSize = (int) call(member(nc, "preliminarySurfaceLevelCache"), "size");
        if (cacheSize != 121) throw new AssertionError("constructor scan size " + cacheSize);
        List<Object> levels = new ArrayList<>();
        int inside = Integer.MIN_VALUE, outside = Integer.MIN_VALUE;
        for (int z = cz * 16 - 16; z <= cz * 16 + 24; z += 4) {
            for (int x = cx * 16 - 16; x <= cx * 16 + 24; x += 4) {
                int height = (int) call(nc, "preliminarySurfaceLevel", x, z);
                levels.add(List.of(x, z, height));
                if (x >= cx * 16 && x <= cx * 16 + 16 && z >= cz * 16 && z <= cz * 16 + 16)
                    inside = Math.max(inside, height);
                else outside = Math.max(outside, height);
            }
        }
        int highest = (int) call(nc, "maxPreliminarySurfaceLevel", cx * 16 - 16, cz * 16 - 16,
            cx * 16 + 25, cz * 16 + 25);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("id", id); result.put("seed", seed); result.put("chunk", List.of(cx, cz));
        result.put("threshold", bound); result.put("highest", highest);
        result.put("cell_count_xz", cellCount);
        if (config == settings) result.put("biome", biome(rs, cx, cz, Math.max(63, highest)));
        result.put("inside_max", inside); result.put("outside_max", outside);
        result.put("levels", levels); result.put("constructor_cached_levels", cacheSize);
        System.out.println("BOUND_CASE " + id + " H=" + highest + " T=" + bound + " inside=" + inside + " outside=" + outside);
        if (!boundaries) return result;

        List<Object> samples = new ArrayList<>();
        for (int dy : new int[]{-1, 0, 1}) for (double density : new double[]{1.0, 0.0, -0.0, -0.25}) {
            Object fresh = call(noiseChunk(rs, config, cx, cz), "aquifer");
            Map<String, Object> row = sample(fresh, cx * 16 + 7, bound + dy, cz * 16 + 9, density);
            boolean nearest = density <= 0 && dy <= 0 && bound + dy >= -54;
            if (((int) row.get("new_locations") > 0) != nearest) throw new AssertionError(row);
            samples.add(row);
        }
        result.put("samples", samples);

        // Find a real flagged evaluation, then reuse the same aquifer to observe reset.
        Map<String, Object> prime = null;
        outer: for (int y = -54; y <= Math.min(100, bound); y++) {
            for (int z = cz * 16; z < cz * 16 + 16; z++) for (int x = cx * 16; x < cx * 16 + 16; x++) {
                Object state = substance.invoke(aquifer, context(x, y, z), -0.25);
                if ((boolean) call(aquifer, "shouldScheduleFluidUpdate")) {
                    prime = Map.of("pos", List.of(x, y, z), "density_bits", bits(-0.25),
                        "state", state == null ? 1 : stateId(state), "update", true);
                    break outer;
                }
            }
        }
        if (prime != null) {
            List<Object> history = new ArrayList<>();
            List<?> p = (List<?>) prime.get("pos");
            for (double density : new double[]{1.0, 0.0, -0.25}) {
                history.add(sample(aquifer, (int) p.get(0), (int) p.get(1), (int) p.get(2), -0.25));
                history.add(sample(aquifer, cx * 16 + 7, bound + 1, cz * 16 + 9, density));
            }
            result.put("history", history);
        }
        return result;
    }

    static Object withPreliminary(Object value) throws Exception {
        Object f = value instanceof Integer ? method("world.level.levelgen.DensityFunctions", "constant", double.class).invoke(null, ((Integer)value).doubleValue()) :
            decode(field("world.level.levelgen.DensityFunction", "DIRECT_CODEC"), value);
        Object router = replaceRecord(call(settings, "noiseRouter"), "preliminarySurfaceLevel", f);
        return replaceRecord(settings, "noiseRouter", router);
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        settings = call(call(registry("NOISE_SETTINGS"), "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD")), "value");
        noises = registry("NOISE");
        Object parameters = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
            field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), registry("BIOME"));
        biomeSource = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", parameters));
        registryOps = call(type("resources.RegistryOps"), "create", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), registries);
        contextClass = type("world.level.levelgen.DensityFunction$FunctionContext");
        substance = type("world.level.levelgen.Aquifer").getMethod("computeSubstance", contextClass, double.class);
        Object lava = make("world.level.levelgen.Aquifer$FluidStatus", -54, state("LAVA"));
        Object water = make("world.level.levelgen.Aquifer$FluidStatus", 63, state("WATER"));
        picker = Proxy.newProxyInstance(AquiferBoundReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.Aquifer$FluidPicker")}, (p, m, a) -> {
                if (m.getName().equals("computeFluid")) return (int) a[1] < -54 ? lava : water;
                throw new UnsupportedOperationException(m.toString());
            });
        List<Object> cases = new ArrayList<>();
        for (long[] c : new long[][]{{0, 0, 0}, {1, 0, 0}, {-1, -1, -1}, {1234, -34, 3},
                {1234, 1, -1}, {846692123413862008L, -125, 187}, {Long.MIN_VALUE, 1874999, -1874999}}) {
            cases.add(boundCase("natural/" + Arrays.toString(c), c[0], (int)c[1], (int)c[2], settings, true));
        }
        boolean ocean = false, mountain = false;
        for (Object c : cases) {
            int h = (int)((Map<?, ?>)c).get("highest");
            ocean |= ((String)((Map<?, ?>)c).get("biome")).contains("ocean"); mountain |= h > 128;
        }
        // Native-only discovery; selected coordinates and all heights are emitted.
        int minDiscovered = Integer.MAX_VALUE, maxDiscovered = Integer.MIN_VALUE;
        for (int z = -128; z <= 128 && !(ocean && mountain); z += 8) {
            for (int x = -128; x <= 128 && !(ocean && mountain); x += 8) {
                Object nc = noiseChunk(randomState(0), settings, x, z);
                int h = (int) call(nc, "maxPreliminarySurfaceLevel", x * 16 - 16, z * 16 - 16, x * 16 + 25, z * 16 + 25);
                minDiscovered = Math.min(minDiscovered, h); maxDiscovered = Math.max(maxDiscovered, h);
                boolean isOcean = biome(randomState(0), x, z, Math.max(63, h)).contains("ocean");
                if ((!ocean && isOcean) || (!mountain && h > 128)) {
                    cases.add(boundCase(isOcean ? "ocean" : "mountain", 0, x, z, settings, true));
                    ocean |= isOcean; mountain |= h > 128;
                }
            }
        }
        if (!ocean || !mountain) {
            output("PARTIAL_AQUIFER_BOUND_REFERENCE", Map.of("cases", cases));
            throw new AssertionError("missing terrain coverage ocean=" + ocean + " mountain=" + mountain + " H range=" + minDiscovered + ".." + maxDiscovered);
        }
        List<Object> arithmetic = new ArrayList<>();
        for (int h : new int[]{Integer.MIN_VALUE, Integer.MIN_VALUE + 1, -32512, -120, -65, -64, -33, -32, -21, -20, -19, -9, -8, -1, 0, 51, 52, 53, 128, 319,
                Integer.MAX_VALUE - 20, Integer.MAX_VALUE - 19, Integer.MAX_VALUE}) {
            Map<String, Object> row = boundCase("constant/" + h, 0, -1, -1, withPreliminary(h), false);
            row.put("preliminary", h); arithmetic.add(row);
        }
        for (int h : new int[]{-120, -33}) {
            Map<String, Object> row = boundCase("negative-bound/" + h, -1, -1, -1, withPreliminary(h), true);
            row.put("preliminary", h); cases.add(row);
        }
        Object function = Map.of("type", "minecraft:find_top_surface", "upper_bound", 16, "lower_bound", -64, "cell_height", 8,
            "density", Map.of("type", "minecraft:flat_cache", "argument", Map.of("type", "minecraft:add", "argument1", -4,
                "argument2", Map.of("type", "minecraft:y_clamped_gradient", "from_y", -64, "to_y", 320, "from_value", -64, "to_value", 320))));
        Map<String, Object> flat = boundCase("outside-flat-cache", -7, -1, 2, withPreliminary(function), true);
        flat.put("preliminary", function);
        cases.add(flat);
        cellCount = 1;
        Map<String, Object> narrow = boundCase("explicit-geometry-narrow-flat-cache", -7, -1, 2, withPreliminary(function), true);
        narrow.put("preliminary", function); cases.add(narrow);
        output("AQUIFER_BOUND_REFERENCE", Map.of("cases", cases, "arithmetic", arithmetic));
        call(resources, "close");
    }
}
