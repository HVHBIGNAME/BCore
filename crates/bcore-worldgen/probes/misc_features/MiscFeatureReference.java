import java.lang.reflect.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.Predicate;

/** Independent worlds; executes ConfiguredFeature.place from the unmodified 26.1 JAR. */
public class MiscFeatureReference extends BaseFeatureReference {
    static Scene scene;
    static boolean captureGaussian;
    static int sampleSeaLevel = 63;
    static final Map<String, Object> custom = new TreeMap<>();

    static final class Scene {
        final Terrain terrain;
        final Pos origin;
        final Map<Pos, Object> writes = new HashMap<>();
        final List<int[]> prefix = new ArrayList<>(), ticks = new ArrayList<>(), observations = new ArrayList<>();
        final MessageDigest writeHash;
        int count, seaLevel = 63, splitY = -64;
        String lowerBiome = "minecraft:plains";
        boolean freeze, observing;
        Object lastBiome;
        int lastBiomeId;
        Pos lastBiomePos;

        Scene(Terrain terrain, Pos origin) throws Exception {
            this.terrain = terrain;
            this.origin = origin;
            writeHash = MessageDigest.getInstance("MD5");
        }
        Object block(Pos pos) throws Exception { return writes.containsKey(pos) ? writes.get(pos) : terrain.initial(pos); }
        int height(Object kind, int x, int z) throws Exception {
            for (int y = 319; y >= -64; y--)
                if (heightPredicates.get(kind).test(block(new Pos(x, y, z)))) return y + 1;
            return -64;
        }
        boolean put(Pos pos, Object state, int flags) {
            boolean accepted = pos.y() >= -64 && pos.y() <= 319 && !terrain.writePolicy.equals("reject")
                && (!terrain.writePolicy.equals("checker") || ((pos.x() + pos.z()) & 1) == 0)
                && (!terrain.writePolicy.equals("source_chunk") || (pos.x() >> 4) == (origin.x() >> 4) && (pos.z() >> 4) == (origin.z() >> 4));
            int[] row = {pos.x(), pos.y(), pos.z(), id(state), flags, accepted ? 1 : 0};
            digest(writeHash, row);
            count++;
            if (prefix.size() < 32) prefix.add(row);
            if (accepted) writes.put(pos, state);
            return accepted;
        }
        Object biome(Pos pos) throws Exception {
            Object result = holder(biomeRegistry, pos.y() < splitY ? lowerBiome : terrain.biome);
            if (!observing) {
                lastBiome = call(result, "value");
                lastBiomeId = (int) call(call(biomeRegistry, "asHolderIdMap"), "getId", result);
                lastBiomePos = pos;
            }
            return result;
        }
        void observeEnvironment(Object world) throws Exception {
            if (!freeze || observing || lastBiomePos == null) return;
            String method = null;
            for (StackTraceElement frame : Thread.currentThread().getStackTrace()) {
                if (frame.getClassName().equals("net.minecraft.world.level.biome.Biome")
                        && (frame.getMethodName().equals("shouldFreeze") || frame.getMethodName().equals("shouldSnow"))) {
                    method = frame.getMethodName();
                    break;
                }
            }
            if (method == null) return;
            // Re-evaluate the actual pure native callback on the same world at
            // its entry (getSeaLevel). The feature itself is not replaced or replayed.
            observing = true;
            try {
                boolean ice = method.equals("shouldFreeze");
                Pos pos = ice ? new Pos(lastBiomePos.x(), lastBiomePos.y() - 1, lastBiomePos.z()) : lastBiomePos;
                boolean result = (boolean) (ice
                    ? call(lastBiome, method, world, nativePos(pos), false)
                    : call(lastBiome, method, world, nativePos(pos)));
                observations.add(new int[]{ice ? 0 : 1, lastBiomeId, pos.x(), pos.y(), pos.z(), 0,
                    result ? 1 : 0, id(block(pos)), id(block(new Pos(pos.x(), pos.y() - 1, pos.z())))});
            } finally { observing = false; }
        }
        Map<String, Object> snapshot() throws Exception {
            List<Pos> sorted = new ArrayList<>(writes.keySet());
            sorted.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
            MessageDigest changedHash = MessageDigest.getInstance("MD5");
            int changed = 0;
            for (Pos pos : sorted) if (writes.get(pos) != terrain.initial(pos)) {
                digest(changedHash, pos.x(), pos.y(), pos.z(), id(writes.get(pos)));
                changed++;
            }
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("write_count", count);
            result.put("write_prefix", prefix);
            result.put("write_md5", HexFormat.of().formatHex(writeHash.digest()));
            result.put("changed_blocks", changed);
            result.put("blocks_md5", HexFormat.of().formatHex(changedHash.digest()));
            result.put("marks", List.of());
            result.put("ticks", ticks);
            result.put("environment", observations);
            result.put("split_y", splitY);
            result.put("lower_biome", lowerBiome);
            result.put("sea_level", seaLevel);
            return result;
        }
    }

    @SuppressWarnings("unchecked")
    static Object world() throws Exception {
        return Proxy.newProxyInstance(MiscFeatureReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> switch (m.getName()) {
            case "getBlockState" -> scene.block(position(a[0]));
            case "getFluidState" -> call(scene.block(position(a[0])), "getFluidState");
            case "isEmptyBlock" -> call(scene.block(position(a[0])), "isAir");
            case "isWaterAt" -> type("world.level.material.FluidState").getMethod("is", type("tags.TagKey"))
                .invoke(call(scene.block(position(a[0])), "getFluidState"), field("tags.FluidTags", "WATER"));
            case "isStateAtPosition" -> ((Predicate<Object>) a[1]).test(scene.block(position(a[0])));
            case "isFluidAtPosition" -> ((Predicate<Object>) a[1]).test(call(scene.block(position(a[0])), "getFluidState"));
            case "getHeight" -> a == null || a.length == 0 ? 384 : scene.height(a[0], (int) a[1], (int) a[2]);
            case "getHeightmapPos" -> { Pos pos = position(a[1]); yield make("core.BlockPos", pos.x(), scene.height(a[0], pos.x(), pos.z()), pos.z()); }
            case "getMinY" -> -64;
            case "getMaxY" -> 319;
            case "isOutsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : position(a[0]).y(); yield y < -64 || y > 319; }
            case "isInsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : position(a[0]).y(); yield y >= -64 && y <= 319; }
            case "getRawBrightness", "getBrightness" -> scene.terrain.light;
            case "getSeaLevel" -> { scene.observeEnvironment(p); yield scene.seaLevel; }
            case "getBiome" -> scene.biome(position(a[0]));
            case "ensureCanWrite" -> !scene.terrain.denyOrigin;
            case "setBlock" -> scene.put(position(a[0]), a[1], (int) a[2]);
            case "scheduleTick" -> {
                Pos pos = position(a[0]);
                boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                int target = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]) : id(call(a[1], "defaultBlockState"));
                scene.ticks.add(new int[]{pos.x(), pos.y(), pos.z(), target, (int) a[2], fluid ? 1 : 0});
                yield null;
            }
            case "getRandom" -> worldRandom;
            case "getBlockEntity" -> null;
            default -> throw new UnsupportedOperationException(m.toString());
        });
    }

    static Map<String, Object> metadata() throws Exception {
        Object empty = emptyWorld(), pos = make("core.BlockPos", 0, 64, 0);
        Object[] directions = type("core.Direction").getEnumConstants();
        Object up = field("core.Direction", "UP");
        List<int[]> ranges = new ArrayList<>();
        int[] previous = null;
        for (Object state : statesById) {
            int faces = 0;
            for (int i = 0; i < directions.length; i++) {
                Object shape = call(state, "getFaceOcclusionShape", directions[i]);
                if ((boolean) call(type("world.level.block.Block"), "isShapeFullBlock", shape)) faces |= 1 << i;
            }
            Object collision = call(state, "getCollisionShape", empty, pos);
            int full = (boolean) call(type("world.level.block.Block"), "isFaceFull", collision, up) ? 1 : 0;
            int nonempty = (boolean) call(call(collision, "getFaceShape", up), "isEmpty") ? 0 : 1;
            int[] row = {id(state), id(state) + 1, faces, full, nonempty};
            if (previous != null && Arrays.equals(Arrays.copyOfRange(previous, 2, 5), Arrays.copyOfRange(row, 2, 5))) previous[1]++;
            else { ranges.add(row); previous = row; }
        }
        Map<String, Object> tags = new TreeMap<>();
        for (String tag : List.of("CORAL_BLOCKS", "CORALS", "WALL_CORALS")) {
            Object values = ((Optional<?>) call(field("core.registries.BuiltInRegistries", "BLOCK"), "get", field("tags.BlockTags", tag))).orElseThrow();
            List<Integer> ids = new ArrayList<>();
            for (Object h : (Iterable<?>) values) ids.add(id(call(call(h, "value"), "defaultBlockState")));
            tags.put(tag.toLowerCase(Locale.ROOT), ids);
        }
        return Map.of("state_count", statesById.size(), "shape_ranges", ranges, "ordered_tags", tags);
    }

    static Object document(String name) {
        return custom.containsKey(name) ? custom.get(name) : Objects.requireNonNull(documents.get("worldgen/configured_feature/" + name + ".json"), name);
    }

    static Map<String, Object> sample(String name, String scenario, long seed, Pos origin, Terrain terrain, int splitY, String lowerBiome) throws Exception {
        scene = new Scene(terrain, origin);
        scene.seaLevel = sampleSeaLevel;
        scene.splitY = splitY;
        scene.lowerBiome = lowerBiome;
        scene.freeze = name.equals("freeze_top_layer");
        Object doc = document(name);
        Object feature = custom.containsKey(name) ? decode("world.level.levelgen.feature.ConfiguredFeature", doc) : call(holder(configuredRegistry, name), "value");
        Object random = random(seed);
        worldRandom = random(987654321L);
        boolean result;
        try { result = (boolean) call(feature, "place", world(), generator, random, nativePos(origin)); }
        catch (Exception e) { throw new IllegalStateException(name + "/" + scenario + " seed=" + seed + " origin=" + origin, e); }
        Map<String, Object> sample = new LinkedHashMap<>(scene.snapshot());
        sample.putAll(Map.of("name", name, "scenario", scenario, "seed", seed, "origin", xyz(origin), "terrain", terrain.json(),
            "result", result, "next_i64", call(random, "nextLong"), "world_next_i64", call(worldRandom, "nextLong")));
        if (captureGaussian) sample.put("next_gaussian_bits", Long.toUnsignedString(Double.doubleToRawLongBits((double) call(random, "nextGaussian"))));
        return sample;
    }

    static void add(List<Object> samples, String name, String scenario, long seed, Pos origin, Terrain terrain) throws Exception {
        samples.add(sample(name, scenario, seed, origin, terrain, -64, terrain.biome));
    }

    static Object configured(String type, Map<String, Object> config) throws Exception {
        return parse((String) call(gson, "toJson", Map.of("type", "minecraft:" + type, "config", config)));
    }

    static Map<String, Object> priority() throws Exception {
        List<Object> samples = new ArrayList<>();
        Pos origin = new Pos(-17, 64, 16);
        custom.put("bamboo_podzol", configured("bamboo", Map.of("probability", 1.0)));
        for (String name : List.of("bamboo_no_podzol", "bamboo_some_podzol", "bamboo_podzol")) {
            for (long seed : new long[]{0, 1, 17, 42, -1, 846692123413862008L}) {
                for (String soil : List.of("GRASS_BLOCK", "SAND", "STONE", "BAMBOO", "BAMBOO_SAPLING"))
                    add(samples, name, "soil_" + soil, seed, origin, new Terrain().layer(-64, 63, soil));
                for (String policy : List.of("reject", "checker", "source_chunk")) {
                    Terrain t = terrain("flat"); t.writePolicy = policy;
                    add(samples, name, "writes_" + policy, seed, origin, t);
                }
            }
            for (int clearance : new int[]{0, 1, 2, 3, 4, 8}) {
                Terrain t = terrain("flat").at(new Pos(origin.x(), origin.y() + clearance, origin.z()), "STONE");
                add(samples, name, "clearance_" + clearance, 17, origin, t);
            }
            add(samples, name, "ceiling", 42, new Pos(15, 319, -16), terrain("ceiling"));
            Terrain denied = terrain("flat"); denied.denyOrigin = true;
            add(samples, name, "origin_denied", 17, origin, denied);
        }
        for (String block : List.of("STONE", "GLASS", "OAK_LEAVES", "OAK_SLAB", "OAK_STAIRS", "OAK_FENCE", "HONEY_BLOCK", "SNOW", "WATER", "AIR")) {
            Object[] directions = type("core.Direction").getEnumConstants();
            for (Object direction : directions) {
                Pos neighbor = position(call(nativePos(origin), "relative", direction));
                Terrain t = terrain("air").at(neighbor, block);
                add(samples, "vines", block + "_" + direction, 17, origin, t);
            }
        }
        for (String policy : List.of("all", "reject", "checker", "source_chunk")) {
            Terrain t = terrain("air"); t.writePolicy = policy;
            for (Object direction : type("core.Direction").getEnumConstants()) t.at(position(call(nativePos(origin), "relative", direction)), "STONE");
            add(samples, "vines", "all_faces_" + policy, 0, origin, t);
        }
        Terrain denied = terrain("air").at(new Pos(-17, 65, 16), "STONE"); denied.denyOrigin = true;
        add(samples, "vines", "origin_denied", 0, origin, denied);
        add(samples, "vines", "occupied", 0, origin, terrain("stone"));

        for (int range : new int[]{0, 1, 2, 4, 5, 6, 8}) {
            String name = "magma_range_" + range;
            custom.put(name, configured("underwater_magma", Map.of("floor_search_range", range,
                "placement_radius_around_floor", 2, "placement_probability_per_valid_position", 1.0)));
            add(samples, name, "floor_distance_5", 17, origin, terrain("ocean"));
        }
        custom.put("magma_dense", configured("underwater_magma", Map.of("floor_search_range", 16,
            "placement_radius_around_floor", 2, "placement_probability_per_valid_position", 1.0)));
        custom.put("magma_never", configured("underwater_magma", Map.of("floor_search_range", 16,
            "placement_radius_around_floor", 2, "placement_probability_per_valid_position", 0.0)));
        for (String name : List.of("underwater_magma", "magma_dense", "magma_never")) {
            for (long seed : new long[]{0, 1, 17, 42, -1}) for (String policy : List.of("all", "reject", "checker", "source_chunk")) {
                Terrain t = terrain("ocean"); t.writePolicy = policy;
                add(samples, name, "ocean_" + policy, seed, origin, t);
            }
            for (String soil : List.of("GLASS", "OAK_LEAVES", "OAK_SLAB", "OAK_STAIRS", "OAK_FENCE", "SNOW", "SAND", "STONE"))
                add(samples, name, "floor_" + soil, 17, origin, new Terrain().layer(-64, 59, soil).layer(60, 67, "WATER"));
            add(samples, name, "dry", 17, origin, terrain("flat"));
            Terrain t = terrain("ocean"); t.denyOrigin = true;
            add(samples, name, "origin_denied", 17, origin, t);
        }
        for (String biome : List.of("snowy_plains", "plains", "desert", "frozen_ocean", "snowy_taiga")) {
            for (String ground : List.of("flat", "ocean", "ceiling")) for (int light : new int[]{0, 9, 10}) {
                Terrain t = terrain(ground); t.biome = "minecraft:" + biome; t.light = light;
                add(samples, "freeze_top_layer", biome + "_" + ground + "_light_" + light, 42, origin, t);
            }
        }
        for (String policy : List.of("reject", "checker", "source_chunk")) for (String ground : List.of("flat", "ocean")) {
            Terrain t = terrain(ground); t.biome = "minecraft:snowy_plains"; t.light = 0; t.writePolicy = policy;
            add(samples, "freeze_top_layer", ground + "_" + policy, 0, origin, t);
        }
        Terrain split = new Terrain().layer(-64, 62, "STONE").layer(63, 63, "WATER");
        split.biome = "minecraft:snowy_plains"; split.light = 0;
        samples.add(sample("freeze_top_layer", "vertical_biome_boundary", 17, origin, split, 64, "minecraft:desert"));
        Terrain t = terrain("flat"); t.biome = "minecraft:snowy_plains"; t.light = 0; t.denyOrigin = true;
        add(samples, "freeze_top_layer", "origin_denied", 17, origin, t);
        Map<String, Object> configs = new TreeMap<>(custom);
        for (String name : List.of("bamboo_no_podzol", "bamboo_some_podzol", "vines", "underwater_magma", "freeze_top_layer")) configs.put(name, document(name));
        return Map.of("samples", samples, "configured", configs);
    }

    static Object modified(String original, String jsonKey, Object value) throws Exception {
        Object copy = parse(document(original).toString());
        call(call(call(copy, "getAsJsonObject"), "get", "config"), "add", jsonKey, value);
        return copy;
    }

    static List<Object> supportTables() throws Exception {
        List<Object> tables = new ArrayList<>();
        Object support = supportWorld(), pos = make("core.BlockPos", 0, 64, 0);
        for (String block : List.of("BAMBOO", "BAMBOO_SAPLING", "KELP", "KELP_PLANT", "SEA_PICKLE", "SNOW")) {
            Object plant = state(block);
            List<int[]> ranges = new ArrayList<>();
            int[] previous = null;
            for (Object soil : statesById) {
                supportSoil = soil;
                boolean survives = (boolean) call(plant, "canSurvive", support, pos);
                int[] row = {id(soil), id(soil) + 1, survives ? 1 : 0};
                if (previous != null && previous[2] == row[2]) previous[1]++;
                else { ranges.add(row); previous = row; }
            }
            tables.add(Map.of("state", id(plant), "state_json", stateJson(plant), "ranges", ranges));
        }
        return tables;
    }

    static List<Object> vineSurvival() throws Exception {
        List<Object> samples = new ArrayList<>();
        Pos origin = new Pos(0, 64, 0);
        for (String block : List.of("STONE", "OAK_SLAB", "OAK_STAIRS", "OAK_LEAVES", "GLASS", "AIR")) {
            for (Object direction : type("core.Direction").getEnumConstants()) {
                if (direction.toString().equals("down")) continue;
                Object vine = call(state("VINE"), "setValue", call(type("world.level.block.VineBlock"), "getPropertyForFace", direction), true);
                Terrain t = terrain("air").at(position(call(nativePos(origin), "relative", direction)), block);
                scene = new Scene(t, origin);
                samples.add(Map.of("state", id(vine), "terrain", t.json(), "origin", xyz(origin), "lower_biome", t.biome,
                    "split_y", -64, "result", call(vine, "canSurvive", world(), nativePos(origin))));
                t = terrain("air"); t.overrides.put(new Pos(0, 65, 0), vine);
                scene = new Scene(t, origin);
                samples.add(Map.of("state", id(vine), "terrain", t.json(), "origin", xyz(origin), "lower_biome", t.biome,
                    "split_y", -64, "result", call(vine, "canSurvive", world(), nativePos(origin))));
            }
        }
        return samples;
    }

    static Map<String, Object> extra() throws Exception {
        captureGaussian = true;
        List<Object> samples = new ArrayList<>();
        Pos origin = new Pos(-17, 64, 16);
        for (String type : List.of("coral_tree", "coral_claw", "coral_mushroom")) custom.put(type, configured(type, Map.of()));
        for (String name : List.of("kelp", "sea_pickle", "coral_tree", "coral_claw", "coral_mushroom")) {
            for (long seed : new long[]{0, 1, 2, 3, 4, 5, 17, 42, -1, 846692123413862008L}) {
                for (String policy : List.of("all", "reject", "checker", "source_chunk")) {
                    Terrain t = new Terrain().layer(-64, 47, "STONE").layer(48, 95, "WATER"); t.writePolicy = policy;
                    add(samples, name, "deep_" + policy, seed, origin, t);
                }
            }
            for (int depth : new int[]{0, 1, 2, 3, 5, 8, 11})
                add(samples, name, "water_depth_" + depth, 17, origin, new Terrain().layer(-64, 63, "STONE").layer(64, 63 + depth, "WATER"));
            for (String ground : List.of("MAGMA_BLOCK", "SOUL_SAND", "OAK_SLAB", "OAK_FENCE", "GRAVEL", "GLASS", "ICE"))
                add(samples, name, "floor_" + ground, 42, origin, new Terrain().layer(-64, 63, ground).layer(64, 95, "WATER"));
            Terrain denied = terrain("ocean"); denied.denyOrigin = true;
            add(samples, name, "origin_denied", 17, origin, denied);
            Terrain roof = new Terrain().layer(-64, 316, "STONE").layer(317, 319, "WATER");
            add(samples, name, "world_ceiling", 17, new Pos(15, 318, -16), roof);
        }
        custom.put("pickle_uniform", configured("sea_pickle", Map.of("count", Map.of("type", "minecraft:uniform", "min_inclusive", 0, "max_inclusive", 16))));
        custom.put("pickle_normal", configured("sea_pickle", Map.of("count", Map.of("type", "minecraft:clamped_normal", "mean", 9.0, "deviation", 2.0, "min_inclusive", 0, "max_inclusive", 16))));
        custom.put("pickle_zero", configured("sea_pickle", Map.of("count", 0)));
        for (String name : List.of("pickle_uniform", "pickle_normal", "pickle_zero")) for (long seed : new long[]{0, 17, 42})
            add(samples, name, "ocean", seed, origin, terrain("ocean"));

        for (String name : List.of("huge_brown_mushroom", "huge_red_mushroom")) {
            for (long seed = 0; seed < 24; seed++) add(samples, name, "flat_height_corpus", seed, origin, terrain("flat"));
            for (String policy : List.of("reject", "checker", "source_chunk")) {
                Terrain t = terrain("flat"); t.writePolicy = policy;
                add(samples, name, "writes_" + policy, 17, origin, t);
            }
            for (String soil : List.of("MYCELIUM", "MUD", "STONE", "SAND", "PODZOL", "MOSS_BLOCK"))
                add(samples, name, "soil_" + soil, 17, origin, new Terrain().layer(-64, 63, soil));
            for (String obstacle : List.of("STONE", "OAK_LEAVES", "BROWN_MUSHROOM_BLOCK", "SNOW", "SHORT_GRASS")) {
                add(samples, name, "column_" + obstacle, 17, origin, terrain("flat").at(new Pos(-17, 66, 16), obstacle));
                add(samples, name, "cap_" + obstacle, 17, origin, terrain("flat").at(new Pos(-15, 69, 16), obstacle));
            }
            for (int y : new int[]{-64, -63, 306, 310, 311, 312, 313, 314, 319}) {
                Terrain t = new Terrain().layer(-64, y - 1, "DIRT");
                add(samples, name, "height_bound_" + y, 42, new Pos(-17, y, 16), t);
            }
            for (int radius : new int[]{-1, 0, 1, 2, 3, 4}) {
                String customName = name + "_radius_" + radius;
                custom.put(customName, modified(name, "foliage_radius", parse(Integer.toString(radius))));
                add(samples, customName, "radius", 17, origin, terrain("flat"));
            }
            String weighted = name + "_weighted";
            custom.put(weighted, modified(name, "cap_provider", parse("{\"type\":\"minecraft:weighted_state_provider\",\"entries\":[{\"data\":{\"Name\":\"minecraft:stone\"},\"weight\":1},{\"data\":{\"Name\":\"minecraft:red_mushroom_block\"},\"weight\":3}]}")));
            add(samples, weighted, "weighted_provider", 17, origin, terrain("flat"));
            String rules = name + "_rules";
            custom.put(rules, modified(name, "cap_provider", parse("{\"type\":\"minecraft:rule_based_state_provider\",\"rules\":[{\"if_true\":{\"type\":\"minecraft:matching_blocks\",\"offset\":[0,-1,0],\"blocks\":\"minecraft:grass_block\"},\"then\":{\"type\":\"minecraft:simple_state_provider\",\"state\":{\"Name\":\"minecraft:gold_block\"}}}]}")));
            add(samples, rules, "provider_samples_origin", 17, origin, terrain("flat"));
            Terrain denied = terrain("flat"); denied.denyOrigin = true;
            add(samples, name, "origin_denied", 17, origin, denied);
        }
        custom.put("pile_normal", configured("block_pile", Map.of("state_provider", parse("{\"type\":\"minecraft:randomized_int_state_provider\",\"source\":{\"type\":\"minecraft:simple_state_provider\",\"state\":{\"Name\":\"minecraft:snow\"}},\"property\":\"layers\",\"values\":{\"type\":\"minecraft:clamped_normal\",\"mean\":4.0,\"deviation\":1.25,\"min_inclusive\":1,\"max_inclusive\":8}}"))));
        for (String name : List.of("pile_hay", "pile_ice", "pile_melon", "pile_pumpkin", "pile_snow", "pile_normal")) {
            for (String soil : List.of("GRASS_BLOCK", "DIRT_PATH", "OAK_SLAB", "SOUL_SAND", "WATER")) {
                for (long seed : new long[]{0, 17, 42})
                    add(samples, name, "soil_" + soil, seed, origin, new Terrain().layer(-64, 63, soil));
            }
            for (String policy : List.of("reject", "checker", "source_chunk")) {
                Terrain t = terrain("flat"); t.writePolicy = policy;
                add(samples, name, "writes_" + policy, 17, origin, t);
            }
            for (int y : new int[]{-60, -59, 319})
                add(samples, name, "height_bound_" + y, 17, new Pos(15, y, -16), new Terrain().layer(-64, y - 1, "DIRT"));
            Terrain t = terrain("flat"); t.denyOrigin = true;
            add(samples, name, "origin_denied", 17, origin, t);
        }
        Map<String, Object> configs = new TreeMap<>(custom);
        for (String name : List.of("kelp", "sea_pickle", "huge_brown_mushroom", "huge_red_mushroom", "pile_hay", "pile_ice", "pile_melon", "pile_pumpkin", "pile_snow")) configs.put(name, document(name));
        List<Object> rejected = new ArrayList<>();
        for (Object doc : List.of(
            configured("sea_pickle", Map.of("count", 257)),
            configured("sea_pickle", Map.of("count", Map.of("type", "minecraft:missing_int"))),
            configured("block_pile", Map.of("state_provider", Map.of("type", "minecraft:missing_provider"))),
            modified("huge_brown_mushroom", "can_place_on", parse("{\"type\":\"minecraft:missing_predicate\"}")))) {
            try { decode("world.level.levelgen.feature.ConfiguredFeature", doc); throw new AssertionError("Expected native codec rejection: " + doc); }
            catch (InvocationTargetException expected) { rejected.add(doc); }
        }
        return Map.of("samples", samples, "configured", configs, "survival_support", supportTables(),
            "vine_survival", vineSurvival(), "native_codec_rejections", rejected);
    }

    static Map<String, Object> shapes() throws Exception {
        captureGaussian = true;
        List<Object> samples = new ArrayList<>();
        Pos origin = new Pos(-17, 64, 16);
        for (String name : List.of("forest_rock", "ice_spike")) {
            for (long seed = 0; seed < 128; seed++) {
                Terrain t = new Terrain().layer(-64, 62, "DIRT").layer(63, 63, name.equals("ice_spike") ? "SNOW_BLOCK" : "GRASS_BLOCK");
                add(samples, name, "shape_seed_corpus", seed, origin, t);
            }
            for (String policy : List.of("reject", "checker", "source_chunk")) {
                Terrain t = new Terrain().layer(-64, 62, "DIRT").layer(63, 63, name.equals("ice_spike") ? "SNOW_BLOCK" : "GRASS_BLOCK");
                t.writePolicy = policy;
                add(samples, name, "writes_" + policy, 17, origin, t);
            }
            for (String soil : List.of("STONE", "WATER", "DIRT", "SNOW", "SNOW_BLOCK", "SAND", "MYCELIUM", "PODZOL"))
                add(samples, name, "soil_" + soil, 42, origin, new Terrain().layer(-64, 63, soil));
            for (int y : new int[]{-64, -63, -62, -61, -60, -59, 318, 319})
                add(samples, name, "bounds_" + y, 42, new Pos(-17, y, 16), new Terrain().layer(-64, y - 1, name.equals("ice_spike") ? "SNOW_BLOCK" : "DIRT"));
            Terrain denied = terrain("flat"); denied.denyOrigin = true;
            add(samples, name, "origin_denied", 17, origin, denied);
        }
        for (long seed : new long[]{0, 1, 17, 42, -1}) for (String policy : List.of("all", "reject", "checker", "source_chunk")) {
            for (Object direction : type("core.Direction").getEnumConstants()) {
                Pos p = new Pos(-17, 60, 16);
                Terrain t = new Terrain().layer(-64, 47, "STONE").layer(48, 62, "WATER");
                t.at(position(call(nativePos(p), "relative", direction)), "PACKED_ICE"); t.writePolicy = policy;
                add(samples, "blue_ice", "adjacent_" + direction + "_" + policy, seed, p, t);
            }
        }
        for (int y : new int[]{-64, -63, 61, 62, 63, 64, 319}) {
            Pos p = new Pos(-17, y, 16);
            Terrain t = new Terrain().layer(-64, 319, "WATER").at(new Pos(-16, y, 16), "PACKED_ICE");
            add(samples, "blue_ice", "bounds_" + y, 17, p, t);
        }
        for (String initial : List.of("AIR", "STONE", "WATER", "ICE", "PACKED_ICE")) {
            Pos p = new Pos(-17, 62, 16);
            Terrain t = new Terrain().layer(-64, 61, "WATER").at(p, initial).at(new Pos(-16, 62, 16), "PACKED_ICE");
            add(samples, "blue_ice", "water_below_" + initial, 17, p, t);
        }
        sampleSeaLevel = 80;
        add(samples, "blue_ice", "custom_world_sea_level", 17, new Pos(-17, 79, 16), new Terrain().layer(-64, 79, "WATER").at(new Pos(-16, 79, 16), "PACKED_ICE"));
        sampleSeaLevel = 63;
        Terrain denied = terrain("ocean"); denied.denyOrigin = true;
        add(samples, "blue_ice", "origin_denied", 17, new Pos(-17, 60, 16), denied);

        for (String name : List.of("iceberg_packed", "iceberg_blue")) {
            for (long seed = 0; seed < 32; seed++) {
                Terrain t = new Terrain().layer(-64, 39, "STONE").layer(40, 62, "WATER");
                add(samples, name, "sea_seed_corpus", seed, origin, t);
            }
            for (long seed : new long[]{0, 1, 17, 42, -1}) {
                for (String policy : List.of("reject", "checker", "source_chunk")) {
                    Terrain t = new Terrain().layer(-64, 39, "STONE").layer(40, 62, "WATER"); t.writePolicy = policy;
                    add(samples, name, "writes_" + policy, seed, origin, t);
                }
                for (String ground : List.of("AIR", "STONE", "ICE", "PACKED_ICE", "SNOW_BLOCK", "SNOW"))
                    add(samples, name, "filled_" + ground, seed, origin, new Terrain().layer(-64, 100, ground));
            }
            Terrain t = terrain("ocean"); t.denyOrigin = true;
            add(samples, name, "origin_denied", 17, origin, t);
            sampleSeaLevel = 80;
            add(samples, name, "uses_generator_sea_level", 17, origin, new Terrain().layer(-64, 62, "WATER"));
            sampleSeaLevel = 63;
        }
        custom.put("blob_custom_predicate", configured("block_blob", Map.of("state", Map.of("Name", "minecraft:gold_block"), "can_place_on", parse("{\"type\":\"minecraft:matching_blocks\",\"blocks\":\"minecraft:stone\"}"))));
        add(samples, "blob_custom_predicate", "custom_soil", 17, origin, new Terrain().layer(-64, 50, "STONE"));
        custom.put("spike_custom_predicate", modified("ice_spike", "state", parse("{\"Name\":\"minecraft:gold_block\"}")));
        add(samples, "spike_custom_predicate", "custom_state", 17, origin, new Terrain().layer(-64, 63, "SNOW_BLOCK"));
        Map<String, Object> configs = new TreeMap<>(custom);
        for (String name : List.of("forest_rock", "ice_spike", "blue_ice", "iceberg_packed", "iceberg_blue")) configs.put(name, document(name));
        return Map.of("samples", samples, "configured", configs, "ellipse_math", ellipseMath());
    }

    static List<Object> ellipseMath() throws Exception {
        Object feature = make("world.level.levelgen.feature.IcebergFeature", field("world.level.levelgen.feature.configurations.BlockStateConfiguration", "CODEC"));
        Method method = type("world.level.levelgen.feature.IcebergFeature").getDeclaredMethod("signedDistanceEllipse", int.class, int.class, type("core.BlockPos"), int.class, int.class, double.class);
        method.setAccessible(true);
        List<Object> samples = new ArrayList<>();
        for (long seed = 0; seed < 32; seed++) {
            Object random = random(seed);
            double angle = (double) call(random, "nextDouble") * 2.0 * Math.PI;
            for (int[] row : List.of(new int[]{3, -5, 0, 0, 7, 3}, new int[]{-7, 3, 1, -2, 11, -1}, new int[]{0, 0, 0, 0, 5, 0}, new int[]{4, 1, 2, -1, 6, 0})) {
                double result = (double) method.invoke(feature, row[0], row[1], make("core.BlockPos", row[2], 0, row[3]), row[4], row[5], angle);
                samples.add(Map.of("parameters", row, "angle_bits", Long.toUnsignedString(Double.doubleToRawLongBits(angle)),
                    "result_bits", Long.toUnsignedString(Double.doubleToRawLongBits(result))));
            }
        }
        return samples;
    }

    static String bits(double value) { return Long.toUnsignedString(Double.doubleToRawLongBits(value)); }

    static Map<String, Object> mathDiagnostics() throws Exception {
        List<Object> samples = new ArrayList<>();
        for (long seed = 0; seed < 1024; seed++) {
            Object random = random(seed);
            double angle = (double) call(random, "nextDouble") * 2.0 * Math.PI;
            for (double a : new double[]{angle, angle + Math.PI / 2.0}) {
                double x = (-8.0 * Math.cos(a) - 5.0 * Math.sin(a)) / 11.0;
                double z = (-8.0 * Math.sin(a) + 5.0 * Math.cos(a)) / -1.0;
                samples.add(Map.of("angle", bits(a), "sin", bits(Math.sin(a)), "cos", bits(Math.cos(a)),
                    "strict_sin", bits(StrictMath.sin(a)), "strict_cos", bits(StrictMath.cos(a)),
                    "x", bits(x), "z", bits(z), "pow_x", bits(Math.pow(x, 2.0)), "square_x", bits(x * x),
                    "distance", bits(Math.pow(x, 2.0) + Math.pow(z, 2.0) - 1.0)));
            }
        }
        return Map.of("samples", samples);
    }

    static Map<String, Object> mathBoundaries() {
        List<Double> angles = new ArrayList<>();
        angles.add(Math.scalb(1.0, -53) * 2.0 * Math.PI);
        angles.add(Math.nextDown(1.0) * 2.0 * Math.PI);
        for (int i = 0; i <= 160; i++) {
            double angle = i * Math.PI / 64.0;
            for (double a : new double[]{Math.nextDown(angle), angle, Math.nextUp(angle)})
                if (a >= 0.0 && a <= 2.5 * Math.PI) angles.add(a);
        }
        List<Object> samples = new ArrayList<>();
        for (double a : angles) {
            double x = (-8.0 * Math.cos(a) - 5.0 * Math.sin(a)) / 11.0;
            double z = (-8.0 * Math.sin(a) + 5.0 * Math.cos(a)) / -1.0;
            samples.add(Map.of("angle", bits(a), "sin", bits(Math.sin(a)), "cos", bits(Math.cos(a)),
                "distance", bits(Math.pow(x, 2.0) + Math.pow(z, 2.0) - 1.0)));
        }
        return Map.of("samples", samples);
    }

    public static void main(String[] args) throws Exception {
        bootstrap(args[1]);
        Object result = switch (args[0]) {
            case "metadata" -> metadata();
            case "math" -> args[2].equals("boundaries") ? mathBoundaries() : mathDiagnostics();
            case "samples" -> switch (args[2]) { case "priority" -> priority(); case "extra" -> extra(); case "shapes" -> shapes(); default -> throw new IllegalArgumentException(args[2]); };
            default -> throw new IllegalArgumentException(args[0]);
        };
        System.out.println("MISC_FEATURE_REFERENCE=" + call(gson, "toJson", result));
    }
}
