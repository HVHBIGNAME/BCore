// Compiles on JDK 21 using reflection; the pinned Minecraft JAR runs on Java 25.
// Every payload is emitted by the real NbtIo.writeAnyTag, not a copied encoder.
import java.io.*;
import java.lang.reflect.*;
import java.util.*;

public final class NbtWireReference {
    static Class<?> type(String name) throws Exception {
        return Class.forName("net.minecraft.nbt." + name);
    }

    static Object scalar(String tag, Class<?> primitive, Object value) throws Exception {
        return type(tag).getMethod("valueOf", primitive).invoke(null, value);
    }

    static Object rawFloat(int bits) throws Exception {
        return type("FloatTag").getConstructor(float.class).newInstance(Float.intBitsToFloat(bits));
    }

    static Object rawDouble(long bits) throws Exception {
        return type("DoubleTag").getConstructor(double.class).newInstance(Double.longBitsToDouble(bits));
    }

    static Object string(String value) throws Exception {
        return scalar("StringTag", String.class, value);
    }

    static Object list(Object... values) throws Exception {
        Object result = type("ListTag").getConstructor().newInstance();
        Method add = type("ListTag").getMethod("addTag", int.class, type("Tag"));
        for (int index = 0; index < values.length; index++) {
            if (!(boolean) add.invoke(result, index, values[index])) {
                throw new AssertionError("native list rejected its element");
            }
        }
        return result;
    }

    static Object compound(Map<String, Object> values) throws Exception {
        // Chosen deterministic map order, while delegating all tag writing to MC.
        Constructor<?> constructor = type("CompoundTag").getDeclaredConstructor(Map.class);
        constructor.setAccessible(true);
        return constructor.newInstance(new TreeMap<>(values));
    }

    static Map<String, Object> capture(String name, Object tag) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream output = new DataOutputStream(bytes);
        type("NbtIo").getMethod("writeAnyTag", type("Tag"), DataOutput.class)
            .invoke(null, tag, output);
        output.flush();
        return new LinkedHashMap<>(Map.of("name", name, "hex", HexFormat.of().formatHex(bytes.toByteArray())));
    }

    public static void main(String[] args) throws Exception {
        List<Map<String, Object>> cases = new ArrayList<>();
        cases.add(capture("byte_min", scalar("ByteTag", byte.class, (byte) -128)));
        cases.add(capture("short_min", scalar("ShortTag", short.class, (short) -32768)));
        cases.add(capture("int_min", scalar("IntTag", int.class, Integer.MIN_VALUE)));
        cases.add(capture("long_min", scalar("LongTag", long.class, Long.MIN_VALUE)));
        cases.add(capture("long_max", scalar("LongTag", long.class, Long.MAX_VALUE)));
        cases.add(capture("float_fraction", rawFloat(0x3eaaaaab)));
        cases.add(capture("float_negative_zero", rawFloat(0x80000000)));
        cases.add(capture("float_nan_payload", rawFloat(0xff812345)));
        cases.add(capture("double_fraction", rawDouble(0x3fd5555555555555L)));
        cases.add(capture("double_negative_zero", rawDouble(0x8000000000000000L)));
        cases.add(capture("double_nan_payload", rawDouble(0xfff0000000000001L)));
        cases.add(capture("empty_string", string("")));
        cases.add(capture("unicode_and_nul", string("Привет\u0000😀𝄞")));
        cases.add(capture("byte_array", type("ByteArrayTag").getConstructor(byte[].class)
            .newInstance((Object) new byte[]{-128, -1, 0, 127})));
        cases.add(capture("int_array", type("IntArrayTag").getConstructor(int[].class)
            .newInstance((Object) new int[]{Integer.MIN_VALUE, -1, 0, Integer.MAX_VALUE})));
        cases.add(capture("long_array", type("LongArrayTag").getConstructor(long[].class)
            .newInstance((Object) new long[]{Long.MIN_VALUE, -1, 0, Long.MAX_VALUE})));
        cases.add(capture("empty_list", list()));
        cases.add(capture("int_list", list(scalar("IntTag", int.class, -1), scalar("IntTag", int.class, Integer.MAX_VALUE))));
        cases.add(capture("heterogeneous_list_wrappers", list(string("text"), scalar("IntTag", int.class, 17), list())));
        cases.add(capture("nested", compound(Map.of(
            "Items", list(compound(Map.of("Slot", scalar("ByteTag", byte.class, (byte) 13),
                                           "count", scalar("IntTag", int.class, 1),
                                           "id", string("minecraft:golden_apple")))),
            "LootTableSeed", scalar("LongTag", long.class, Long.MIN_VALUE),
            "empty", compound(Map.of()),
            "list", list(list(), list(string("x"))),
            "name", string("Chest\u0000😀")))));
        cases.add(capture("unicode_key", compound(Map.of("\u0000😀", string("value")))));
        cases.add(capture("empty_compound", compound(Map.of())));
        String json = (String) Class.forName("com.google.gson.Gson").getMethod("toJson", Object.class)
            .invoke(Class.forName("com.google.gson.Gson").getConstructor().newInstance(), cases);
        System.out.println("NBT_WIRE_REFERENCE=" + json);
    }
}
