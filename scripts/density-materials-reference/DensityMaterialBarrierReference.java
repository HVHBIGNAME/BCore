import java.util.*;

/** Additional branch coverage. The existing natural-noise fixture is immutable.
 * Only the three ore router signals are controlled; terrain, aquifer and full
 * NoiseBasedChunkGenerator filling remain the real pinned native implementations.
 */
public class DensityMaterialBarrierReference extends DensityMaterialsReference {
    public static void main(String[] args) throws Exception {
        bootstrap();
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        settings = call(call(registry("NOISE_SETTINGS"), "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD")), "value");
        noises = registry("NOISE");
        densityClass = type("world.level.levelgen.DensityFunction");
        contextClass = type("world.level.levelgen.DensityFunction$FunctionContext");
        compute = densityClass.getMethod("compute", contextClass);
        Object normalSettings = settings;
        Object lava = make("world.level.levelgen.Aquifer$FluidStatus", -54, state("LAVA"));
        Object water = make("world.level.levelgen.Aquifer$FluidStatus", 63, state("WATER"));
        picker = java.lang.reflect.Proxy.newProxyInstance(DensityMaterialBarrierReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.levelgen.Aquifer$FluidPicker")}, (p, m, a) -> {
                if (m.getName().equals("computeFluid")) return (int) a[1] < -54 ? lava : water;
                throw new UnsupportedOperationException(m.toString());
            });
        List<Object> cases = new ArrayList<>();
        for (long[] input : new long[][]{{0, 0, 0}, {1234, -34, 3}, {846692123413862008L, -125, 187}}) {
            for (double toggle : new double[]{0.8, -0.8}) {
                Object router = call(normalSettings, "noiseRouter");
                router = replaceRecord(router, "veinToggle", method("world.level.levelgen.DensityFunctions", "constant", double.class).invoke(null, toggle));
                router = replaceRecord(router, "veinRidged", method("world.level.levelgen.DensityFunctions", "constant", double.class).invoke(null, -0.01));
                router = replaceRecord(router, "veinGap", method("world.level.levelgen.DensityFunctions", "constant", double.class).invoke(null, 0.0));
                settings = replaceRecord(normalSettings, "noiseRouter", router);
                Map<String, Object> result = noiseCase("pressure-barrier/" + Arrays.toString(input) + "/" + toggle,
                    input[0], (int) input[1], (int) input[2], List.of(), true);
                result.put("vein_functions", Map.of("vein_toggle", toggle, "vein_ridged", -0.01, "vein_gap", 0.0));
                cases.add(result);
            }
        }
        output("DENSITY_MATERIALS_REFERENCE", Map.of("cases", cases, "states", describeStates(),
            "signal_order", List.of("full_density", "vein_toggle", "vein_ridged", "vein_gap")));
        call(resources, "close");
    }
}
