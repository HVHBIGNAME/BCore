import java.lang.reflect.*;
import java.util.*;

/** Native aquifer states, driven by explicit input densities before features. */
public class AquiferReference extends OreReference {
    public static void main(String[] args) throws Exception {
        bootstrapOre();
        Object lookup = call(type("data.registries.VanillaRegistries"), "createLookup");
        Object settingsRegistry = call(lookup, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS"));
        Object settingsHolder = call(settingsRegistry, "getOrThrow", field("world.level.levelgen.NoiseGeneratorSettings", "OVERWORLD"));
        Object settings = call(settingsHolder, "value");
        Object noiseRegistry = call(lookup, "lookupOrThrow", field("core.registries.Registries", "NOISE"));
        Object lavaStatus = make("world.level.levelgen.Aquifer$FluidStatus", -54, state("LAVA"));
        Object waterStatus = make("world.level.levelgen.Aquifer$FluidStatus", 63, water);
        Object picker = Proxy.newProxyInstance(AquiferReference.class.getClassLoader(), new Class<?>[]{type("world.level.levelgen.Aquifer$FluidPicker")}, (p, m, a) -> {
            if (!m.getName().equals("computeFluid")) throw new UnsupportedOperationException(m.toString());
            return (int) a[1] < -54 ? lavaStatus : waterStatus;
        });
        Method compute = type("world.level.levelgen.Aquifer").getMethod("computeSubstance", type("world.level.levelgen.DensityFunction$FunctionContext"), double.class);
        List<Object> samples = new ArrayList<>();
        List<Object> centers = new ArrayList<>();
        for (long seed : new long[]{0, 1, -1, 846692123413862008L}) {
            Object randomState = call(type("world.level.levelgen.RandomState"), "create", settings, noiseRegistry, seed);
            Object factory = call(randomState, "aquiferRandom");
            for (Pos c : List.of(new Pos(0, 0, 0), new Pos(-1, -5, 1), new Pos(-127, 3, 188), new Pos(1875000, 26, -1875000))) {
                Object random = call(factory, "at", c.x(), c.y(), c.z());
                centers.add(Map.of("seed", seed, "grid", List.of(c.x(), c.y(), c.z()), "center", List.of(c.x() * 16 + (int) call(random, "nextInt", 10), c.y() * 12 + (int) call(random, "nextInt", 9), c.z() * 16 + (int) call(random, "nextInt", 10))));
            }
            for (int[] xz : new int[][]{{0,0},{15,15},{-1,-1},{1000,0},{-2000,3000},{-2001,3007},{29999983,-29999983}}) {
                terrain = "air"; surface = 64; base = stone; baseFactory = factory(stone); chunks = new HashMap<>();
                Object noiseChunk = call(type("world.level.levelgen.NoiseChunk"), "forChunk", chunk(xz[0] >> 4, xz[1] >> 4), randomState,
                    field("world.level.levelgen.DensityFunctions$BeardifierMarker", "INSTANCE"), settings, picker, call(type("world.level.levelgen.blending.Blender"), "empty"));
                Object aquifer = call(noiseChunk, "aquifer");
                for (double density : new double[]{0.0, -0.25, 1.0}) {
                    List<Integer> states = new ArrayList<>();
                    List<Boolean> updates = new ArrayList<>();
                    for (int y = -64; y < 320; y++) {
                        Object context = make("world.level.levelgen.DensityFunction$SinglePointContext", xz[0], y, xz[1]);
                        Object state = compute.invoke(aquifer, context, density);
                        states.add(state == null ? 1 : (int) call(type("world.level.block.Block"), "getId", state));
                        updates.add((boolean) call(aquifer, "shouldScheduleFluidUpdate"));
                    }
                    samples.add(Map.of("seed", seed, "x", xz[0], "z", xz[1], "density", density, "states", states, "updates", updates));
                }
            }
        }
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("AQUIFER_REFERENCE=" + call(gson, "toJson", Map.of("samples", samples, "centers", centers,
            "no_fluid", field("world.level.dimension.DimensionType", "WAY_BELOW_MIN_Y"))));
    }
}
