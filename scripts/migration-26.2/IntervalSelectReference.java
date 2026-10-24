import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

/** Target codec admission and raw exceptional inputs, in addition to the numeric corpus. */
public class IntervalSelectReference extends NumericReference {
    static Object beardifier() throws Exception {
        Field field = type("world.level.levelgen.DensityFunctions$BeardifierMarker").getDeclaredField("INSTANCE");
        field.setAccessible(true);
        return field.get(null);
    }

    static Object constant(double value, String name, List<String> calls) throws Exception {
        return Proxy.newProxyInstance(IntervalSelectReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.DensityFunction")}, (proxy, method, args) -> {
                if (method.getName().equals("compute")) {
                    calls.add(name);
                    return value;
                }
                throw new UnsupportedOperationException(method.toString());
            });
    }

    public static void main(String[] args) throws Exception {
        NumericReference.main(args);
        Object document = call(Class.forName("com.google.gson.JsonParser"), "parseString",
            Files.readString(Path.of(args[0]), StandardCharsets.UTF_8));
        Object codec = field("world.level.levelgen.DensityFunction", "CODEC");
        List<Object> removed = new ArrayList<>();
        for (Object row : (Iterable<?>) call(member(document, "removed_codecs"), "getAsJsonArray")) {
            Object result = call(codec, "parse", registryOps, member(row, "function"));
            if (((Optional<?>) call(result, "result")).isPresent()) {
                throw new IllegalStateException("removed codec was admitted: " + text(row, "id"));
            }
            Optional<?> error = (Optional<?>) call(result, "error");
            removed.add(Map.of("id", text(row, "id"), "accepted", false,
                "error", call(error.orElseThrow(), "message")));
        }
        List<Object> codecs = new ArrayList<>();
        for (Object row : (Iterable<?>) call(member(document, "interval_codecs"), "getAsJsonArray")) {
            Object result = call(codec, "parse", registryOps, member(row, "function"));
            Optional<?> value = (Optional<?>) call(result, "result");
            Map<String, Object> record = new LinkedHashMap<>();
            record.put("id", text(row, "id"));
            record.put("accepted", value.isPresent());
            if (value.isPresent()) {
                record.put("encoded", call(call(codec, "encodeStart", registryOps, value.get()), "getOrThrow"));
                record.put("value_bits", HexFormat.of().toHexDigits(Double.doubleToRawLongBits(
                    densitySampler(value.get()).sample(new double[]{0, 0, 0}))));
            } else {
                Optional<?> error = (Optional<?>) call(result, "error");
                record.put("error", call(error.orElseThrow(), "message"));
            }
            codecs.add(record);
        }
        Object runtime = member(document, "interval_runtime");
        double[] thresholds = doubles(member(runtime, "thresholds"));
        double[] values = doubles(member(runtime, "functions"));
        Object nativeThresholds = Class.forName("it.unimi.dsi.fastutil.doubles.DoubleArrayList")
            .getConstructor(double[].class).newInstance((Object) thresholds);
        Constructor<?> constructor = type("world.level.levelgen.DensityFunctions$IntervalSelect")
            .getDeclaredConstructor(type("world.level.levelgen.DensityFunction"),
                Class.forName("it.unimi.dsi.fastutil.doubles.DoubleList"), List.class);
        constructor.setAccessible(true);
        List<Object> computations = new ArrayList<>();
        for (Object encoded : (Iterable<?>) call(member(runtime, "inputs"), "getAsJsonArray")) {
            String bits = (String) call(encoded, "getAsString");
            double input = Double.longBitsToDouble(Long.parseUnsignedLong(bits, 16));
            List<String> calls = new ArrayList<>();
            List<Object> functions = new ArrayList<>();
            for (int i = 0; i < values.length; i++) functions.add(constant(values[i], "function/" + i, calls));
            Object function = constructor.newInstance(constant(input, "input", calls), nativeThresholds, functions);
            double value = densitySampler(function).sample(new double[]{0, 0, 0});
            computations.add(Map.of("input_bits", bits, "output_bits",
                HexFormat.of().toHexDigits(Double.doubleToRawLongBits(value)), "calls", calls));
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("INTERVAL_REFERENCE=" + call(gson, "toJson",
            Map.of("codecs", codecs, "runtime", computations, "removed_codecs", removed)));
    }
}
