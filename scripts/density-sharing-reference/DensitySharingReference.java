import java.lang.reflect.*;
import java.util.*;

/** Observe native holder/RandomState/NoiseChunk identity; evaluate the JAR kernel. */
public class DensitySharingReference extends DensityMaterialsReference {
    public static void main(String[] args) throws Exception {
        bootstrap();
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        settings = call(call(registry("NOISE_SETTINGS"), "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD")), "value");
        noises = registry("NOISE");
        registryOps = call(type("resources.RegistryOps"), "create", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), registries);
        densityClass = type("world.level.levelgen.DensityFunction");
        contextClass = type("world.level.levelgen.DensityFunction$FunctionContext");
        compute = densityClass.getMethod("compute", contextClass);
        Object lava = make("world.level.levelgen.Aquifer$FluidStatus", -54, state("LAVA"));
        Object water = make("world.level.levelgen.Aquifer$FluidStatus", 63, state("WATER"));
        picker = Proxy.newProxyInstance(DensitySharingReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.Aquifer$FluidPicker")}, (p, m, a) -> {
                if (m.getName().equals("computeFluid")) return (int) a[1] < -54 ? lava : water;
                throw new UnsupportedOperationException(m.toString());
            });
        List<Object> cases = new ArrayList<>();
        for (String name : List.of("minecraft:overworld/depth", "minecraft:overworld/sloped_cheese",
                "minecraft:overworld/caves/noodle", "minecraft:shift_x")) {
            Object input = Map.of("type", "minecraft:add", "argument1", name, "argument2", name);
            Object root = decode(field("world.level.levelgen.DensityFunction", "DIRECT_CODEC"), input);
            Object a = call(root, "argument1"), b = call(root, "argument2");
            Object holderA = call(a, "function"), holderB = call(b, "function");
            Object config = replaceRecord(settings, "noiseRouter", replaceRecord(call(settings, "noiseRouter"), "finalDensity", root));
            List<Object> seeded = new ArrayList<>();
            for (long seed : new long[]{0, -7, 1234, Long.MIN_VALUE}) {
                Object rs = call(type("world.level.levelgen.RandomState"), "create", config, noises, seed);
                Object mapped = call(call(rs, "router"), "finalDensity");
                Object ma = call(mapped, "argument1"), mb = call(mapped, "argument2");
                Object nc = make("world.level.levelgen.NoiseChunk", 4, rs, -16, 32, call(config, "noiseSettings"),
                    field("world.level.levelgen.DensityFunctions$BeardifierMarker", "INSTANCE"), config, picker,
                    call(type("world.level.levelgen.blending.Blender"), "empty"));
                Object wa = call(nc, "wrap", ma), wb = call(nc, "wrap", mb);
                List<Object> points = new ArrayList<>();
                for (int[] xyz : new int[][]{{-17,-55,31}, {-1,17,-3}, {0,0,0}, {7,63,9}, {29999983,255,-29999983}}) {
                    points.add(Map.of("pos", List.of(xyz[0], xyz[1], xyz[2]),
                        "bits", bits((double) compute.invoke(mapped, context(xyz[0], xyz[1], xyz[2])))));
                }
                seeded.add(Map.of("seed", seed, "mapped_arguments_same", ma == mb,
                    "mapped_class", ma.getClass().getName(), "wrapped_arguments_same", wa == wb, "points", points));
            }
            cases.add(Map.of("name", name, "input", input, "holder_wrappers_same", a == b,
                "holders_same", holderA == holderB, "values_same", call(holderA, "value") == call(holderB, "value"),
                "seeded", seeded));
        }
        output("DENSITY_SHARING_REFERENCE", Map.of("cases", cases));
        call(resources, "close");
    }
}
