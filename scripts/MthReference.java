import java.lang.reflect.*;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;

/** Reads the native SIN table and invokes exact Mth signatures on raw-bit inputs. */
public class MthReference extends TreeReference {
    static final HexFormat HEX = HexFormat.of();

    static Field declaredField(Class<?> owner, String name, Class<?> expected) throws Exception {
        Field field = owner.getDeclaredField(name);
        if (field.getType() != expected || !Modifier.isStatic(field.getModifiers()))
            throw new IllegalStateException("unexpected field: " + field);
        field.setAccessible(true);
        return field;
    }

    static Method doubleMethod(Class<?> owner, String name) throws Exception {
        Method method = owner.getDeclaredMethod(name, double.class);
        if (method.getReturnType() != float.class || !Modifier.isStatic(method.getModifiers()))
            throw new IllegalStateException("unexpected method: " + method);
        return method;
    }

    static boolean hasFloatOverload(Class<?> owner, String name) throws Exception {
        try {
            owner.getDeclaredMethod(name, float.class);
            return true;
        } catch (NoSuchMethodException absent) {
            return false;
        }
    }

    static void neighborhood(Map<Integer, String> floats, Map<Long, String> doubles,
                             String label, double center, int ulps) {
        float f = (float) center;
        double d = center;
        for (int i = 0; i < ulps; i++) {
            f = Math.nextDown(f);
            d = Math.nextDown(d);
        }
        for (int i = -ulps; i <= ulps; i++) {
            floats.putIfAbsent(Float.floatToRawIntBits(f), label + "/ulp=" + i);
            doubles.putIfAbsent(Double.doubleToRawLongBits(d), label + "/ulp=" + i);
            f = Math.nextUp(f);
            d = Math.nextUp(d);
        }
    }

    static Map<String, Object> sample(String precision, String label, String inputBits,
                                      Object input, Method sin, Method cos) throws Exception {
        Map<String, Object> row = new LinkedHashMap<>();
        row.put("precision", precision);
        row.put("case", label);
        row.put("input_bits", inputBits);
        // Reflection widens a boxed Float to the declared double parameter.
        row.put("sin_bits", HEX.toHexDigits(Float.floatToRawIntBits((float) sin.invoke(null, input))));
        row.put("cos_bits", HEX.toHexDigits(Float.floatToRawIntBits((float) cos.invoke(null, input))));
        return row;
    }

    public static void main(String[] args) throws Exception {
        Class<?> mth = type("util.Mth");
        Method sin = doubleMethod(mth, "sin"), cos = doubleMethod(mth, "cos");
        Map<String, Boolean> floatOverloads = new LinkedHashMap<>();
        floatOverloads.put("sin", hasFloatOverload(mth, "sin"));
        floatOverloads.put("cos", hasFloatOverload(mth, "cos"));
        if (floatOverloads.containsValue(true))
            throw new IllegalStateException("26.1 float inputs must widen to double: " + floatOverloads);

        double scale = declaredField(mth, "SIN_SCALE", double.class).getDouble(null);
        int mask = declaredField(mth, "SIN_MASK", int.class).getInt(null);
        int offset = declaredField(mth, "COS_OFFSET", int.class).getInt(null);
        float[] table = (float[]) declaredField(mth, "SIN", float[].class).get(null);
        if (table.length != 65536) throw new IllegalStateException("unexpected SIN length: " + table.length);

        Map<Integer, String> floats = new LinkedHashMap<>();
        Map<Long, String> doubles = new LinkedHashMap<>();
        for (int bits : new int[] {
            0x00000000, 0x80000000, 0x00000001, 0x80000001,
            0x007fffff, 0x807fffff, 0x00800000, 0x80800000,
            0x3f800000, 0xbf800000, 0x7f7fffff, 0xff7fffff,
            0x7f800000, 0xff800000, 0x7fc00000, 0xffc00000,
            0x7fc12345, 0xffc12345, 0x7f800001, 0xff800001
        }) floats.put(bits, "raw/" + HEX.toHexDigits(bits));
        for (long bits : new long[] {
            0x0000000000000000L, 0x8000000000000000L,
            0x0000000000000001L, 0x8000000000000001L,
            0x000fffffffffffffL, 0x800fffffffffffffL,
            0x0010000000000000L, 0x8010000000000000L,
            0x3ff0000000000000L, 0xbff0000000000000L,
            0x7fefffffffffffffL, 0xffefffffffffffffL,
            0x7ff0000000000000L, 0xfff0000000000000L,
            0x7ff8000000000000L, 0xfff8000000000000L,
            0x7ff8123456789abcL, 0xfff8123456789abcL,
            0x7ff0000000000001L, 0xfff0000000000001L
        }) doubles.put(bits, "raw/" + HEX.toHexDigits(bits));

        for (double root : new double[] {
            1e-300, 1e-40, 1e-12, 1e-6, 0.5, 1.0,
            Math.PI / 2, Math.PI, Math.PI * 2, Math.PI * 2048,
            1000.0, 1e6, 1e12, 1e15, 1e30, 1e300
        }) {
            neighborhood(floats, doubles, "angle/+" + root, root, 2);
            neighborhood(floats, doubles, "angle/-" + root, -root, 2);
        }
        for (double target : new double[] {
            -131073, -65537, -65536, -32769, -32768,
            -16385, -16384, -16383, -2, -1, 0, 1, 2,
            16383, 16384, 16385, 32767, 32768, 32769,
            49151, 49152, 49153, 65535, 65536, 65537, 131073,
            -2147483649.0, -2147483648.0, -2147483647.0,
            2147483647.0, 2147483648.0, 2147483649.0,
            -0x1.0p52, 0x1.0p52, -0x1.0p63, 0x1.0p63
        }) {
            for (int phase : new int[] {0, offset}) {
                neighborhood(floats, doubles, "index/" + target + "/offset=" + phase,
                    (target - phase) / scale, Math.abs(target) >= 0x1.0p63 ? 8 : 2);
            }
        }
        neighborhood(floats, doubles, "multiply-overflow/+", Double.MAX_VALUE / scale, 8);
        neighborhood(floats, doubles, "multiply-overflow/-", -Double.MAX_VALUE / scale, 8);

        Random random = new Random(0x4d5448323631L);
        for (int i = 0; i < 256; i++) {
            floats.putIfAbsent(random.nextInt(), "random-bits/" + i);
            doubles.putIfAbsent(random.nextLong(), "random-bits/" + i);
            neighborhood(floats, doubles, "finite-angle/" + i,
                (random.nextDouble() * 2 - 1) * Math.PI * 8, 0);
        }

        List<Map<String, Object>> samples = new ArrayList<>();
        for (var entry : floats.entrySet()) {
            int bits = entry.getKey();
            samples.add(sample("f32", entry.getValue(), HEX.toHexDigits(bits),
                Float.valueOf(Float.intBitsToFloat(bits)), sin, cos));
        }
        for (var entry : doubles.entrySet()) {
            long bits = entry.getKey();
            samples.add(sample("f64", entry.getValue(), HEX.toHexDigits(bits),
                Double.valueOf(Double.longBitsToDouble(bits)), sin, cos));
        }

        List<String> tableBits = new ArrayList<>(table.length);
        ByteBuffer rawTable = ByteBuffer.allocate(table.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (float value : table) {
            int bits = Float.floatToRawIntBits(value);
            tableBits.add(HEX.toHexDigits(bits));
            rawTable.putInt(bits);
        }
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("java_version", System.getProperty("java.version"));
        result.put("methods", List.of(sin.toGenericString(), cos.toGenericString()));
        result.put("float_overloads", floatOverloads);
        result.put("sin_scale_bits", HEX.toHexDigits(Double.doubleToRawLongBits(scale)));
        result.put("sin_mask", mask);
        result.put("cos_offset", offset);
        result.put("sin_table_length", table.length);
        result.put("sin_table_sha256_le", HEX.formatHex(MessageDigest.getInstance("SHA-256").digest(rawTable.array())));
        result.put("sample_counts", Map.of("f32", floats.size(), "f64", doubles.size()));
        result.put("samples", samples);
        result.put("sin_table_bits", tableBits);
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        String json = (String) gson.getClass().getMethod("toJson", Object.class).invoke(gson, result);
        System.out.println("MTH_REFERENCE=" + json);
    }
}
