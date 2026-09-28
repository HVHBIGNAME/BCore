import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

/** Calls the JAR's math/noise/density implementations on explicit JSON inputs. */
public class NumericReference extends TreeReference {
    static Object registries, settings, noises, registryOps;
    static final Map<Long, Object> randomStates = new HashMap<>();

    interface Sampler { double sample(double[] point) throws Exception; }

    static Object member(Object json, String name) throws Exception {
        Object value = call(call(json, "getAsJsonObject"), "get", name);
        if (value == null) throw new IllegalArgumentException("missing field " + name);
        return value;
    }
    static String text(Object json, String name) throws Exception {
        return (String) call(member(json, name), "getAsString");
    }
    static double number(Object json, String name) throws Exception {
        return (double) call(member(json, name), "getAsDouble");
    }
    static long seed(Object json) throws Exception {
        // Gson's JsonElement retains the integer; deserializing to Map loses bits.
        return (long) call(member(json, "seed"), "getAsLong");
    }
    static double[] doubles(Object json) throws Exception {
        List<Double> values = new ArrayList<>();
        for (Object element : (Iterable<?>) call(json, "getAsJsonArray"))
            values.add((double) call(element, "getAsDouble"));
        double[] result = new double[values.size()];
        for (int i = 0; i < result.length; i++) result[i] = values.get(i);
        return result;
    }
    static Object key(String registry, String name) throws Exception {
        return call(type("resources.ResourceKey"), "create",
            field("core.registries.Registries", registry),
            call(type("resources.Identifier"), "parse", name));
    }
    static Object randomState(long seed) throws Exception {
        Object state = randomStates.get(seed);
        if (state == null) {
            state = call(type("world.level.levelgen.RandomState"), "create", settings, noises, seed);
            randomStates.put(seed, state);
        }
        return state;
    }
    static Object replaceRecord(Object record, String field, Object replacement) throws Exception {
        RecordComponent[] components = record.getClass().getRecordComponents();
        Class<?>[] types = new Class<?>[components.length];
        Object[] values = new Object[components.length];
        boolean found = false;
        for (int i = 0; i < components.length; i++) {
            types[i] = components[i].getType();
            if (components[i].getName().equals(field)) {
                values[i] = replacement;
                found = true;
            } else values[i] = components[i].getAccessor().invoke(record);
        }
        if (!found) throw new IllegalArgumentException("no record component " + field);
        return record.getClass().getConstructor(types).newInstance(values);
    }
    static Object densityState(Object json, long seed) throws Exception {
        Object function = call(call(field("world.level.levelgen.DensityFunction", "HOLDER_HELPER_CODEC"),
            "parse", registryOps, json), "getOrThrow");
        Object router = replaceRecord(call(settings, "noiseRouter"), "finalDensity", function);
        Object customSettings = replaceRecord(settings, "noiseRouter", router);
        // Native RandomState performs the entire noise/holder/BlendedNoise wiring.
        return call(type("world.level.levelgen.RandomState"), "create", customSettings, noises, seed);
    }
    static Object context(double[] p) throws Exception {
        for (double v : p) {
            if (!Double.isFinite(v) || v != (int) v)
                throw new IllegalArgumentException("density coordinates must be int32: " + Arrays.toString(p));
        }
        return make("world.level.levelgen.DensityFunction$SinglePointContext", (int) p[0], (int) p[1], (int) p[2]);
    }
    static Sampler densitySampler(Object function) throws Exception {
        Method compute = type("world.level.levelgen.DensityFunction").getMethod("compute",
            type("world.level.levelgen.DensityFunction$FunctionContext"));
        return p -> (double) compute.invoke(function, context(p));
    }
    static Method doubleMethod(String owner, String name, int count) throws Exception {
        Class<?>[] types = new Class<?>[count];
        Arrays.fill(types, double.class);
        return type(owner).getMethod(name, types);
    }
    static Sampler triple(Object receiver, String owner, String method) throws Exception {
        Method sample = doubleMethod(owner, method, 3);
        return p -> (double) sample.invoke(receiver, p[0], p[1], p[2]);
    }
    static Sampler noiseChunk(long seed, Object function) throws Exception {
        Object state = function == null ? randomState(seed) : densityState(function, seed);
        Object noiseSettings = call(settings, "noiseSettings");
        int minY = (int) call(noiseSettings, "minY");
        int height = (int) call(noiseSettings, "height");
        int width = (int) call(noiseSettings, "getCellWidth");
        int cellHeight = (int) call(noiseSettings, "getCellHeight");
        Object fluid = make("world.level.levelgen.Aquifer$FluidStatus", 63, state("WATER"));
        Object picker = Proxy.newProxyInstance(NumericReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.Aquifer$FluidPicker")}, (p, m, a) -> {
                if (m.getName().equals("computeFluid")) return fluid;
                throw new UnsupportedOperationException(m.toString());
            });
        Method updateY = type("world.level.levelgen.NoiseChunk").getMethod("updateForY", int.class, double.class);
        Method updateX = type("world.level.levelgen.NoiseChunk").getMethod("updateForX", int.class, double.class);
        Method updateZ = type("world.level.levelgen.NoiseChunk").getMethod("updateForZ", int.class, double.class);
        return p -> {
            context(p); // Validate before converting.
            int x = (int) p[0], y = (int) p[1], z = (int) p[2];
            if (y < minY || y >= minY + height) throw new IllegalArgumentException("Y outside noise settings");
            int x0 = Math.floorDiv(x, width) * width, z0 = Math.floorDiv(z, width) * width;
            int y0 = Math.floorDiv(y, cellHeight) * cellHeight;
            Object chunk = make("world.level.levelgen.NoiseChunk", 1, state, x0, z0, noiseSettings,
                field("world.level.levelgen.DensityFunctions$BeardifierMarker", "INSTANCE"), settings,
                picker, call(type("world.level.levelgen.blending.Blender"), "empty"));
            call(chunk, "initializeForFirstCellX");
            try {
                call(chunk, "advanceCellX", 0);
                call(chunk, "selectCellYZ", Math.floorDiv(y - minY, cellHeight), 0);
                updateY.invoke(chunk, y, (double) (y - y0) / cellHeight);
                updateX.invoke(chunk, x, (double) (x - x0) / width);
                updateZ.invoke(chunk, z, (double) (z - z0) / width);
                return (double) call(chunk, "getInterpolatedDensity");
            } finally {
                call(chunk, "stopInterpolation");
            }
        };
    }
    static String routerMethod(String field) {
        return switch (field) {
            case "barrier" -> "barrierNoise";
            case "fluid_level_floodedness" -> "fluidLevelFloodednessNoise";
            case "fluid_level_spread" -> "fluidLevelSpreadNoise";
            case "lava" -> "lavaNoise";
            case "temperature", "vegetation", "continents", "erosion", "depth", "ridges" -> field;
            case "preliminary_surface_level" -> "preliminarySurfaceLevel";
            case "final_density" -> "finalDensity";
            case "vein_toggle" -> "veinToggle";
            case "vein_ridged" -> "veinRidged";
            case "vein_gap" -> "veinGap";
            default -> throw new IllegalArgumentException("unknown router field " + field);
        };
    }
    static Sampler sampler(Object c) throws Exception {
        return switch (text(c, "op")) {
            case "smoothstep", "wrap" -> {
                boolean fade = text(c, "op").equals("smoothstep");
                Method method = doubleMethod(fade ? "util.Mth" : "world.level.levelgen.synth.PerlinNoise",
                    fade ? "smoothstep" : "wrap", 1);
                yield p -> (double) method.invoke(null, p[0]);
            }
            case "lerp" -> {
                if (text(c, "precision").equals("f32")) {
                    Method method = type("util.Mth").getMethod("lerp", float.class, float.class, float.class);
                    yield p -> (float) method.invoke(null, (float) p[0], (float) p[1], (float) p[2]);
                }
                yield triple(null, "util.Mth", "lerp");
            }
            case "clamped_lerp" -> triple(null, "util.Mth", "clampedLerp");
            case "improved" -> {
                Object noise = make("world.level.levelgen.synth.ImprovedNoise",
                    make("world.level.levelgen.XoroshiroRandomSource", seed(c)));
                Method method = doubleMethod("world.level.levelgen.synth.ImprovedNoise", "noise", 5);
                double scale = number(c, "y_scale"), fudge = number(c, "y_fudge");
                yield p -> (double) method.invoke(noise, p[0], p[1], p[2], scale, fudge);
            }
            case "perlin" -> {
                Object random = make("world.level.levelgen.XoroshiroRandomSource", seed(c));
                Object amplitudes = Class.forName("it.unimi.dsi.fastutil.doubles.DoubleArrayList")
                    .getConstructor(double[].class).newInstance((Object) doubles(member(c, "amplitudes")));
                Object noise = call(type("world.level.levelgen.synth.PerlinNoise"), "create", random,
                    (int) call(member(c, "first_octave"), "getAsInt"), amplitudes);
                yield triple(noise, "world.level.levelgen.synth.PerlinNoise", "getValue");
            }
            case "normal" -> {
                Object noise = call(randomState(seed(c)), "getOrCreateNoise", key("NOISE", text(c, "noise")));
                yield triple(noise, "world.level.levelgen.synth.NormalNoise", "getValue");
            }
            case "blended" -> {
                Object root = make("world.level.levelgen.XoroshiroRandomSource", seed(c));
                Object random = call(call(root, "forkPositional"), "fromHashOf", "minecraft:terrain");
                Object noise = type("world.level.levelgen.synth.BlendedNoise")
                    .getConstructor(type("util.RandomSource"), double.class, double.class, double.class, double.class, double.class)
                    .newInstance(random, number(c, "xz_scale"), number(c, "y_scale"), number(c, "xz_factor"),
                        number(c, "y_factor"), number(c, "smear"));
                yield densitySampler(noise);
            }
            case "density" -> densitySampler(call(call(densityState(member(c, "function"), seed(c)), "router"), "finalDensity"));
            case "router" -> densitySampler(call(call(randomState(seed(c)), "router"), routerMethod(text(c, "field"))));
            case "noise_chunk" -> noiseChunk(seed(c), call(call(c, "getAsJsonObject"), "get", "function"));
            default -> throw new IllegalArgumentException("unknown operation " + text(c, "op"));
        };
    }
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("usage: NumericReference requests.json");
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        registries = NativeWorldgenRegistries.load();
        settings = call(call(call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS")),
            "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD")), "value");
        noises = call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE"));
        Object jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        registryOps = call(type("resources.RegistryOps"), "create", jsonOps, registries);
        Object document = call(Class.forName("com.google.gson.JsonParser"), "parseString",
            Files.readString(Path.of(args[0]), StandardCharsets.UTF_8));
        Object points = member(document, "points"), cases = member(document, "cases");
        List<Object> samples = new ArrayList<>();
        Set<String> ids = new HashSet<>();
        for (Object c : (Iterable<?>) call(cases, "getAsJsonArray")) {
            String id = text(c, "id");
            if (!ids.add(id)) throw new IllegalArgumentException("duplicate case " + id);
            try {
                Sampler sample = sampler(c);
                boolean f32 = text(c, "op").equals("lerp") && text(c, "precision").equals("f32");
                List<String> bits = new ArrayList<>();
                for (Object point : (Iterable<?>) call(member(points, text(c, "points")), "getAsJsonArray")) {
                    double[] p = doubles(point);
                    if (p.length != 3) throw new IllegalArgumentException("points must have three coordinates");
                    double value = sample.sample(p);
                    bits.add(f32 ? HexFormat.of().toHexDigits(Float.floatToRawIntBits((float) value))
                        : HexFormat.of().toHexDigits(Double.doubleToRawLongBits(value)));
                }
                samples.add(Map.of("id", id, "bits", bits));
            } catch (Exception e) {
                throw new IllegalStateException("numeric case " + id, e);
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("NUMERIC_REFERENCE=" + call(gson, "toJson", Map.of("points", points, "cases", cases, "samples", samples)));
    }
}
