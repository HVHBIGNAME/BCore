import java.lang.reflect.*;
import java.nio.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;
import java.util.function.*;
import java.util.jar.*;
import java.util.stream.*;

/** Runs native 26.1 codecs, block predicates and configured features. */
public class BaseFeatureReference extends TreeReference {
    static Object gson, registries, configuredRegistry, placedRegistry, biomeRegistry, generator, biomeSource;
    static final Map<Object, Integer> stateIds = new IdentityHashMap<>();
    static final List<Object> statesById = new ArrayList<>();
    static final Map<String, Object> documents = new TreeMap<>();
    static final Map<Object, String> biomeNames = new IdentityHashMap<>();
    static final Map<Object, Predicate<Object>> heightPredicates = new IdentityHashMap<>();
    static Method px, py, pz;
    static Object chunkFactory, heightAccessor, worldRandom, registryOps, supportSoil;
    static Capture current;

    static Object holder(Object registry, String name) throws Exception {
        return ((Optional<?>) call(registry, "get", call(type("resources.Identifier"), "parse", name)))
            .orElseThrow(() -> new IllegalArgumentException("missing native holder " + name));
    }

    static Object parse(String text) throws Exception {
        return call(Class.forName("com.google.gson.JsonParser"), "parseString", text);
    }

    static Method declaredMethod(Class<?> owner, String name) throws Exception {
        for (Class<?> c = owner; c != null; c = c.getSuperclass()) {
            for (Method method : c.getDeclaredMethods()) if (method.getName().equals(name)) return method;
        }
        throw new NoSuchMethodException(owner + "." + name);
    }

    static void bootstrap(String jar) throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        registries = NativeWorldgenRegistries.load();
        Object builtins = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", field("core.registries.BuiltInRegistries", "REGISTRY"));
        Object allRegistries = call(type("core.HolderLookup$Provider"), "create", Stream.concat(
            (Stream<?>) call(builtins, "listRegistries"), (Stream<?>) call(registries, "listRegistries")));
        registryOps = call(type("resources.RegistryOps"), "create", Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null), allRegistries);
        configuredRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "CONFIGURED_FEATURE"));
        placedRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "PLACED_FEATURE"));
        biomeRegistry = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
        Object parameters = make("world.level.biome.MultiNoiseBiomeSourceParameterList",
            field("world.level.biome.MultiNoiseBiomeSourceParameterList$Preset", "OVERWORLD"), biomeRegistry);
        Object source = call(type("world.level.biome.MultiNoiseBiomeSource"), "createFromPreset", call(type("core.Holder"), "direct", parameters));
        biomeSource = source;
        Object settings = call(registries, "lookupOrThrow", field("core.registries.Registries", "NOISE_SETTINGS"));
        generator = make("world.level.levelgen.NoiseBasedChunkGenerator", source, holder(settings, "overworld"));
        air = state("AIR"); grass = state("GRASS_BLOCK");
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            stateIds.put(state, (int) call(type("world.level.block.Block"), "getId", state));
            statesById.add(state);
        }
        px = type("core.Vec3i").getMethod("getX"); py = type("core.Vec3i").getMethod("getY"); pz = type("core.Vec3i").getMethod("getZ");
        for (Object kind : type("world.level.levelgen.Heightmap$Types").getEnumConstants()) {
            @SuppressWarnings("unchecked") Predicate<Object> predicate = (Predicate<Object>) call(kind, "isOpaque");
            heightPredicates.put(kind, predicate);
        }
        heightAccessor = call(type("world.level.LevelHeightAccessor"), "create", -64, 384);
        Object blockStrategy = call(type("world.level.chunk.Strategy"), "createForBlockStates", field("world.level.block.Block", "BLOCK_STATE_REGISTRY"));
        Object biomeStrategy = call(type("world.level.chunk.Strategy"), "createForBiomes", call(biomeRegistry, "asHolderIdMap"));
        chunkFactory = make("world.level.chunk.PalettedContainerFactory", blockStrategy, air, null, biomeStrategy, holder(biomeRegistry, "plains"), null);
        try (JarFile input = new JarFile(jar)) {
            for (JarEntry entry : input.stream().filter(e -> e.getName().endsWith(".json")).toList()) {
                String name = entry.getName();
                if (name.startsWith("data/minecraft/worldgen/") || name.startsWith("data/minecraft/tags/block/") || name.startsWith("data/minecraft/tags/fluid/")) {
                    try (var bytes = input.getInputStream(entry)) {
                        documents.put(name.substring("data/minecraft/".length()), parse(new String(bytes.readAllBytes(), StandardCharsets.UTF_8)));
                    }
                }
            }
        }
    }

    static Object emptyWorld() throws Exception {
        return Proxy.newProxyInstance(BaseFeatureReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> switch (m.getName()) {
            case "getBlockState" -> air;
            case "getFluidState" -> call(air, "getFluidState");
            case "getMinY" -> -64;
            case "getMaxY" -> 319;
            case "getHeight" -> 384;
            case "getBlockEntity" -> null;
            default -> throw new UnsupportedOperationException(m.toString());
        });
    }

    static Object supportWorld() throws Exception {
        Object empty = emptyWorld();
        return Proxy.newProxyInstance(BaseFeatureReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> {
            if (m.getName().equals("getBlockState") || m.getName().equals("getFluidState")) {
                Object state = position(a[0]).equals(new Pos(0, 63, 0)) ? supportSoil : air;
                return m.getName().equals("getFluidState") ? call(state, "getFluidState") : state;
            }
            return m.invoke(empty, a);
        });
    }

    static Map<String, Object> catalog() throws Exception {
        Map<String, Object> result = new TreeMap<>();
        for (String directory : List.of("placed_feature", "configured_feature", "biome")) {
            Map<String, Object> entries = new TreeMap<>();
            String prefix = "worldgen/" + directory + "/";
            for (var entry : documents.entrySet()) if (entry.getKey().startsWith(prefix)) {
                Object value = entry.getValue();
                if (directory.equals("biome")) value = Map.of("features", call(call(value, "getAsJsonObject"), "get", "features"));
                entries.put("minecraft:" + entry.getKey().substring(prefix.length(), entry.getKey().length() - 5), value);
            }
            result.put(directory, entries);
        }
        List<String> overworldBiomes = new ArrayList<>();
        for (Object biome : (Set<?>) call(biomeSource, "possibleBiomes")) overworldBiomes.add(call(call(biome, "key"), "identifier").toString());
        Collections.sort(overworldBiomes);
        result.put("overworld_biomes", overworldBiomes);
        for (String directory : List.of("block", "fluid")) {
            Map<String, Object> entries = new TreeMap<>();
            String prefix = "tags/" + directory + "/";
            for (var entry : documents.entrySet()) if (entry.getKey().startsWith(prefix))
                entries.put("minecraft:" + entry.getKey().substring(prefix.length(), entry.getKey().length() - 5), entry.getValue());
            result.put(directory + "_tags", entries);
        }
        Map<String, Object> blocks = new TreeMap<>();
        Object registry = field("core.registries.BuiltInRegistries", "BLOCK");
        Object supportWorld = supportWorld(), supportPos = make("core.BlockPos", 0, 63, 0);
        for (Object block : (Iterable<?>) registry) {
            String name = call(registry, "getKey", block).toString();
            Object definition = call(block, "getStateDefinition");
            List<?> states = (List<?>) call(definition, "getPossibleStates");
            int first = stateIds.get(states.getFirst());
            List<Object> properties = new ArrayList<>();
            List<?> props = new ArrayList<>((Collection<?>) call(definition, "getProperties"));
            for (Object property : props) {
                List<String> values = new ArrayList<>();
                for (Object value : (Collection<?>) call(property, "getPossibleValues")) values.add((String) call(property, "getName", value));
                properties.add(List.of(call(property, "getName"), values));
            }
            for (int i = 0; i < states.size(); i++) {
                if (stateIds.get(states.get(i)) != first + i) throw new IllegalStateException("non-contiguous " + name);
                int index = 0;
                for (Object property : props) {
                    List<?> values = new ArrayList<>((Collection<?>) call(property, "getPossibleValues"));
                    index = index * values.size() + values.indexOf(call(states.get(i), "getValue", property));
                }
                if (index != i) throw new IllegalStateException("state order " + name);
            }
            Method survival = declaredMethod(block.getClass(), "canSurvive");
            Map<String, Object> blockData = new TreeMap<>();
            blockData.putAll(Map.of("first", first, "count", states.size(), "default", stateIds.get(call(block, "defaultBlockState")),
                "properties", properties, "survival", survival.getDeclaringClass().getSimpleName(), "class", block.getClass().getSimpleName(),
                "double_plant", type("world.level.block.DoublePlantBlock").isInstance(block)));
            if (type("world.level.block.VegetationBlock").isInstance(block)) {
                Method mayPlaceOn = declaredMethod(block.getClass(), "mayPlaceOn");
                mayPlaceOn.setAccessible(true);
                List<int[]> support = new ArrayList<>();
                int[] range = null;
                for (Object soil : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
                    int id = stateIds.get(soil);
                    supportSoil = soil;
                    if ((boolean) mayPlaceOn.invoke(block, soil, supportWorld, supportPos)) {
                        if (range != null && range[1] == id) range[1]++;
                        else { range = new int[]{id, id + 1}; support.add(range); }
                    }
                }
                blockData.put("support", support);
            }
            blocks.put(name, blockData);
        }
        result.put("blocks", blocks);
        Object world = emptyWorld(), origin = make("core.BlockPos", 0, 64, 0);
        Object fluids = field("core.registries.BuiltInRegistries", "FLUID");
        Map<String, Integer> fluidIds = new TreeMap<>();
        Map<String, Object> fluidDefaults = new TreeMap<>();
        List<Object> fluidStates = new ArrayList<>();
        Object jsonOps = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        Object fluidCodec = field("world.level.material.FluidState", "CODEC");
        for (Object fluid : (Iterable<?>) fluids) {
            String name = call(fluids, "getKey", fluid).toString();
            int id = (int) call(fluids, "getId", fluid);
            fluidIds.put(name, id);
            fluidDefaults.put(name, call(call(fluidCodec, "encodeStart", jsonOps, call(fluid, "defaultFluidState")), "getOrThrow"));
            for (Object state : (Iterable<?>) call(call(fluid, "getStateDefinition"), "getPossibleStates")) {
                Object encoded = call(call(fluidCodec, "encodeStart", jsonOps, state), "getOrThrow");
                fluidStates.add(Map.of("state", encoded, "fluid", id, "block", stateIds.get(call(state, "createLegacyBlock"))));
            }
        }
        result.put("fluids", fluidIds);
        result.put("fluid_defaults", fluidDefaults);
        result.put("fluid_states", fluidStates);
        List<int[]> ranges = new ArrayList<>();
        List<int[]> shapeRanges = new ArrayList<>();
        int[] previous = null;
        int[] previousShape = null;
        Object[] directions = (Object[]) type("core.Direction").getEnumConstants();
        Object fire = field("world.level.block.Blocks", "FIRE");
        Method canBurn = declaredMethod(fire.getClass(), "canBurn"); canBurn.setAccessible(true);
        for (Object state : (Iterable<?>) field("world.level.block.Block", "BLOCK_STATE_REGISTRY")) {
            int flags = 0;
            for (String method : List.of("isAir", "isSolid", "canBeReplaced", "liquid", "blocksMotion")) {
                flags = (flags << 1) | ((boolean) call(state, method) ? 1 : 0);
            }
            int faces = 0;
            for (int i = 0; i < directions.length; i++) if ((boolean) call(state, "isFaceSturdy", world, origin, directions[i])) faces |= 1 << i;
            Object fluid = call(state, "getFluidState");
            int id = stateIds.get(state);
            supportSoil = state;
            int attach = 0, center = 0;
            for (int i = 0; i < directions.length; i++) {
                if ((boolean) call(type("world.level.block.MultifaceBlock"), "canAttachTo", world, directions[i], origin, state)) attach |= 1 << i;
                if ((boolean) call(type("world.level.block.Block"), "canSupportCenter", supportWorld, supportPos, directions[i])) center |= 1 << i;
            }
            int[] shapeRow = {id, id + 1, attach, center, (boolean) canBurn.invoke(fire, state) ? 1 : 0};
            if (previousShape != null && Arrays.equals(Arrays.copyOfRange(previousShape, 2, 5), Arrays.copyOfRange(shapeRow, 2, 5))) previousShape[1]++;
            else { shapeRanges.add(shapeRow); previousShape = shapeRow; }
            int[] row = {id, id + 1, flags, (int) call(fluids, "getId", call(fluid, "getType")), (int) call(fluid, "getAmount"), faces};
            if (previous != null && Arrays.equals(Arrays.copyOfRange(previous, 2, 6), Arrays.copyOfRange(row, 2, 6))) previous[1]++;
            else { ranges.add(row); previous = row; }
        }
        result.put("state_count", stateIds.size());
        result.put("state_ranges", ranges);
        result.put("shape_ranges", shapeRanges);
        Map<String, Integer> biomeIds = new TreeMap<>();
        Object idMap = call(biomeRegistry, "asHolderIdMap");
        try (Stream<?> elements = (Stream<?>) call(biomeRegistry, "listElements")) {
            for (Object holder : elements.toList()) biomeIds.put(call(call(holder, "key"), "identifier").toString(), (int) call(idMap, "getId", holder));
        }
        result.put("biome_ids", biomeIds);
        return result;
    }

    static Pos position(Object pos) throws Exception { return new Pos((int) px.invoke(pos), (int) py.invoke(pos), (int) pz.invoke(pos)); }
    static Object nativePos(Pos pos) throws Exception { return make("core.BlockPos", pos.x(), pos.y(), pos.z()); }
    static int id(Object state) { return Objects.requireNonNull(stateIds.get(state)); }
    static int[] xyz(Pos p) { return new int[]{p.x(), p.y(), p.z()}; }
    static void digest(MessageDigest md, int... row) {
        ByteBuffer buffer = ByteBuffer.allocate(4 * row.length).order(ByteOrder.LITTLE_ENDIAN);
        for (int n : row) buffer.putInt(n);
        md.update(buffer.array());
    }

    static final class Terrain {
        final List<int[]> layers = new ArrayList<>();
        final Map<Pos, Object> overrides = new HashMap<>();
        String biome = "minecraft:plains", writePolicy = "all";
        int light = 12;
        boolean denyOrigin;
        Terrain layer(int lo, int hi, String block) throws Exception { layers.add(new int[]{lo, hi, id(state(block))}); return this; }
        Terrain at(Pos pos, String block) throws Exception { overrides.put(pos, state(block)); return this; }
        Object initial(Pos pos) throws Exception {
            if (pos.y() < -64 || pos.y() > 319) return state("VOID_AIR");
            Object override = overrides.get(pos);
            if (override != null) return override;
            for (int[] layer : layers) if (pos.y() >= layer[0] && pos.y() <= layer[1])
                return statesById.get(layer[2]);
            return air;
        }
        Map<String, Object> json() {
            List<Pos> keys = new ArrayList<>(overrides.keySet());
            keys.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
            List<int[]> overridesJson = keys.stream().map(p -> new int[]{p.x(), p.y(), p.z(), id(overrides.get(p))}).toList();
            return Map.of("layers", layers, "overrides", overridesJson, "biome", biome, "write_policy", writePolicy,
                "light", light, "deny_origin", denyOrigin);
        }
    }

    static Terrain terrain(String name) throws Exception {
        Terrain t = new Terrain();
        switch (name) {
            case "flat" -> t.layer(-64, 62, "DIRT").layer(63, 63, "GRASS_BLOCK");
            case "sand" -> t.layer(-64, 63, "SAND");
            case "stone" -> t.layer(-64, 80, "STONE");
            case "ocean" -> t.layer(-64, 59, "DIRT").layer(60, 67, "WATER");
            case "shallow" -> t.layer(-64, 63, "DIRT").layer(64, 64, "WATER");
            case "cave" -> t.layer(-64, 31, "STONE").layer(48, 50, "STONE").layer(80, 86, "STONE");
            case "ceiling" -> t.layer(-64, 317, "DIRT").layer(318, 318, "GRASS_BLOCK");
            case "air" -> { }
            default -> throw new IllegalArgumentException(name);
        }
        return t;
    }

    static final class Capture {
        final Terrain terrain;
        final Map<Pos, Object> writes = new HashMap<>(), chunks = new HashMap<>();
        final List<int[]> writePrefix = new ArrayList<>(), marks = new ArrayList<>(), ticks = new ArrayList<>();
        final MessageDigest writeHash;
        int writeCount;
        Capture(Terrain terrain) throws Exception { this.terrain = terrain; writeHash = MessageDigest.getInstance("MD5"); }
        Object block(Pos pos) throws Exception { return writes.containsKey(pos) ? writes.get(pos) : terrain.initial(pos); }
        int height(Object kind, int x, int z) throws Exception {
            for (int y = 319; y >= -64; y--) if (heightPredicates.get(kind).test(block(new Pos(x, y, z)))) return y + 1;
            return -64;
        }
        boolean put(Pos pos, Object state, int flags) {
            boolean accepted = pos.y() >= -64 && pos.y() <= 319 && !terrain.writePolicy.equals("reject")
                && (!terrain.writePolicy.equals("checker") || ((pos.x() + pos.z()) & 1) == 0);
            int[] row = {pos.x(), pos.y(), pos.z(), id(state), flags, accepted ? 1 : 0};
            digest(writeHash, row); writeCount++;
            if (writePrefix.size() < 32) writePrefix.add(row);
            if (accepted) writes.put(pos, state);
            return accepted;
        }
        Object chunk(Pos pos) throws Exception {
            if (pos.y() >= -64 && pos.y() <= 319) marks.add(xyz(pos));
            Pos key = new Pos(pos.x() >> 4, 0, pos.z() >> 4);
            Object chunk = chunks.get(key);
            if (chunk != null) return chunk;
            Object sections = Array.newInstance(type("world.level.chunk.LevelChunkSection"), 24);
            for (int i = 0; i < 24; i++) Array.set(sections, i, make("world.level.chunk.LevelChunkSection", chunkFactory));
            chunk = make("world.level.chunk.ProtoChunk", make("world.level.ChunkPos", key.x(), key.z()), field("world.level.chunk.UpgradeData", "EMPTY"),
                sections, make("world.ticks.ProtoChunkTicks"), make("world.ticks.ProtoChunkTicks"), heightAccessor, chunkFactory, null);
            chunks.put(key, chunk);
            return chunk;
        }
        Map<String, Object> snapshot() throws Exception {
            List<Pos> sorted = new ArrayList<>(writes.keySet());
            sorted.sort(Comparator.comparingInt(Pos::x).thenComparingInt(Pos::y).thenComparingInt(Pos::z));
            MessageDigest blocksHash = MessageDigest.getInstance("MD5");
            int count = 0;
            for (Pos pos : sorted) if (writes.get(pos) != terrain.initial(pos)) {
                digest(blocksHash, pos.x(), pos.y(), pos.z(), id(writes.get(pos))); count++;
            }
            // Each getChunk in these feature implementations is immediately
            // followed by markPosForPostprocessing. Verify against native lists.
            int nativeMarks = 0;
            for (Object chunk : chunks.values()) for (Object list : (Object[]) call(chunk, "getPostProcessing"))
                if (list != null) nativeMarks += (int) call(list, "size");
            if (nativeMarks != marks.size()) throw new IllegalStateException("postprocessing capture mismatch");
            return Map.of("write_count", writeCount, "write_prefix", writePrefix, "write_md5", HexFormat.of().formatHex(writeHash.digest()),
                "changed_blocks", count, "blocks_md5", HexFormat.of().formatHex(blocksHash.digest()), "marks", marks, "ticks", ticks);
        }
    }

    @SuppressWarnings("unchecked")
    static Object world() throws Exception {
        return Proxy.newProxyInstance(BaseFeatureReference.class.getClassLoader(), new Class<?>[]{type("world.level.WorldGenLevel")}, (p, m, a) -> switch (m.getName()) {
            case "getBlockState" -> current.block(position(a[0]));
            case "getFluidState" -> call(current.block(position(a[0])), "getFluidState");
            case "isEmptyBlock" -> call(current.block(position(a[0])), "isAir");
            case "isWaterAt" -> call(current.block(position(a[0])), "getFluidState") != null &&
                (boolean) type("world.level.material.FluidState").getMethod("is", type("tags.TagKey")).invoke(call(current.block(position(a[0])), "getFluidState"), field("tags.FluidTags", "WATER"));
            case "isStateAtPosition" -> ((Predicate<Object>) a[1]).test(current.block(position(a[0])));
            case "isFluidAtPosition" -> ((Predicate<Object>) a[1]).test(call(current.block(position(a[0])), "getFluidState"));
            case "getHeight" -> a == null || a.length == 0 ? 384 : current.height(a[0], (int) a[1], (int) a[2]);
            case "getHeightmapPos" -> { Pos pos = position(a[1]); yield make("core.BlockPos", pos.x(), current.height(a[0], pos.x(), pos.z()), pos.z()); }
            case "getMinY" -> -64;
            case "getMaxY" -> 319;
            case "isOutsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : position(a[0]).y(); yield y < -64 || y > 319; }
            case "isInsideBuildHeight" -> { int y = a[0] instanceof Integer i ? i : position(a[0]).y(); yield y >= -64 && y <= 319; }
            case "getRawBrightness" -> current.terrain.light;
            case "getBiome" -> holder(biomeRegistry, current.terrain.biome);
            case "ensureCanWrite" -> !current.terrain.denyOrigin;
            case "setBlock" -> current.put(position(a[0]), a[1], (int) a[2]);
            case "getChunk" -> { if (a.length != 1) throw new UnsupportedOperationException(m.toString()); yield current.chunk(position(a[0])); }
            case "scheduleTick" -> {
                Pos pos = position(a[0]);
                boolean fluid = type("world.level.material.Fluid").isInstance(a[1]);
                int target = fluid ? (int) call(field("core.registries.BuiltInRegistries", "FLUID"), "getId", a[1]) : id(call(a[1], "defaultBlockState"));
                current.ticks.add(new int[]{pos.x(), pos.y(), pos.z(), target, (int) a[2], fluid ? 1 : 0});
                yield null;
            }
            case "getRandom" -> worldRandom;
            case "getBlockEntity" -> null;
            default -> throw new UnsupportedOperationException(m.toString());
        });
    }

    static Object decode(String codecOwner, Object json) throws Exception {
        return call(call(field(codecOwner, codecOwner.endsWith("ConfiguredFeature") || codecOwner.endsWith("PlacedFeature") ? "DIRECT_CODEC" : "CODEC"), "parse", registryOps, json), "getOrThrow");
    }

    static Object stateJson(Object state) throws Exception {
        Object ops = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        return call(call(field("world.level.block.state.BlockState", "CODEC"), "encodeStart", ops, state), "getOrThrow");
    }

    static Object random(long seed) throws Exception { return make("world.level.levelgen.WorldgenRandom", make("world.level.levelgen.XoroshiroRandomSource", seed)); }

    static String biomeFor(String feature) throws Exception {
        List<String> names = new ArrayList<>();
        Object target = call(holder(placedRegistry, feature), "value");
        for (Object biome : (Set<?>) call(biomeSource, "possibleBiomes"))
            if ((boolean) call(call(call(biome, "value"), "getGenerationSettings"), "hasFeature", target)) names.add(call(call(biome, "key"), "identifier").toString());
        Collections.sort(names);
        return names.isEmpty() ? "minecraft:plains" : names.getFirst();
    }

    static Map<String, Object> featureSample(String name, Object feature, boolean placed, long seed, Pos origin, Terrain terrain) throws Exception {
        current = new Capture(terrain);
        Object random = random(seed);
        worldRandom = random(987654321L);
        boolean result;
        try {
            result = (boolean) call(feature, placed ? "placeWithBiomeCheck" : "place", world(), generator, random, nativePos(origin));
        } catch (Exception e) { throw new IllegalStateException(name + " seed=" + seed + " origin=" + origin, e); }
        Map<String, Object> sample = new LinkedHashMap<>(current.snapshot());
        sample.putAll(Map.of("name", name, "placed", placed, "seed", seed, "origin", xyz(origin), "terrain", terrain.json(), "result", result,
            "next_i64", call(random, "nextLong"), "world_next_i64", call(worldRandom, "nextLong")));
        return sample;
    }

    static Map<String, Object> baseSamples() throws Exception {
        List<Object> samples = new ArrayList<>();
        Map<String, Object> custom = new TreeMap<>();
        String prefix = "worldgen/configured_feature/";
        Set<String> simpleTypes = Set.of("minecraft:simple_block", "minecraft:block_column", "minecraft:disk", "minecraft:spring_feature", "minecraft:seagrass", "minecraft:lake");
        for (var entry : documents.entrySet()) {
            if (!entry.getKey().startsWith(prefix)) continue;
            Object doc = call(entry.getValue(), "getAsJsonObject");
            String featureType = (String) call(call(doc, "get", "type"), "getAsString");
            if (!simpleTypes.contains(featureType)) continue;
            String name = entry.getKey().substring(prefix.length(), entry.getKey().length() - 5);
            Object feature = call(holder(configuredRegistry, name), "value");
            String scenario = featureType.equals("minecraft:lake") ? "stone" : featureType.equals("minecraft:seagrass") ? "ocean" : "flat";
            for (long seed : new long[]{0, 17}) samples.add(featureSample(name, feature, false, seed, new Pos(8, 64, 8), terrain(scenario)));
        }
        for (String name : List.of("flower_default", "flower_plain", "flower_meadow", "flower_flower_forest", "flower_pale_garden", "tall_grass", "large_fern", "cactus", "sugar_cane", "cave_vine", "cave_vine_in_moss", "disk_sand", "disk_grass", "lake_lava")) {
            Object feature = call(holder(configuredRegistry, name), "value");
            Terrain t = terrain(name.equals("lake_lava") ? "stone" : "flat"); t.writePolicy = "reject";
            samples.add(featureSample(name, feature, false, 1, new Pos(8, 64, 8), t));
            t = terrain("ceiling"); t.writePolicy = "checker";
            samples.add(featureSample(name, feature, false, 42, new Pos(-17, 319, 16), t));
        }
        for (String name : List.of("cactus", "sugar_cane", "cave_vine")) {
            Object feature = call(holder(configuredRegistry, name), "value");
            for (int clearance : new int[]{0, 1, 2}) {
                Terrain t = terrain("flat").at(new Pos(8, 65 + clearance, 8), "STONE");
                samples.add(featureSample(name, feature, false, 17, new Pos(8, 64, 8), t));
            }
        }
        for (String name : List.of("spring_water", "spring_lava_overworld", "spring_lava_frozen")) {
            Object feature = call(holder(configuredRegistry, name), "value");
            for (String policy : List.of("all", "reject")) {
                Terrain t = terrain("stone").at(new Pos(7, 64, 8), "AIR"); t.writePolicy = policy;
                samples.add(featureSample(name, feature, false, 17, new Pos(8, 64, 8), t));
            }
        }
        for (String name : List.of("seagrass_short", "seagrass_tall", "seagrass_mid")) {
            Object feature = call(holder(configuredRegistry, name), "value");
            for (String scenario : List.of("shallow", "flat")) samples.add(featureSample(name, feature, false, 1, new Pos(8, 64, 8), terrain(scenario)));
        }
        // Native selector codecs with small inline leaves exercise branch failure,
        // return propagation and the shared stream without requiring tree dispatch.
        Object flower = parse("{\"feature\":{\"type\":\"minecraft:simple_block\",\"config\":{\"to_place\":{\"type\":\"minecraft:simple_state_provider\",\"state\":{\"Name\":\"minecraft:fern\"}}}},\"placement\":[{\"type\":\"minecraft:count\",\"count\":3},{\"type\":\"minecraft:random_offset\",\"xz_spread\":{\"type\":\"minecraft:trapezoid\",\"min\":-2,\"max\":2,\"plateau\":0},\"y_spread\":0}]}");
        Object stone = parse("{\"feature\":{\"type\":\"minecraft:simple_block\",\"config\":{\"to_place\":{\"type\":\"minecraft:weighted_state_provider\",\"entries\":[{\"data\":{\"Name\":\"minecraft:stone\"},\"weight\":1},{\"data\":{\"Name\":\"minecraft:gold_block\"},\"weight\":2}]}}},\"placement\":[]}");
        custom.put("selector_boolean", Map.of("type", "minecraft:random_boolean_selector", "config", Map.of("feature_true", flower, "feature_false", stone)));
        custom.put("selector_simple", Map.of("type", "minecraft:simple_random_selector", "config", Map.of("features", List.of(flower, stone))));
        custom.put("selector_weighted", Map.of("type", "minecraft:random_selector", "config", Map.of("features", List.of(Map.of("feature", flower, "chance", 0.4)), "default", stone)));
        custom.put("selector_failed_branch", Map.of("type", "minecraft:random_selector", "config", Map.of("features", List.of(Map.of("feature", flower, "chance", 1.0)), "default", stone)));
        for (var entry : custom.entrySet()) {
            Object feature = decode("world.level.levelgen.feature.ConfiguredFeature", parse((String) call(gson, "toJson", entry.getValue())));
            for (long seed : new long[]{0, 1, 17}) for (String scenario : List.of("flat", "sand"))
                samples.add(featureSample(entry.getKey(), feature, false, seed, new Pos(8, 64, 8), terrain(scenario)));
        }
        Object moss = parse("{\"type\":\"minecraft:simple_block\",\"config\":{\"to_place\":{\"type\":\"minecraft:simple_state_provider\",\"state\":{\"Name\":\"minecraft:pale_moss_carpet\"}}}}");
        custom.put("moss_carpet", moss);
        Object mossFeature = decode("world.level.levelgen.feature.ConfiguredFeature", moss);
        for (String policy : List.of("all", "reject", "checker")) {
            Terrain t = terrain("flat"); t.writePolicy = policy;
            for (int[] side : List.of(new int[]{0,-1}, new int[]{1,0}, new int[]{0,1}, new int[]{-1,0})) {
                t.at(new Pos(8 + side[0], 64, 8 + side[1]), "STONE");
                t.at(new Pos(8 + side[0], 65, 8 + side[1]), "STONE");
            }
            samples.add(featureSample("moss_carpet", mossFeature, false, 0, new Pos(8,64,8), t));
        }
        Terrain shore = terrain("sand"); shore.biome = biomeFor("patch_sugar_cane_badlands");
        for (int x = -32; x <= 16; x += 4) for (int z = -16; z <= 32; z++) shore.at(new Pos(x,63,z), "WATER");
        samples.add(featureSample("patch_sugar_cane_badlands", call(holder(placedRegistry, "patch_sugar_cane_badlands"), "value"), true, 17, new Pos(-16,64,0), shore));
        for (String name : List.of("patch_grass_plain", "patch_grass_forest", "patch_grass_taiga", "patch_grass_jungle", "patch_tall_grass", "patch_large_fern", "patch_sugar_cane", "patch_cactus", "flower_plain", "flower_default", "flower_meadow", "flower_flower_forest", "flower_pale_garden", "flower_cherry", "forest_flowers", "brown_mushroom_normal", "red_mushroom_normal", "patch_dry_grass_badlands", "patch_dead_bush_badlands", "patch_bush", "patch_firefly_bush_near_water", "patch_waterlily", "disk_sand", "seagrass_normal")) {
            Object feature = call(holder(placedRegistry, name), "value");
            for (long seed : new long[]{0, 17}) {
                Terrain t = terrain(name.contains("seagrass") || name.contains("waterlily") ? "ocean" : name.contains("cactus") || name.contains("dry_grass") ? "sand" : "flat");
                t.biome = biomeFor(name);
                samples.add(featureSample(name, feature, true, seed, new Pos(-16, 64, 0), t));
            }
        }
        return Map.of("samples", samples, "custom_configured", custom, "survival", survivalSamples(), "providers", providerSamples());
    }

    static List<Object> survivalSamples() throws Exception {
        List<Object> samples = new ArrayList<>();
        List<String> plants = List.of("SHORT_GRASS", "FERN", "TALL_GRASS", "LARGE_FERN", "DANDELION", "POPPY", "BLUE_ORCHID", "WITHER_ROSE", "PINK_PETALS", "WILDFLOWERS", "LEAF_LITTER", "SHORT_DRY_GRASS", "TALL_DRY_GRASS", "DEAD_BUSH", "BUSH", "FIREFLY_BUSH", "SWEET_BERRY_BUSH", "CACTUS", "CACTUS_FLOWER", "SUGAR_CANE", "SEAGRASS", "TALL_SEAGRASS", "BROWN_MUSHROOM", "RED_MUSHROOM", "LILY_PAD", "SMALL_DRIPLEAF", "SPORE_BLOSSOM", "FIRE", "SOUL_FIRE");
        for (String plant : plants) for (String soil : List.of("GRASS_BLOCK", "STONE", "SAND", "MYCELIUM", "MOSS_BLOCK", "FARMLAND", "WATER")) {
            Terrain t = new Terrain().layer(-64, 63, soil);
            samples.add(survivalSample(state(plant), new Pos(8, 64, 8), t));
        }
        for (String plant : plants) {
            samples.add(survivalSample(state(plant), new Pos(8, 60, 8), terrain("ocean")));
        }
        for (String plant : List.of("BROWN_MUSHROOM", "RED_MUSHROOM")) for (int light : new int[]{12, 13}) {
            Terrain t = new Terrain().layer(-64, 63, "STONE"); t.light = light;
            samples.add(survivalSample(state(plant), new Pos(8, 64, 8), t));
        }
        for (String plant : List.of("CACTUS", "SUGAR_CANE")) for (String neighbor : List.of("WATER", "LAVA", "FROSTED_ICE", "STONE", "OAK_LEAVES")) {
            Terrain t = terrain("sand").at(new Pos(9, plant.equals("CACTUS") ? 64 : 63, 8), neighbor);
            samples.add(survivalSample(state(plant), new Pos(8, 64, 8), t));
        }
        for (String plant : List.of("TALL_GRASS", "LARGE_FERN", "TALL_SEAGRASS", "SUNFLOWER")) {
            Object lower = state(plant);
            Object upper = call(lower, "setValue", field("world.level.block.DoublePlantBlock", "HALF"), field("world.level.block.state.properties.DoubleBlockHalf", "UPPER"));
            for (boolean correctBelow : new boolean[]{false, true}) {
                Terrain t = terrain("flat"); if (correctBelow) t.overrides.put(new Pos(8, 63, 8), lower);
                samples.add(survivalSample(upper, new Pos(8, 64, 8), t));
            }
        }
        for (String block : List.of("STONE", "OAK_LEAVES", "SOUL_SOIL", "WATER", "OAK_FENCE")) {
            Terrain t = terrain("air").at(new Pos(8,65,8), block);
            samples.add(survivalSample(state("SPORE_BLOSSOM"), new Pos(8,64,8), t));
            samples.add(survivalSample(state("FIRE"), new Pos(8,64,8), t));
        }
        return samples;
    }

    static Map<String, Object> survivalSample(Object state, Pos pos, Terrain terrain) throws Exception {
        current = new Capture(terrain);
        return Map.of("state", stateJson(state), "state_id", id(state), "origin", xyz(pos), "terrain", terrain.json(),
            "result", call(state, "canSurvive", world(), nativePos(pos)));
    }

    static void gatherProviders(Object json, Map<String, Object> providers) throws Exception {
        if ((boolean) call(json, "isJsonObject")) {
            Object object = call(json, "getAsJsonObject");
            Object type = call(object, "get", "type");
            if (type != null) {
                String name = (String) call(type, "getAsString");
                if (name.endsWith("_provider")) providers.put(json.toString(), json);
            }
            for (Object entry : (Set<?>) call(object, "entrySet")) gatherProviders(((Map.Entry<?, ?>) entry).getValue(), providers);
        } else if ((boolean) call(json, "isJsonArray")) for (Object child : (Iterable<?>) json) gatherProviders(child, providers);
    }

    static List<Object> providerSamples() throws Exception {
        Map<String, Object> providers = new TreeMap<>();
        for (String name : List.of("flower_plain", "flower_meadow", "flower_flower_forest", "flower_default", "grass_jungle", "cave_vine", "disk_sand", "disk_grass", "pile_hay"))
            gatherProviders(documents.get("worldgen/configured_feature/" + name + ".json"), providers);
        List<Object> samples = new ArrayList<>();
        for (Object json : providers.values()) {
            Object provider = decode("world.level.levelgen.feature.stateproviders.BlockStateProvider", json);
            for (long seed : new long[]{0, 17}) for (Pos pos : List.of(new Pos(0, 64, 0), new Pos(-9217, 75, 10483), new Pos(10000001, 300, -10000001))) {
                Terrain t = terrain("flat"); current = new Capture(t);
                Object random = random(seed);
                Object state = call(provider, "getOptionalState", world(), random, nativePos(pos));
                Map<String, Object> sample = new LinkedHashMap<>();
                sample.putAll(Map.of("provider", json, "seed", seed, "origin", xyz(pos), "terrain", t.json(), "next_i64", call(random, "nextLong")));
                sample.put("state", state == null ? null : id(state));
                if (type("world.level.levelgen.feature.stateproviders.NoiseBasedStateProvider").isInstance(provider)) {
                    Field scale = type("world.level.levelgen.feature.stateproviders.NoiseBasedStateProvider").getDeclaredField("scale"); scale.setAccessible(true);
                    Method noise = declaredMethod(provider.getClass(), "getNoiseValue"); noise.setAccessible(true);
                    sample.put("noise_bits", Long.toUnsignedString(Double.doubleToRawLongBits((double) noise.invoke(provider, nativePos(pos), (double) scale.getFloat(provider)))));
                }
                samples.add(sample);
            }
        }
        return samples;
    }

    public static void main(String[] args) throws Exception {
        bootstrap(args[1]);
        Object result = switch (args[0]) { case "catalog" -> catalog(); case "base" -> baseSamples(); default -> throw new IllegalArgumentException(args[0]); };
        System.out.println("BASE_FEATURE_REFERENCE=" + call(gson, "toJson", result));
    }
}
