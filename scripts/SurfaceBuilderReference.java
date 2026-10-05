import java.lang.reflect.*;
import java.nio.*;
import java.security.MessageDigest;
import java.util.*;

/** Runs the real 26.1 SurfaceSystem.buildSurface on fully populated ProtoChunks.
 * Only the input slabs, biome source and optional NoiseChunk preliminary-cache
 * entries are controlled. Rules, random state, noise, extensions, heightmap
 * updates and fluid postprocessing all execute in the server JAR.
 */
public class SurfaceBuilderReference extends TreeReference {
    static final String GEN = "world.level.levelgen.";
    static Object registries, biomeRegistry, settingsHolder, settings, factory, height, jsonOps;
    static Object[] states;
    static boolean[] fluids;
    static final IdentityHashMap<Object, Integer> stateIds = new IdentityHashMap<>();
    static Method sectionGet, sectionSet, getHeight, buildSurface, sampleNoise;
    static final Map<String, Object> biomeHolders = new TreeMap<>();
    static final Map<Long, Environment> environments = new HashMap<>();
    static final List<String> MIXED = List.of("plains", "desert", "snowy_slopes", "eroded_badlands",
        "frozen_ocean", "deep_frozen_ocean", "swamp", "stony_peaks", "badlands");

    static Object registryValue(String registry, String name) throws Exception {
        Object key = call(type("resources.ResourceKey"), "create", field("core.registries.Registries", registry),
            call(type("resources.Identifier"), "parse", name));
        return call(call(registries, "lookupOrThrow", field("core.registries.Registries", registry)), "getOrThrow", key);
    }

    static Object member(Object target, String owner, String name) throws Exception {
        Field f = type(owner).getDeclaredField(name);
        f.setAccessible(true);
        return f.get(target);
    }

    static int id(String name) throws Exception { return stateIds.get(state(name)); }
    static String bits(double value) { return HexFormat.of().toHexDigits(Double.doubleToRawLongBits(value)); }

    static void bootstrap() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        registries = NativeWorldgenRegistries.load();
        biomeRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        settingsHolder = registryValue("NOISE_SETTINGS", "minecraft:overworld");
        settings = call(settingsHolder, "value");
        factory = call(type("world.level.chunk.PalettedContainerFactory"), "create", registries);
        height = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        int count = (int) call(field("world.level.block.Block", "BLOCK_STATE_REGISTRY"), "size");
        states = new Object[count]; fluids = new boolean[count];
        Method stateById = type("world.level.block.Block").getMethod("stateById", int.class);
        for (int i = 0; i < count; i++) {
            states[i] = stateById.invoke(null, i); stateIds.put(states[i], i);
            fluids[i] = !(boolean) call(call(states[i], "getFluidState"), "isEmpty");
        }
        for (Object b : (Iterable<?>) biomeRegistry) {
            String name = call(biomeRegistry, "getKey", b).toString();
            biomeHolders.put(name.substring("minecraft:".length()), registryValue("BIOME", name));
        }
        sectionGet = type("world.level.chunk.LevelChunkSection").getMethod("getBlockState", int.class, int.class, int.class);
        sectionSet = type("world.level.chunk.LevelChunkSection").getMethod("setBlockState", int.class, int.class, int.class,
            type("world.level.block.state.BlockState"), boolean.class);
        getHeight = type("world.level.chunk.ChunkAccess").getMethod("getHeight", type(GEN + "Heightmap$Types"), int.class, int.class);
        buildSurface = type(GEN + "SurfaceSystem").getMethod("buildSurface", type(GEN + "RandomState"),
            type("world.level.biome.BiomeManager"), type("core.Registry"), boolean.class, type(GEN + "WorldGenerationContext"),
            type("world.level.chunk.ChunkAccess"), type(GEN + "NoiseChunk"), type(GEN + "SurfaceRules$RuleSource"));
        sampleNoise = type(GEN + "synth.NormalNoise").getMethod("getValue", double.class, double.class, double.class);
    }

    static final class Environment {
        final long seed;
        final Object randomState, system, source, sampler, generator, picker, rule;
        Environment(long seed) throws Exception {
            this.seed = seed;
            randomState = call(type(GEN + "RandomState"), "create", settings,
                call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE")), seed);
            system = call(randomState, "surfaceSystem");
            Object parameters = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
                field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomeRegistry);
            source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", parameters));
            sampler = call(randomState, "sampler");
            generator = make(GEN + "NoiseBasedChunkGenerator", source, settingsHolder);
            rule = call(settings, "surfaceRule");
            Object lava = make(GEN + "Aquifer$FluidStatus", -54, state("LAVA"));
            Object water = make(GEN + "Aquifer$FluidStatus", 63, state("WATER"));
            picker = Proxy.newProxyInstance(getClass().getClassLoader(), new Class<?>[]{type(GEN + "Aquifer$FluidPicker")}, (p, m, a) -> {
                if (!m.getName().equals("computeFluid")) throw new UnsupportedOperationException(m.toString());
                return (int) a[1] < -54 ? lava : water;
            });
        }
        double noise(String field, int x, int z, double scale) throws Exception {
            return (double) sampleNoise.invoke(member(system, GEN + "SurfaceSystem", field), x * scale, 0.0, z * scale);
        }
    }

    static Environment environment(long seed) throws Exception {
        Environment env = environments.get(seed);
        if (env == null) { env = new Environment(seed); environments.put(seed, env); }
        return env;
    }

    static Object parseRule(Object spec) throws Exception {
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        Object json = call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", spec));
        return call(call(field(GEN + "SurfaceRules$RuleSource", "CODEC"), "parse", jsonOps, json), "getOrThrow");
    }

    static Object blockRule(String block) { return Map.of("type", "minecraft:block", "result_state", Map.of("Name", "minecraft:" + block)); }
    static Object when(Object condition, Object rule) { return Map.of("type", "minecraft:condition", "if_true", condition, "then_run", rule); }
    static Object condition(String type) { return Map.of("type", "minecraft:" + type); }
    static Object depth(String direction, int offset, boolean add, int secondary) {
        return Map.of("type", "minecraft:stone_depth", "surface_type", direction, "offset", offset,
            "add_surface_depth", add, "secondary_depth_range", secondary);
    }
    static Object sequence(Object... rules) { return Map.of("type", "minecraft:sequence", "sequence", List.of(rules)); }
    static Object yAbove(int y) { return Map.of("type", "minecraft:y_above", "anchor", Map.of("absolute", y), "surface_depth_multiplier", 0, "add_stone_depth", false); }

    static Map<String, Object> rules() throws Exception {
        Map<String, Object> rules = new LinkedHashMap<>();
        rules.put("overworld", call(call(field(GEN + "SurfaceRules$RuleSource", "CODEC"), "encodeStart", jsonOps,
            call(settings, "surfaceRule")), "getOrThrow"));
        rules.put("depth", sequence(
            when(depth("ceiling", 0, false, 0), blockRule("calcite")),
            when(depth("floor", 0, false, 0), blockRule("grass_block")),
            when(depth("floor", 0, true, 0), blockRule("dirt")),
            when(depth("ceiling", 1, true, 7), blockRule("sandstone")),
            when(depth("floor", 1, true, 23), blockRule("red_sandstone")), blockRule("stone")));
        Object waterAt = Map.of("type", "minecraft:water", "offset", -1, "surface_depth_multiplier", 0, "add_stone_depth", false);
        Object waterAbove = Map.of("type", "minecraft:water", "offset", -6, "surface_depth_multiplier", -1, "add_stone_depth", true);
        rules.put("water", sequence(when(waterAt, blockRule("dirt")), when(waterAbove, blockRule("sand")), blockRule("gravel")));
        rules.put("preliminary", sequence(when(condition("above_preliminary_surface"), blockRule("red_sand")), blockRule("stone")));
        rules.put("fluid_writes", sequence(when(depth("floor", 0, false, 0), blockRule("water")), blockRule("stone")));
        rules.put("steep", sequence(when(condition("steep"), blockRule("snow_block")), blockRule("dirt")));
        // On clamped edges, changing the top before the first steep read matters.
        rules.put("steep_after_air", sequence(when(yAbove(83), blockRule("air")),
            when(condition("steep"), blockRule("snow_block")), blockRule("dirt")));
        rules.put("temperature", sequence(when(condition("temperature"), blockRule("ice")), blockRule("water")));
        return rules;
    }

    static int[] profile(String name) throws Exception {
        int[] input = new int[384 * 256];
        int stone = id("STONE"), water = id("WATER"), lava = id("LAVA"), cave = id("CAVE_AIR"), voidAir = id("VOID_AIR");
        Object waterloggedSpec = Map.of("Name", "minecraft:oak_slab", "Properties", Map.of("type", "bottom", "waterlogged", "true"));
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        Object json = call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", waterloggedSpec));
        int waterlogged = stateIds.get(call(call(field("world.level.block.state.BlockState", "CODEC"), "parse", jsonOps, json), "getOrThrow"));
        for (int x = 0; x < 16; x++) for (int z = 0; z < 16; z++) {
            int top = switch (name) {
                case "flat" -> 84;
                case "terraces" -> 55 + x * 9 + z * 2;
                case "south_up" -> 80 + z * 2;
                case "north_up" -> 110 - z * 2;
                case "west_up" -> 110 - x * 2;
                case "east_up" -> 80 + x * 2;
                case "ocean", "lava_ocean" -> 36 + ((x + 3 * z) % 7);
                case "low" -> 60;
                case "ceiling" -> 319;
                case "empty" -> -65;
                case "slabs" -> 110;
                default -> throw new IllegalArgumentException(name);
            };
            for (int y = -64; y < 320; y++) {
                int state = y <= top ? stone : 0;
                if ((name.equals("ocean") || name.equals("lava_ocean")) && y > top && y < 63)
                    state = (name.equals("ocean") ? water : lava) + ((x + z) % 3 == 0 ? 7 : 0);
                if (name.equals("slabs")) {
                    if ((y >= -42 && y <= -39) || (y >= 24 && y <= 28) || (y >= 67 && y <= 70)) state = cave;
                    if (y >= 40 && y <= 42) state = water + 7;
                    if (y >= 84 && y <= 86) state = lava + 3;
                    if (y == 98) state = waterlogged;
                    if (y == 101) state = id("TUFF");
                    if (y == 110 && (x & 1) == 0) state = id("DIRT");
                    if (y == -64 && (z & 1) == 0) state = voidAir;
                    if (x == 15 && z == 15 && y >= 116 && y <= 119) state = stone;
                }
                input[(y + 64) * 256 + z * 16 + x] = state;
            }
        }
        return input;
    }

    static Map<String, Object> encodeColumns(int[] voxels) {
        Map<List<Integer>, Integer> indices = new LinkedHashMap<>();
        List<Integer> columns = new ArrayList<>();
        for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
            List<Integer> runs = new ArrayList<>();
            int current = voxels[z * 16 + x];
            for (int y = -63; y <= 320; y++) {
                int next = y == 320 ? -1 : voxels[(y + 64) * 256 + z * 16 + x];
                if (current != next) { runs.add(y); runs.add(current); current = next; }
            }
            columns.add(indices.computeIfAbsent(runs, r -> indices.size()));
        }
        return Map.of("palette", new ArrayList<>(indices.keySet()), "columns", columns);
    }

    static String hash(int[] values) throws Exception {
        ByteBuffer bytes = ByteBuffer.allocate(values.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int value : values) bytes.putInt(value);
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes.array()));
    }

    static Object chunk(int cx, int cz, int[] input) throws Exception {
        Object chunk = make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", cx, cz),
            field("world.level.chunk.UpgradeData", "EMPTY"), height, factory, null);
        Object[] sections = (Object[]) call(chunk, "getSections");
        for (int y = -64; y < 320; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
            int id = input[(y + 64) * 256 + z * 16 + x];
            if (id != 0) sectionSet.invoke(sections[(y + 64) >> 4], x, y & 15, z, states[id], false);
        }
        for (Object section : sections) call(section, "recalcBlockCounts");
        call(chunk, "setPersistedStatus", field("world.level.chunk.status.ChunkStatus", "NOISE"));
        call(type(GEN + "Heightmap"), "primeHeightmaps", chunk,
            Set.of(field(GEN + "Heightmap$Types", "WORLD_SURFACE_WG"), field(GEN + "Heightmap$Types", "OCEAN_FLOOR_WG")));
        return chunk;
    }

    static int[] snapshot(Object chunk) throws Exception {
        Object[] sections = (Object[]) call(chunk, "getSections");
        int[] result = new int[384 * 256];
        for (int y = -64; y < 320; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++)
            result[(y + 64) * 256 + z * 16 + x] = stateIds.get(sectionGet.invoke(sections[(y + 64) >> 4], x, y & 15, z));
        return result;
    }

    static List<Object> posts(Object chunk) throws Exception {
        List<Object> result = new ArrayList<>();
        Object[] sections = (Object[]) call(chunk, "getPostProcessing");
        for (int s = 0; s < sections.length; s++) if (sections[s] != null)
            for (Object packed : (Iterable<?>) sections[s]) {
                int value = ((Number) packed).intValue() & 65535;
                result.add(List.of(value & 15, -64 + s * 16 + ((value >> 4) & 15), (value >> 8) & 15));
            }
        return result;
    }

    static Map<String, Object> capture(String id, long seed, int cx, int cz, String biome, String profile,
            String ruleName, Map<String, Object> rules, Map<String, int[]> inputs, int[] prescribed) throws Exception {
        Environment env = environment(seed);
        Object chunk = chunk(cx, cz, inputs.get(profile));
        Object noiseChunk = call(type(GEN + "NoiseChunk"), "forChunk", chunk, env.randomState,
            field(GEN + "DensityFunctions$BeardifierMarker", "INSTANCE"), settings, env.picker, call(type(GEN + "blending.Blender"), "empty"));
        List<Integer> corners = new ArrayList<>();
        Object cache = member(noiseChunk, GEN + "NoiseChunk", "preliminarySurfaceLevelCache");
        for (int i = 0; i < 4; i++) {
            int x = cx * 16 + (i & 1) * 16, z = cz * 16 + (i >> 1) * 16;
            if (prescribed != null) call(cache, "put", call(type("world.level.ChunkPos"), "pack", x, z), prescribed[i]);
            corners.add((int) call(noiseChunk, "preliminarySurfaceLevel", x, z));
        }
        Map<Pos, Object> biomes = new HashMap<>();
        Object source = Proxy.newProxyInstance(SurfaceBuilderReference.class.getClassLoader(),
            new Class<?>[]{type("world.level.biome.BiomeManager$NoiseBiomeSource")}, (p, m, a) -> {
                if (!m.getName().equals("getNoiseBiome")) throw new UnsupportedOperationException(m.toString());
                int qx = (int) a[0], qy = Math.clamp((int) a[1], -16, 79), qz = (int) a[2];
                Pos key = new Pos(qx, qy, qz);
                Object result = biomes.get(key);
                if (result == null) {
                    result = biome.equals("overworld") ? call(env.source, "getNoiseBiome", qx, qy, qz, env.sampler)
                        : biomeHolders.get(biome.equals("mixed") ? MIXED.get(Math.floorMod(qx * 31 + qz * 17 + qy, MIXED.size())) : biome);
                    if (result == null) throw new IllegalArgumentException("missing biome " + biome);
                    biomes.put(key, result);
                }
                return result;
            });
        Object manager = make("world.level.biome.BiomeManager", source, call(type("world.level.biome.BiomeManager"), "obfuscateSeed", seed));
        Object context = make(GEN + "WorldGenerationContext", env.generator, height);
        Object rule = ruleName.equals("overworld") ? env.rule : parseRule(rules.get(ruleName));
        MessageDigest contextDigest = MessageDigest.getInstance("SHA-256");
        int[] contextCount = {0};
        Object observedRule = Proxy.newProxyInstance(SurfaceBuilderReference.class.getClassLoader(),
            new Class<?>[]{type(GEN + "SurfaceRules$RuleSource")}, (p, m, a) -> {
                if (!m.getName().equals("apply")) throw new UnsupportedOperationException(m.toString());
                Object ruleContext = a[0];
                Object nativeRule = call(rule, "apply", ruleContext);
                Method apply = type(GEN + "SurfaceRules$SurfaceRule").getMethod("tryApply", int.class, int.class, int.class);
                List<Field> fields = new ArrayList<>();
                for (String name : List.of("blockX", "blockY", "blockZ", "stoneDepthAbove", "stoneDepthBelow", "waterHeight", "surfaceDepth")) {
                    Field f = type(GEN + "SurfaceRules$Context").getDeclaredField(name); f.setAccessible(true); fields.add(f);
                }
                Method minSurface = type(GEN + "SurfaceRules$Context").getDeclaredMethod("getMinSurfaceLevel"); minSurface.setAccessible(true);
                ByteBuffer bytes = ByteBuffer.allocate(36).order(ByteOrder.LITTLE_ENDIAN);
                return Proxy.newProxyInstance(SurfaceBuilderReference.class.getClassLoader(),
                    new Class<?>[]{type(GEN + "SurfaceRules$SurfaceRule")}, (q, n, b) -> {
                        if (!n.getName().equals("tryApply")) throw new UnsupportedOperationException(n.toString());
                        Object state = apply.invoke(nativeRule, b);
                        bytes.clear();
                        for (Field f : fields) bytes.putInt(f.getInt(ruleContext));
                        bytes.putInt((int) minSurface.invoke(ruleContext));
                        bytes.putInt(state == null ? -1 : stateIds.get(state));
                        contextDigest.update(bytes.array()); contextCount[0]++;
                        return state;
                    });
            });
        buildSurface.invoke(env.system, env.randomState, manager, biomeRegistry, false, context, chunk, noiseChunk, observedRule);
        int[] output = snapshot(chunk);
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("id", id); result.put("seed", seed); result.put("chunk", List.of(cx, cz));
        result.put("biome_source", biome); result.put("profile", profile); result.put("rule", ruleName);
        result.put("preliminary_mode", prescribed == null ? "native" : "prescribed"); result.put("preliminary_corners", corners);
        result.put("states", encodeColumns(output)); result.put("states_sha256", hash(output));
        result.put("rule_context_sha256", HexFormat.of().formatHex(contextDigest.digest())); result.put("rule_calls", contextCount[0]);
        result.put("postprocessing", posts(chunk));
        Map<Integer, Integer> counts = new TreeMap<>();
        for (int i = 0; i < output.length; i++) if (output[i] != inputs.get(profile)[i]) counts.merge(output[i], 1, Integer::sum);
        result.put("changed_to", counts);
        List<Integer> depths = new ArrayList<>(), heights = new ArrayList<>();
        List<String> secondary = new ArrayList<>();
        for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
            depths.add((int) call(env.system, "getSurfaceDepth", cx * 16 + x, cz * 16 + z));
            secondary.add(bits((double) call(env.system, "getSurfaceSecondary", cx * 16 + x, cz * 16 + z)));
            heights.add((int) getHeight.invoke(chunk, field(GEN + "Heightmap$Types", "WORLD_SURFACE_WG"), x, z) + 1);
        }
        result.put("surface_depths", depths); result.put("surface_secondary_bits", secondary); result.put("world_surface", heights);
        if (biome.equals("overworld")) {
            Map<Object, Integer> paletteIds = new IdentityHashMap<>();
            List<String> names = new ArrayList<>();
            List<Integer> quarts = new ArrayList<>();
            for (int qy = -16; qy <= 79; qy++) for (int qz = cz * 4 - 1; qz <= cz * 4 + 4; qz++) for (int qx = cx * 4 - 1; qx <= cx * 4 + 4; qx++) {
                Pos key = new Pos(qx, qy, qz);
                Object holder = biomes.get(key);
                if (holder == null) holder = call(env.source, "getNoiseBiome", qx, qy, qz, env.sampler);
                Integer index = paletteIds.get(holder);
                if (index == null) {
                    index = names.size(); paletteIds.put(holder, index); names.add(call(call(holder, "key"), "identifier").toString());
                }
                quarts.add(index);
            }
            result.put("noise_biomes", Map.of("palette", names, "quarts", quarts));
        }
        return result;
    }

    static int[] witness(long seed, boolean ice, boolean warm) throws Exception {
        Environment env = environment(seed);
        Random random = new Random(261);
        for (int i = 0; i < 65536; i++) {
            int x = random.nextInt(32768) - 16384, z = random.nextInt(32768) - 16384;
            double strength = ice ? Math.min(Math.abs(env.noise("icebergSurfaceNoise", x, z, 1) * 8.25), env.noise("icebergPillarNoise", x, z, 1.28) * 15)
                : Math.min(Math.abs(env.noise("badlandsSurfaceNoise", x, z, 1) * 8.25), env.noise("badlandsPillarNoise", x, z, 0.2) * 15);
            if (strength < (ice ? 6 : 4)) continue;
            if (ice) {
                double roof = Math.abs(env.noise("icebergPillarRoofNoise", x, z, 1.17) * 1.5);
                if (Math.min(strength * strength * 1.2, Math.ceil(roof * 40) + 14) < 31) continue;
                Object b = call(biomeHolders.get("frozen_ocean"), "value");
                boolean melts = (boolean) call(b, "shouldMeltFrozenOceanIcebergSlightly", make("core.BlockPos", x, 63, z), 63);
                if (melts != warm) continue;
            }
            return new int[]{x >> 4, z >> 4};
        }
        throw new IllegalStateException("no extension witness for " + seed + "/" + ice + "/" + warm);
    }

    static Map<String, Object> blockPredicates() throws Exception {
        List<Object> ranges = new ArrayList<>();
        for (int i = 0; i < fluids.length; i++) if (fluids[i]) {
            int start = i;
            while (i + 1 < fluids.length && fluids[i + 1]) i++;
            ranges.add(List.of(start, i + 1));
        }
        Map<String, Integer> palette = new TreeMap<>();
        for (String name : List.of("WATER", "LAVA", "PACKED_ICE", "SNOW_BLOCK", "ICE", "STONE", "TUFF", "CAVE_AIR", "VOID_AIR")) palette.put(name, id(name));
        return Map.of("state_count", states.length, "fluid_ranges", ranges, "palette", palette);
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 0) throw new IllegalArgumentException("usage: SurfaceBuilderReference");
        bootstrap();
        Map<String, Object> rules = rules();
        Map<String, int[]> inputs = new LinkedHashMap<>();
        Map<String, Object> profiles = new LinkedHashMap<>();
        for (String profile : List.of("flat", "terraces", "south_up", "north_up", "west_up", "east_up", "ocean", "lava_ocean", "low", "ceiling", "empty", "slabs")) {
            int[] input = profile(profile); inputs.put(profile, input); profiles.put(profile, encodeColumns(input));
        }
        List<Object> samples = new ArrayList<>();
        List<String> possible = new ArrayList<>();
        for (Object holder : (Set<?>) call(environment(0).source, "possibleBiomes")) possible.add(call(call(holder, "key"), "identifier").toString().substring(10));
        Collections.sort(possible);
        for (String b : possible) samples.add(capture("biome/" + b, 0, 0, 0, b, "terraces", "overworld", rules, inputs, null));
        long[] seeds = {0, 1, -1, 846692123413862008L, Long.MIN_VALUE, Long.MAX_VALUE};
        for (long seed : seeds) {
            for (int[] c : new int[][]{{-1, -1}, {1, -2}, {-17, 29}, {1874999, -1875000}})
                samples.add(capture("mixed/" + seed + "/" + c[0] + "/" + c[1], seed, c[0], c[1], "mixed", "slabs", "overworld", rules, inputs, new int[]{47, 84, -19, 120}));
            for (boolean warm : new boolean[]{false, true}) {
                int[] c = witness(seed, true, warm);
                samples.add(capture("iceberg/" + seed + "/" + warm, seed, c[0], c[1], "frozen_ocean", "ocean", "overworld", rules, inputs, new int[]{24, 41, 32, 50}));
                if (seed == 0) samples.add(capture("deep_iceberg/" + warm, seed, c[0], c[1], "deep_frozen_ocean", "ocean", "overworld", rules, inputs, null));
            }
            int[] c = witness(seed, false, false);
            for (String profile : List.of("low", "ocean", "lava_ocean"))
                samples.add(capture("badlands/" + seed + "/" + profile, seed, c[0], c[1], "eroded_badlands", profile, "overworld", rules, inputs, new int[]{48, 59, 63, 71}));
        }
        for (String rule : List.of("depth", "water", "preliminary", "fluid_writes", "temperature"))
            samples.add(capture("scan/" + rule, -1, -1, 1, "mixed", "slabs", rule, rules, inputs, new int[]{-63, 319, 170, -20}));
        for (String rule : List.of("steep", "steep_after_air")) for (String profile : List.of("south_up", "north_up", "west_up", "east_up"))
            samples.add(capture("steep/" + rule + "/" + profile, 0, -1, -1, "plains", profile, rule, rules, inputs, null));
        for (String profile : List.of("empty", "ceiling"))
            samples.add(capture("limits/" + profile, Long.MIN_VALUE, -1, -1, "mixed", profile, "overworld", rules, inputs, null));
        for (long seed : new long[]{0, -1, 846692123413862008L}) for (int[] c : new int[][]{{0, 0}, {-1, -1}, {-126, 187}, {1874999, -1875000}})
            samples.add(capture("overworld/" + seed + "/" + c[0] + "/" + c[1], seed, c[0], c[1], "overworld", "terraces", "overworld", rules, inputs, null));
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println("SURFACE_BUILDER_REFERENCE=" + call(gson, "toJson", Map.of(
            "samples", samples, "profiles", profiles, "rules", rules, "block_predicates", blockPredicates(), "mixed_biomes", MIXED,
            "environment", "26.1 SurfaceSystem.buildSurface; real NOISE-status ProtoChunks, RandomState and NoiseChunk.forChunk; empty Blender; complete 384x16x16 input slabs; native rules/extensions/heightmap updates; clamped quart biome source",
            "postprocessing_order", "native section order, stable insertion order within each section",
            "rule_context_fields", List.of("block_x", "block_y", "block_z", "stone_depth_above", "stone_depth_below", "water_height", "surface_depth", "min_surface_level", "result_state_or_minus_one"),
            "noise_biome_encoding", "palette names indexed by quarts; X fastest, then Z, then Y; local quart X/Z -1..4, absolute quart Y -16..79; sampled from the native overworld source",
            "column_encoding", "columns[z*16+x] indexes palette; each palette row repeats [exclusive_end_y, state_id], starting at -64 and ending at 320")));
    }
}
