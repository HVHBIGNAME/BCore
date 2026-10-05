import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.function.Predicate;

/** Native 26.1 cave features, with an independent bounded fake world.
 * No native feature operation is silently accepted: unknown calls fail capture.
 * Compile with TreeReference and NativeWorldgenRegistries; run on pinned Java 25.
 */
public class CaveFeatureReference extends TreeReference {
    static final List<String> FEATURES = List.of(
        "pointed_dripstone", "dripstone_cluster", "large_dripstone", "rooted_azalea_tree",
        "moss_patch", "moss_patch_bonemeal", "moss_patch_ceiling", "moss_vegetation",
        "lush_caves_clay", "clay_with_dripleaves", "clay_pool_with_dripleaves", "dripleaf",
        "cave_vine", "cave_vine_in_moss", "glow_lichen", "azalea_tree", "spore_blossom");
    static final List<String> TAGS = List.of(
        "air", "dripstone_replaceable", "base_stone_overworld", "moss_replaceable",
        "lush_ground_replaceable", "azalea_root_replaceable", "azalea_grows_on",
        "replaceable_by_trees", "supports_vegetation", "supports_azalea", "supports_small_dripleaf",
        "supports_big_dripleaf", "unstable_bottom_center", "dirt");
    static Object gson, registries, codecRegistries, configured, biomes, factory, heightAccessor, generator;
    static Method blockTag, fluidTag;
    static final Map<Object, Integer> ids = new IdentityHashMap<>();
    static final Map<Object, Predicate<Object>> heights = new IdentityHashMap<>();

    static Object holder(Object registry, String name) throws Exception {
        return ((Optional<?>) call(registry, "get", call(type("resources.Identifier"), "withDefaultNamespace", name))).orElseThrow();
    }

    static Object json(String path) throws Exception {
        try (var stream = CaveFeatureReference.class.getResourceAsStream("/data/minecraft/" + path + ".json")) {
            return call(gson, "fromJson", new String(Objects.requireNonNull(stream, path).readAllBytes(), StandardCharsets.UTF_8), Map.class);
        }
    }

    @SuppressWarnings("unchecked")
    static void bootstrap() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        registries = NativeWorldgenRegistries.load();
        Object builtins = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", field("core.registries.BuiltInRegistries", "REGISTRY"));
        codecRegistries = call(type("core.HolderLookup$Provider"), "create", java.util.stream.Stream.concat(
            (java.util.stream.Stream<?>) call(builtins, "listRegistries"), (java.util.stream.Stream<?>) call(registries, "listRegistries")));
        configured = call(registries, "lookupOrThrow", field("core.registries.Registries", "CONFIGURED_FEATURE"));
        biomes = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        Object settings = call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS"));
        generator = make("world.level.levelgen.NoiseBasedChunkGenerator", make("world.level.biome.FixedBiomeSource", holder(biomes, "lush_caves")), holder(settings, "overworld"));
        air = state("AIR");
        int next = 0;
        for (Object s : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) ids.put(s, next++);
        blockTag = type("world.level.block.state.BlockBehaviour$BlockStateBase").getMethod("is", type("tags.TagKey"));
        fluidTag = type("world.level.material.FluidState").getMethod("is", type("tags.TagKey"));
        for (Object kind : type("world.level.levelgen.Heightmap$Types").getEnumConstants()) {
            heights.put(kind, (Predicate<Object>) call(kind, "isOpaque"));
        }
        heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        Object blockStrategy = call(type("world.level.chunk.Strategy"), "createForBlockStates", field("world.level.block.Block", "BLOCK_STATE_REGISTRY"));
        Object biomeStrategy = call(type("world.level.chunk.Strategy"), "createForBiomes", call(biomes, "asHolderIdMap"));
        factory = make("world.level.chunk.PalettedContainerFactory", blockStrategy, air, null, biomeStrategy, holder(biomes, "lush_caves"), null);
    }

    static int id(Object state) { return Objects.requireNonNull(ids.get(state)); }
    static int[] xyz(Pos p) { return new int[]{p.x(), p.y(), p.z()}; }

    static final class Capture {
        final int floor, ceiling, fillTop;
        final Object rock, fill;
        final boolean reject;
        boolean denyOrigin;
        final Map<Pos, Object> initial = new HashMap<>(), writes = new HashMap<>(), chunks = new HashMap<>();
        final List<int[]> calls = new ArrayList<>(), marks = new ArrayList<>(), implicitMarks = new ArrayList<>(), ticks = new ArrayList<>();
        final Set<String> operations = new TreeSet<>();

        Capture(int floor, int ceiling, Object rock, Object fill, int fillTop, boolean reject) {
            this.floor = floor; this.ceiling = ceiling; this.rock = rock;
            this.fill = fill; this.fillTop = fillTop; this.reject = reject;
        }

        Object get(Pos p) {
            if (p.y() < -64 || p.y() > 319) return air;
            Object background = p.y() <= floor || p.y() >= ceiling ? rock : p.y() <= fillTop ? fill : air;
            return writes.getOrDefault(p, initial.getOrDefault(p, background));
        }

        int height(Object kind, int x, int z) {
            for (int y = 319; y >= -64; y--) if (heights.get(kind).test(get(new Pos(x, y, z)))) return y + 1;
            return -64;
        }

        Object chunk(Pos pos) throws Exception {
            Pos key = new Pos(pos.x() >> 4, 0, pos.z() >> 4);
            if (!chunks.containsKey(key)) {
                Object sections = Array.newInstance(type("world.level.chunk.LevelChunkSection"), 24);
                for (int i = 0; i < 24; i++) Array.set(sections, i, make("world.level.chunk.LevelChunkSection", factory));
                chunks.put(key, make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", key.x(), key.z()),
                    field("world.level.chunk.UpgradeData", "EMPTY"), sections, make("world.ticks.ProtoChunkTicks"),
                    make("world.ticks.ProtoChunkTicks"), heightAccessor, factory, null));
            }
            return chunks.get(key);
        }

        @SuppressWarnings("unchecked")
        Object world() throws Exception {
            return Proxy.newProxyInstance(CaveFeatureReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
                operations.add(m.getName());
                return switch (m.getName()) {
                    case "getMinY" -> -64;
                    case "getMaxY" -> 319;
                    case "getHeight" -> a == null || a.length == 0 ? 384 : height(a[0], (int) a[1], (int) a[2]);
                    case "getHeightmapPos" -> {
                        Pos pos = Pos.from(a[1]);
                        yield make("core.BlockPos", pos.x(), height(a[0], pos.x(), pos.z()), pos.z());
                    }
                    case "getBiome" -> holder(biomes, "lush_caves");
                    case "getBlockState" -> get(Pos.from(a[0]));
                    case "getFluidState" -> call(get(Pos.from(a[0])), "getFluidState");
                    case "isEmptyBlock" -> call(get(Pos.from(a[0])), "isAir");
                    case "isWaterAt" -> fluidTag.invoke(call(get(Pos.from(a[0])), "getFluidState"), field("tags.FluidTags", "WATER"));
                    case "isStateAtPosition" -> ((Predicate<Object>) a[1]).test(get(Pos.from(a[0])));
                    case "isFluidAtPosition" -> ((Predicate<Object>) a[1]).test(call(get(Pos.from(a[0])), "getFluidState"));
                    case "ensureCanWrite" -> !denyOrigin && Pos.from(a[0]).y() >= -64 && Pos.from(a[0]).y() <= 319;
                    case "isOutsideBuildHeight" -> {
                        int y = a[0] instanceof Integer i ? i : Pos.from(a[0]).y();
                        yield y < -64 || y > 319;
                    }
                    case "isClientSide" -> false;
                    case "setBlock" -> {
                        Pos pos = Pos.from(a[0]);
                        boolean accepted = !reject && pos.y() >= -64 && pos.y() <= 319;
                        calls.add(new int[]{pos.x(), pos.y(), pos.z(), id(a[1]), (int) a[2], accepted ? 1 : 0});
                        if (accepted) {
                            writes.put(pos, a[1]);
                            if (((int) a[2] & 16) == 0) {
                                Object mark = call(a[1], "getPostProcessPos", p, a[0]);
                                if (mark != null) implicitMarks.add(xyz(Pos.from(mark)));
                            }
                        }
                        yield accepted;
                    }
                    case "getChunk" -> {
                        if (a.length != 1) throw new UnsupportedOperationException(m.toString());
                        Pos pos = Pos.from(a[0]);
                        marks.add(xyz(pos));
                        yield chunk(pos);
                    }
                    case "scheduleTick" -> {
                        Pos pos = Pos.from(a[0]);
                        boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                        int target = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]) : id(call(a[1], "defaultBlockState"));
                        ticks.add(new int[]{pos.x(), pos.y(), pos.z(), target, (int) a[2], fluid ? 1 : 0});
                        yield null;
                    }
                    default -> throw new UnsupportedOperationException(m.toString());
                };
            });
        }
    }

    static List<int[]> states(Map<Pos, Object> data) {
        return data.keySet().stream().sorted(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z))
            .map(p -> new int[]{p.x(), p.y(), p.z(), id(data.get(p))}).toList();
    }

    static Object decode(Object document) throws Exception {
        Object ops = call(type("resources.RegistryOps"), "create", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), codecRegistries);
        Object value = call(Class.forName("com.google.gson.JsonParser"), "parseString", call(gson, "toJson", document));
        return call(call(field("world.level.levelgen.feature.ConfiguredFeature", "DIRECT_CODEC"), "parse", ops, value), "getOrThrow");
    }

    static Map<String, Object> sample(String name, Object document, long seed, String scenario, Pos origin, Capture capture, int repeats) throws Exception {
        Object feature = decode(document);
        Object random = make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed));
        if (scenario.contains("advanced")) {
            call(random, "nextInt", 17); call(random, "nextFloat"); call(random, "nextInt", 1073741825);
        }
        Object world = capture.world();
        List<Boolean> results = new ArrayList<>();
        List<Integer> drawCounts = new ArrayList<>();
        for (int i = 0; i < repeats; i++) {
            if (scenario.equals("reseed_repeat") && i > 0) call(random, "setSeed", seed + i);
            results.add((boolean) call(feature, "place", world, generator, random, make("core.BlockPos", origin.x(), origin.y(), origin.z())));
            drawCounts.add((int) call(random, "getCount"));
        }
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("name", name); out.put("document", document); out.put("seed", seed); out.put("scenario", scenario);
        out.put("origin", xyz(origin)); out.put("floor", capture.floor); out.put("ceiling", capture.ceiling);
        out.put("rock", id(capture.rock)); out.put("fill", id(capture.fill)); out.put("fill_top", capture.fillTop);
        out.put("reject", capture.reject); out.put("deny_origin", capture.denyOrigin); out.put("initial", states(capture.initial)); out.put("results", results);
        out.put("writes", states(capture.writes)); out.put("write_calls", capture.calls);
        out.put("marks", capture.marks); out.put("implicit_marks", capture.implicitMarks); out.put("ticks", capture.ticks); out.put("operations", capture.operations);
        int nativeMarks = 0;
        for (Object chunk : capture.chunks.values()) for (Object list : (Object[]) call(chunk, "getPostProcessing")) {
            if (list != null) nativeMarks += ((List<?>) list).size();
        }
        if (nativeMarks != capture.marks.size()) throw new IllegalStateException("getChunk mark trace differs from actual ProtoChunk marks");
        out.put("native_mark_count", nativeMarks);
        out.put("draw_counts", drawCounts); out.put("rng_count", call(random, "getCount")); out.put("next_i64", call(random, "nextLong"));
        return out;
    }

    @SuppressWarnings("unchecked")
    static Object featureSamples() throws Exception {
        List<Object> samples = new ArrayList<>();
        Map<String, Object> documents = new TreeMap<>();
        for (String name : FEATURES) documents.put(name, json("worldgen/configured_feature/" + name));
        Map<?, ?> pointedConfig = (Map<?, ?>) ((Map<?, ?>) documents.get("pointed_dripstone")).get("config");
        Map<String, Object> pointed = (Map<String, Object>) ((Map<?, ?>) ((List<?>) pointedConfig.get("features")).get(0)).get("feature");
        for (String name : List.of("pointed_dripstone", "dripstone_cluster", "large_dripstone", "cave_vine", "cave_vine_in_moss", "glow_lichen",
                "moss_patch", "moss_patch_bonemeal", "moss_patch_ceiling", "moss_vegetation", "clay_with_dripleaves", "clay_pool_with_dripleaves", "lush_caves_clay", "dripleaf", "spore_blossom")) {
            Object document = name.equals("pointed_dripstone") ? pointed : documents.get(name);
            for (long seed : new long[]{0, 1, 17, 42, 1234}) {
                boolean ceiling = name.contains("ceiling") || name.contains("vine") || name.equals("spore_blossom") || name.equals("glow_lichen");
                int floor = 20, roof = name.equals("large_dripstone") ? 65 : 30;
                int y = ceiling ? roof - 1 : name.equals("pointed_dripstone") || name.contains("moss") || name.contains("clay") || name.equals("dripleaf") ? floor + 1 : (floor + roof) / 2;
                Capture capture = new Capture(floor, roof, state("STONE"), air, floor, false);
                samples.add(sample(name, document, seed, "stone", new Pos(15, y, -17), capture, 1));
            }
        }
        for (String name : List.of("pointed_dripstone", "dripstone_cluster", "large_dripstone", "cave_vine", "glow_lichen", "moss_patch_bonemeal", "clay_pool_with_dripleaves")) {
            Object document = name.equals("pointed_dripstone") ? pointed : documents.get(name);
            for (String scenario : List.of("water", "flowing_water", "lava", "bedrock", "deepslate", "no_floor", "no_ceiling", "reject_writes", "denied_origin", "advanced_repeat", "bottom", "top")) {
                int floor = scenario.equals("bottom") ? -64 : scenario.equals("top") ? 309 : 20;
                int roof = name.equals("large_dripstone") ? floor + 44 : floor + 10;
                Object rock = state(scenario.equals("bedrock") ? "BEDROCK" : scenario.equals("deepslate") ? "DEEPSLATE" : "STONE");
                Object fill = scenario.equals("water") ? state("WATER") : scenario.equals("flowing_water") ? call(state("WATER"), "setValue", field("world.level.block.LiquidBlock", "LEVEL"), 1) : scenario.equals("lava") ? state("LAVA") : air;
                int y = name.equals("pointed_dripstone") || name.contains("moss") || name.contains("clay") ? floor + 1 : name.contains("vine") || name.equals("glow_lichen") ? roof - 1 : floor + 5;
                Capture capture = new Capture(scenario.equals("no_floor") ? -1000 : floor, scenario.equals("no_ceiling") ? 1000 : roof, rock, fill, roof - 1, scenario.equals("reject_writes"));
                capture.denyOrigin = scenario.equals("denied_origin");
                samples.add(sample(name, document, 42, scenario, new Pos(-16, y, 31), capture, scenario.contains("repeat") ? 3 : 1));
            }
        }
        addRootSamples(samples, documents);
        addEdgeSamples(samples, documents, pointed);
        addProviderSamples(samples, documents);
        return Map.of("configs", documents, "samples", samples);
    }

    @SuppressWarnings("unchecked")
    static Map<String, Object> copyDocument(Object document) throws Exception {
        return (Map<String, Object>) call(gson, "fromJson", call(gson, "toJson", document), Map.class);
    }

    static Map<String, Object> simple(String block, boolean tick) {
        return Map.of("type", "minecraft:simple_block", "config", Map.of("to_place", Map.of("type", "minecraft:simple_state_provider", "state", Map.of("Name", "minecraft:" + block)), "schedule_tick", tick));
    }

    @SuppressWarnings("unchecked")
    static void addRootSamples(List<Object> samples, Map<String, Object> documents) throws Exception {
        for (String scenario : List.of("rooted_stone", "rooted_deepslate", "rooted_bedrock", "rooted_water", "rooted_lava", "rooted_reject", "rooted_obstructed", "rooted_no_surface", "rooted_bottom", "rooted_top", "rooted_child_false", "rooted_already_dirt")) {
            Map<String, Object> document = copyDocument(documents.get("rooted_azalea_tree"));
            Map<String, Object> config = (Map<String, Object>) document.get("config");
            // A real native SimpleBlockFeature is the controlled child. This
            // tests RootSystemFeature's control flow without replaying a tree.
            config.put("feature", Map.of("feature", simple(scenario.equals("rooted_child_false") ? "spore_blossom" : "moss_block", false), "placement", List.of()));
            int y = scenario.equals("rooted_bottom") ? -63 : scenario.equals("rooted_top") ? 316 : 21;
            Pos origin = new Pos(-16, y, 31);
            Capture capture = new Capture(y - 4, 1000, state("STONE"), air, y - 4, scenario.equals("rooted_reject"));
            String rock = scenario.equals("rooted_deepslate") ? "DEEPSLATE" : scenario.equals("rooted_bedrock") ? "BEDROCK" : scenario.equals("rooted_already_dirt") ? "ROOTED_DIRT" : "STONE";
            if (!scenario.equals("rooted_no_surface")) {
                for (int x = -4; x <= 4; x++) for (int z = -4; z <= 4; z++) for (int dy = 1; dy <= 7; dy++) {
                    capture.initial.put(new Pos(origin.x() + x, y + dy, origin.z() + z), state(dy == 7 ? "DIRT" : rock));
                }
            }
            if (scenario.equals("rooted_water") || scenario.equals("rooted_lava")) capture.initial.put(new Pos(origin.x(), y + 9, origin.z()), state(scenario.equals("rooted_water") ? "WATER" : "LAVA"));
            if (scenario.equals("rooted_obstructed")) capture.initial.put(new Pos(origin.x(), y + 10, origin.z()), state("STONE"));
            samples.add(sample("root_system", document, 17, scenario, origin, capture, 1));
        }
        // Unmodified production config, with no valid surface: native success
        // is independent of child success and consumes no randomness here.
        samples.add(sample("rooted_azalea_tree", documents.get("rooted_azalea_tree"), 42, "no_surface", new Pos(0, 21, 0), new Capture(20, 30, state("STONE"), air, 20, false), 1));
    }

    @SuppressWarnings("unchecked")
    static void addEdgeSamples(List<Object> samples, Map<String, Object> documents, Map<String, Object> pointed) throws Exception {
        for (String name : List.of("pointed_dripstone", "dripstone_cluster", "large_dripstone", "glow_lichen", "moss_patch_bonemeal", "cave_vine")) {
            Object document = name.equals("pointed_dripstone") ? pointed : documents.get(name);
            for (int y : new int[]{-65, -64, 319, 320}) {
                Capture capture = new Capture(y - 1, y + 1, state("STONE"), air, y - 1, false);
                samples.add(sample(name, document, 1, "build_boundary", new Pos(-17, y, 16), capture, 1));
            }
        }
        for (String soil : List.of("STONE", "DEEPSLATE", "BEDROCK", "MOSS_BLOCK", "CLAY", "WATER", "LAVA", "OAK_SLAB")) {
            Capture capture = new Capture(20, 1000, state(soil), air, 20, false);
            samples.add(sample("moss_vegetation", documents.get("moss_vegetation"), 1, "soil_" + soil, new Pos(0, 21, 0), capture, 3));
        }
        for (String scenario : List.of("ceiling", "wall", "wrap_corner", "existing_face", "flowing_source", "source_water", "rejected_spread", "non_supporting_whitelist")) {
            Map<String, Object> document = copyDocument(documents.get("glow_lichen"));
            Map<String, Object> config = (Map<String, Object>) document.get("config");
            config.put("chance_of_spreading", 1.0);
            config.put("can_place_on_floor", true);
            if (scenario.equals("non_supporting_whitelist")) config.put("can_be_placed_on", List.of("minecraft:oak_slab", "minecraft:stone"));
            Capture capture = new Capture(-1000, 1000, state("STONE"), air, -1000, scenario.equals("rejected_spread"));
            Pos p = new Pos(15, 20, -17);
            if (scenario.equals("ceiling") || scenario.equals("rejected_spread")) {
                for (int x = -1; x <= 1; x++) for (int z = -1; z <= 1; z++) capture.initial.put(new Pos(p.x() + x, p.y() + 1, p.z() + z), state("STONE"));
            } else capture.initial.put(new Pos(p.x() + 1, p.y(), p.z()), state("STONE"));
            if (scenario.equals("existing_face")) capture.initial.put(new Pos(p.x(), p.y() + 1, p.z()), call(state("GLOW_LICHEN"), "setValue", field("world.level.block.state.properties.BlockStateProperties", "EAST"), true));
            if (scenario.equals("source_water")) capture.initial.put(p, state("WATER"));
            if (scenario.equals("flowing_source")) capture.initial.put(p, call(state("WATER"), "setValue", field("world.level.block.LiquidBlock", "LEVEL"), 1));
            if (scenario.equals("non_supporting_whitelist")) capture.initial.put(new Pos(p.x(), p.y() + 1, p.z()), state("OAK_SLAB"));
            samples.add(sample("glow_lichen", document, 42, scenario, p, capture, 1));
        }
        for (String name : List.of("moss_patch_bonemeal", "clay_pool_with_dripleaves", "clay_with_dripleaves")) {
            for (String scenario : List.of("uneven", "existing_ground", "pool_leak", "pool_water", "stone_cutoff")) {
                Capture capture = new Capture(20, 30, state("STONE"), air, 20, false);
                if (scenario.equals("uneven")) for (int x = -4; x <= 4; x++) for (int z = -4; z <= 4; z++) {
                    for (int y = 21; y < 21 + Math.floorMod(x + z, 3); y++) capture.initial.put(new Pos(x, y, z), state("STONE"));
                }
                if (scenario.equals("existing_ground")) for (int x = -4; x <= 4; x++) for (int z = -4; z <= 4; z++) capture.initial.put(new Pos(x, 20, z), state(name.contains("moss") ? "MOSS_BLOCK" : "CLAY"));
                if (scenario.equals("pool_leak")) capture.initial.put(new Pos(0, 19, 0), air);
                if (scenario.equals("pool_water")) capture.initial.put(new Pos(0, 21, 0), state("WATER"));
                if (scenario.equals("stone_cutoff")) capture.initial.put(new Pos(0, 19, 0), state("BEDROCK"));
                samples.add(sample(name, documents.get(name), 42, scenario, new Pos(0, 21, 0), capture, 2));
            }
        }
        samples.add(sample("simple_block", simple("moss_block", true), 0, "scheduled_tick", new Pos(0, 21, 0), new Capture(20, 30, state("STONE"), air, 20, false), 1));
        samples.add(sample("simple_block", simple("moss_block", true), 0, "scheduled_tick_rejected_write", new Pos(0, 21, 0), new Capture(20, 30, state("STONE"), air, 20, true), 1));
        for (int gap : new int[]{1, 2, 3, 6}) {
            Map<String, Object> document = copyDocument(documents.get("dripstone_cluster"));
            Map<String, Object> config = (Map<String, Object>) document.get("config");
            config.put("density", 2.0); config.put("height", 12); config.put("radius", 2); config.put("wetness", 0);
            config.put("chance_of_dripstone_column_at_max_distance_from_center", 1);
            samples.add(sample("dripstone_cluster", document, 17, "merge_gap_" + gap, new Pos(0, 21, 0), new Capture(20, 21 + gap, state("DEEPSLATE"), air, 20, false), 2));
        }
        samples.add(sample("dripstone_cluster", documents.get("dripstone_cluster"), 17, "reseed_repeat", new Pos(15, 25, -17), new Capture(20, 30, state("STONE"), air, 20, false), 3));
        Map<String, Object> mossWater = copyDocument(documents.get("moss_patch_bonemeal"));
        mossWater.put("type", "minecraft:waterlogged_vegetation_patch");
        samples.add(sample("moss_water", mossWater, 42, "moss_water", new Pos(0, 21, 0), new Capture(20, 30, state("DEEPSLATE"), air, 20, false), 3));
    }

    @SuppressWarnings("unchecked")
    static void addProviderSamples(List<Object> samples, Map<String, Object> documents) throws Exception {
        List<Object> intProviders = List.of(
            Map.of("type", "minecraft:constant", "value", 3),
            Map.of("type", "minecraft:uniform", "min_inclusive", 3, "max_inclusive", 3),
            Map.of("type", "minecraft:biased_to_bottom", "min_inclusive", 1, "max_inclusive", 8),
            Map.of("type", "minecraft:clamped_normal", "mean", 4.5, "deviation", 2.5, "min_inclusive", 1, "max_inclusive", 9),
            Map.of("type", "minecraft:clamped", "source", Map.of("type", "minecraft:uniform", "min_inclusive", -2, "max_inclusive", 10), "min_inclusive", 2, "max_inclusive", 6),
            Map.of("type", "minecraft:weighted_list", "distribution", List.of(Map.of("data", 4, "weight", 0), Map.of("data", 2, "weight", 1)))
        );
        for (int i = 0; i < intProviders.size(); i++) {
            Map<String, Object> document = copyDocument(documents.get("dripstone_cluster"));
            Map<String, Object> config = (Map<String, Object>) document.get("config");
            config.put("height", intProviders.get(i)); config.put("radius", 2);
            samples.add(sample("dripstone_cluster", document, 42, "int_provider_" + i, new Pos(0, 25, 0), new Capture(20, 30, state("STONE"), air, 20, false), 2));
        }
        for (int i = 0; i < 3; i++) {
            Map<String, Object> document = copyDocument(documents.get("large_dripstone"));
            Map<String, Object> config = (Map<String, Object>) document.get("config");
            config.put("column_radius", 5);
            config.put("height_scale", i == 0 ? Map.of("type", "minecraft:constant", "value", 1.0) : i == 1 ? Map.of("type", "minecraft:clamped_normal", "mean", 1.1, "deviation", 0.6, "min", 0.4, "max", 2.0) : Map.of("type", "minecraft:trapezoid", "min", 0.4, "max", 2.0, "plateau", 0.2));
            config.put("min_radius_for_wind", 0); config.put("min_bluntness_for_wind", 0);
            samples.add(sample("large_dripstone", document, 17, "float_provider_" + i, new Pos(0, 30, 0), new Capture(20, 50, state("STONE"), air, 20, false), 1));
        }
    }

    static Object blockData() throws Exception {
        Map<String, Object> configs = new TreeMap<>(), tags = new TreeMap<>(), schema = new TreeMap<>(), blockIds = new TreeMap<>(), tagSources = new TreeMap<>();
        for (String name : FEATURES) configs.put(name, json("worldgen/configured_feature/" + name));
        for (String tag : TAGS) {
            Object key = field("tags.BlockTags", tag.toUpperCase(Locale.ROOT));
            String location = call(key, "location").toString().replace("minecraft:", "");
            tagSources.put(tag, Map.of("location", location, "document", json("tags/block/" + location)));
            List<Integer> matches = new ArrayList<>();
            for (Object s : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) if ((boolean) blockTag.invoke(s, key)) matches.add(id(s));
            tags.put(tag, matches);
            tags.put(location, matches);
        }
        Object zero = make("core.BlockPos", 0, 0, 0);
        Object empty = Proxy.newProxyInstance(CaveFeatureReference.class.getClassLoader(), new Class<?>[]{type("world.level.BlockGetter")},
            (p, m, a) -> { throw new UnsupportedOperationException("Contextual shape: " + m); });
        Field cache = type("world.level.block.state.BlockBehaviour$BlockStateBase").getDeclaredField("cache"); cache.setAccessible(true);
        Object blockRegistry = field("core.registries.BuiltInRegistries", "BLOCK");
        for (Object block : (Iterable<?>) blockRegistry) {
            blockIds.put(call(blockRegistry, "getKey", block).toString(), id(call(block, "defaultBlockState")));
        }
        List<int[]> ranges = new ArrayList<>();
        int[] last = null;
        for (Object s : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            Object fluid = call(s, "getFluidState");
            int flags = (boolean) call(s, "isAir") ? 1 : 0;
            if (call(s, "getBlock") == field("world.level.block.Blocks", "WATER")) flags |= 2;
            if (call(s, "getBlock") == field("world.level.block.Blocks", "LAVA")) flags |= 4;
            if ((boolean) fluidTag.invoke(fluid, field("tags.FluidTags", "WATER"))) flags |= 8;
            if ((boolean) fluidTag.invoke(fluid, field("tags.FluidTags", "LAVA"))) flags |= 16;
            if ((boolean) call(fluid, "isSource") && (flags & 8) != 0) flags |= 32;
            if ((boolean) call(s, "isSolid")) flags |= 64;
            if ((boolean) call(s, "canBeReplaced")) flags |= 128;
            int faces = 0, attach = 0, center = 0;
            if (cache.get(s) == null) { faces = -1; attach = -1; center = -1; }
            else for (Object direction : type("core.Direction").getEnumConstants()) {
                if ((boolean) call(s, "isFaceSturdy", empty, zero, direction)) faces |= 1 << ((Enum<?>) direction).ordinal();
                if ((boolean) call(type("world.level.block.MultifaceBlock"), "canAttachTo", empty, call(direction, "getOpposite"), zero, s)) attach |= 1 << ((Enum<?>) direction).ordinal();
                if ((boolean) call(s, "isFaceSturdy", empty, zero, direction, field("world.level.block.SupportType", "CENTER"))) center |= 1 << ((Enum<?>) direction).ordinal();
            }
            int block = id(call(call(s, "getBlock"), "defaultBlockState"));
            if (last != null && last[1] == id(s) && last[2] == flags && last[3] == faces && last[4] == block && last[5] == attach && last[6] == center) last[1]++;
            else { last = new int[]{id(s), id(s) + 1, flags, faces, block, attach, center}; ranges.add(last); }
        }
        for (String name : List.of("air", "stone", "deepslate", "water", "lava", "bedrock", "dripstone_block", "pointed_dripstone", "rooted_dirt", "hanging_roots", "moss_block", "moss_carpet", "clay", "short_grass", "tall_grass", "azalea", "flowering_azalea", "cave_vines", "cave_vines_plant", "glow_lichen", "small_dripleaf", "big_dripleaf", "big_dripleaf_stem", "spore_blossom")) {
            Object block = field("world.level.block.Blocks", name.toUpperCase(Locale.ROOT));
            List<Object> states = new ArrayList<>();
            for (Object s : (Iterable<?>) call(call(block, "getStateDefinition"), "getPossibleStates")) {
                Map<String, String> properties = new TreeMap<>();
                for (Object prop : (Iterable<?>) call(s, "getProperties")) {
                    properties.put((String) call(prop, "getName"), (String) call(prop, "getName", call(s, "getValue", prop)));
                }
                states.add(Map.of("id", id(s), "properties", properties));
            }
            schema.put(name, Map.of("default", id(call(block, "defaultBlockState")), "states", states));
        }
        return Map.of("configs", configs, "tags", tags, "tag_sources", tagSources, "blocks", schema, "block_ids", blockIds, "state_ranges", ranges, "state_count", ids.size(), "position_sets", positionSets());
    }

    static List<Object> positionSets() throws Exception {
        List<Object> samples = new ArrayList<>();
        for (int sign : new int[]{1, -1}) for (int size : new int[]{8, 9, 10, 11, 24, 48, 49, 97}) {
            List<int[]> input = new ArrayList<>();
            for (int i = 0; i < size; i++) {
                int[] p = new int[]{sign * i * 64, 20, 0};
                input.add(p);
                if (i % 5 == 0) input.add(p);
            }
            if (sign < 0) Collections.rotate(input, input.size() / 3);
            Set<Object> positions = new HashSet<>();
            for (int[] p : input) positions.add(make("core.BlockPos", p[0], p[1], p[2]));
            List<int[]> order = new ArrayList<>();
            for (Object p : positions) order.add(xyz(Pos.from(p)));
            samples.add(Map.of("name", sign + "_" + size, "input", input, "order", order));
        }
        return samples;
    }

    public static void main(String[] args) throws Exception {
        bootstrap();
        Object result = args[0].equals("blocks") ? blockData() : featureSamples();
        System.out.println("CAVE_FEATURE_REFERENCE=" + call(gson, "toJson", result));
    }
}
