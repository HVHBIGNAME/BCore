import java.io.*;
import java.lang.reflect.Proxy;
import java.nio.file.*;
import java.util.*;

/** Actual ProtoChunk, WorldGenRegion, LevelChunk, native BE and NBT/save calls.
 * The small air chunks/holders are constructed with original constructors; they
 * are not generated terrain and do not claim to exercise ChunkMap scheduling.
 */
public final class LifecycleProbe {
    private static Path output;
    private static Object level, registries, factory, light;
    private static int snapshotId, regionId;
    private static final List<Object> cases = new ArrayList<>();

    static void initialize(String path) { output = Path.of(path); }
    public static void fatal(Throwable error) { error.printStackTrace(); Runtime.getRuntime().halt(3); }
    public static void unexpectedTick() { fatal(new IllegalStateException("unexpected gameplay tick")); }

    private static Object call(Object target, String name, Object... args) throws Exception {
        return NativeAccess.call(target, name, args);
    }
    private static Object make(String name, Object... args) throws Exception { return NativeAccess.make(name, args); }
    private static Object constant(String owner, String name) throws Exception { return NativeAccess.constant(owner, name); }
    private static Object pos(int x, int y, int z) throws Exception { return make("core.BlockPos", x, y, z); }
    private static Object state(String name) throws Exception { return call(constant("world.level.block.Blocks", name), "defaultBlockState"); }
    private static int stateId(Object state) throws Exception { return (int) call(NativeAccess.type("world.level.block.Block"), "getId", state); }
    private static Object tag(String snbt) throws Exception { return call(NativeAccess.type("nbt.TagParser"), "parseCompoundFully", snbt); }
    private static Object copy(Object tag) throws Exception { return tag == null ? null : call(tag, "copy"); }
    private static Map<String, Object> map(Object... pairs) { return NativeAccess.map(pairs); }
    private static List<Integer> xyz(Object p) throws Exception {
        return List.of((int)call(p, "getX"), (int)call(p, "getY"), (int)call(p, "getZ"));
    }
    private static byte[] nbt(Object tag) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        call(NativeAccess.type("nbt.NbtIo"), "write", tag, new DataOutputStream(bytes));
        return bytes.toByteArray();
    }
    private static Object encoded(Object tag) throws Exception {
        return tag == null ? null : map("nbt", Base64.getEncoder().encodeToString(nbt(tag)));
    }

    private record Region(Object world, Map<List<Integer>, Object> chunks, int x) {
        Object owner(Object p) throws Exception { List<Integer> v = xyz(p); return chunks.get(List.of(v.get(0) >> 4, v.get(2) >> 4)); }
        Object position(int offsetX, int y) throws Exception { return pos(x * 16 + offsetX, y, 33); }
        List<Integer> source() { return List.of(x, 2); }
    }

    private static Region region() throws Exception {
        Map<List<Integer>, Object> chunks = new LinkedHashMap<>();
        int sourceX = -2 - 4 * regionId++;
        Object carvers = constant("world.level.chunk.status.ChunkStatus", "CARVERS");
        Class<?> initializer = NativeAccess.type("util.StaticCache2D$Initializer");
        Object init = Proxy.newProxyInstance(initializer.getClassLoader(), new Class<?>[]{initializer}, (p, m, a) -> {
            if (!m.getName().equals("get")) throw new UnsupportedOperationException(m.toString());
            int x = (int)a[0], z = (int)a[1];
            Object cp = make("world.level.ChunkPos", x, z);
            Object chunk = make("world.level.chunk.ProtoChunk", cp,
                constant("world.level.chunk.UpgradeData", "EMPTY"), level, factory, null);
            call(chunk, "setPersistedStatus", carvers);
            chunks.put(List.of(x, z), chunk);
            return holder(chunk);
        });
        Object cache = call(NativeAccess.type("util.StaticCache2D"), "create", sourceX, 2, 2, init);
        Object step = call(constant("world.level.chunk.status.ChunkPyramid", "GENERATION_PYRAMID"),
            "getStepTo", constant("world.level.chunk.status.ChunkStatus", "FEATURES"));
        return new Region(make("server.level.WorldGenRegion", level, cache, step, chunks.get(List.of(sourceX, 2))), chunks, sourceX);
    }

    private static Object holder(Object chunk) throws Exception {
        Object holder = make("server.level.ChunkHolder", call(chunk, "getPos"), 33, level, light, null, null);
        for (String status : List.of("EMPTY", "STRUCTURE_STARTS", "STRUCTURE_REFERENCES", "BIOMES", "NOISE", "SURFACE", "CARVERS"))
            call(holder, "completeFuture", constant("world.level.chunk.status.ChunkStatus", status), chunk);
        return holder;
    }

    private static Object lookupRestored(Object chunk, Object p) throws Exception {
        Object cp = call(chunk, "getPos"), holder = holder(chunk);
        Class<?> initializer = NativeAccess.type("util.StaticCache2D$Initializer");
        Object init = Proxy.newProxyInstance(initializer.getClassLoader(), new Class<?>[]{initializer}, (o, m, a) -> {
            if (!m.getName().equals("get")) throw new UnsupportedOperationException(m.toString());
            return holder;
        });
        Object cache = call(NativeAccess.type("util.StaticCache2D"), "create", call(cp, "x"), call(cp, "z"), 0, init);
        Object step = call(constant("world.level.chunk.status.ChunkPyramid", "GENERATION_PYRAMID"),
            "getStepTo", constant("world.level.chunk.status.ChunkStatus", "FEATURES"));
        return call(make("server.level.WorldGenRegion", level, cache, step, chunk), "getBlockEntity", p);
    }

    private static Map<?, ?> loaded(Object chunk) throws Exception { return (Map<?, ?>)call(chunk, "getBlockEntities"); }
    private static Map<?, ?> pending(Object chunk) throws Exception { return (Map<?, ?>)NativeAccess.field(chunk, "pendingBlockEntities"); }

    /** No getBlockEntity call: observing maps/update tags must not itself promote. */
    private static Object observe(Object chunk, Object p, String boundary) throws Exception {
        Object entity = loaded(chunk).get(p);
        return map("boundary", boundary, "chunk_class", chunk.getClass().getSimpleName(),
            "pos", xyz(p), "state", stateId(call(chunk, "getBlockState", p)),
            "loaded_count", loaded(chunk).size(), "pending_count", pending(chunk).size(),
            "pending", encoded(pending(chunk).get(p)),
            "full", entity == null ? null : encoded(call(entity, "saveWithFullMetadata", registries)),
            "update", entity == null ? null : encoded(call(entity, "getUpdateTag", registries)));
    }

    private static Object save(Object chunk, Object p, String boundary) throws Exception {
        Object saved = call(chunk, "getBlockEntityNbtForSaving", p, registries);
        return map("boundary", boundary, "saved", encoded(saved), "after", observe(chunk, p, boundary + "/after"));
    }

    private static Object diskRoundTrip(Object chunk, Object p) throws Exception {
        Object data = call(NativeAccess.type("world.level.chunk.storage.SerializableChunkData"), "copyOf", level, chunk);
        Object root = call(data, "write");
        String filename = String.format(Locale.ROOT, "chunk-%03d.nbt", snapshotId++);
        Files.write(output.resolve(filename), nbt(root), StandardOpenOption.CREATE_NEW);
        Object parsed = call(NativeAccess.type("world.level.chunk.storage.SerializableChunkData"), "parse", level, factory, root);
        Object info = make("world.level.chunk.storage.RegionStorageInfo", "be-lifecycle", call(level, "dimension"), "chunk");
        Object restored = call(parsed, "read", level, call(level, "getPoiManager"), info, call(chunk, "getPos"));
        Object before = observe(restored, p, "native disk read");
        lookupRestored(restored, p);
        return map("chunk_nbt", filename, "saved_entities", encoded(listRoot(call(root, "get", "block_entities"))),
            "restored", before, "restored_lookup", observe(restored, p, "region lookup after native disk read"));
    }

    private static Object listRoot(Object list) throws Exception {
        Object root = make("nbt.CompoundTag");
        call(root, "put", "block_entities", list);
        return root;
    }

    private static Object packet(Object chunk) throws Exception {
        Object packet = make("network.protocol.game.ClientboundLevelChunkPacketData", chunk);
        List<?> entries = (List<?>) NativeAccess.field(packet, "blockEntitiesData");
        List<Object> tags = new ArrayList<>();
        for (Object entry : entries) {
            Object buffer = make("network.RegistryFriendlyByteBuf", call(Class.forName("io.netty.buffer.Unpooled"), "buffer"), registries);
            call(entry, "write", buffer);
            byte[] bytes = new byte[(int)call(buffer, "writerIndex")];
            call(buffer, "getBytes", 0, bytes); call(buffer, "release");
            tags.add(map("type", call(constant("core.registries.BuiltInRegistries", "BLOCK_ENTITY_TYPE"),
                "getId", NativeAccess.field(entry, "type")), "tag", encoded(NativeAccess.field(entry, "tag")),
                "wire_hex", HexFormat.of().formatHex(bytes)));
        }
        return map("count", entries.size(), "entries", tags);
    }

    private static void basic(String block, int flags) throws Exception {
        Region region = region();
        Object p = region.position(15, 72), s = state(block), chunk = region.owner(p);
        List<Object> observations = new ArrayList<>();
        boolean written = (boolean)call(region.world, "setBlock", p, s, flags, 512);
        observations.add(observe(chunk, p, "write/no lookup"));
        boolean directNull = call(chunk, "getBlockEntity", p) == null;
        observations.add(save(chunk, p, "proto save/no lookup"));
        observations.add(diskRoundTrip(chunk, p));
        Object first = call(region.world, "getBlockEntity", p);
        if (first == null) throw new IllegalStateException(block + " region lookup failed");
        observations.add(observe(chunk, p, "region lookup"));
        boolean same = first == call(region.world, "getBlockEntity", p);
        call(region.world, "setBlock", p, s, flags, 512);
        boolean retained = first == call(region.world, "getBlockEntity", p);
        observations.add(save(chunk, p, "same-state rewrite/loaded save"));
        observations.add(diskRoundTrip(chunk, p));
        call(region.world, "setBlock", p, state("AIR"), flags, 512);
        observations.add(observe(chunk, p, "remove to air"));

        // Independent chunk: conversion alone retains pending, packet omits it,
        // and saving the LevelChunk uses its CHECK lookup to promote it.
        Region convertedRegion = region();
        Object cp = convertedRegion.position(15, 72);
        call(convertedRegion.world, "setBlock", cp, s, flags, 512);
        Object converted = make("world.level.chunk.LevelChunk", level, convertedRegion.owner(cp), null);
        observations.add(observe(converted, cp, "after conversion/no lookup"));
        observations.add(map("boundary", "packet before level save", "packet", packet(converted)));
        observations.add(save(converted, cp, "level save"));
        observations.add(map("boundary", "packet after level save", "packet", packet(converted)));
        observations.add(save(converted, cp, "level save repeated"));
        cases.add(map("name", block + "/flags-" + flags, "kind", "basic", "block", block,
            "source", region.source(), "owner", region.source(), "flags", flags, "written", written,
            "proto_lookup_null", directNull, "lookup_idempotent", same, "loaded_rewrite_retained", retained,
            "observations", observations));
    }

    private static void callback(String block) throws Exception {
        Region region = region();
        Object p = region.position(31, 80), s = state(block), chunk = region.owner(p);
        call(region.world, "setBlock", p, s, 2, 512);
        List<Object> observations = new ArrayList<>();
        observations.add(observe(chunk, p, "source write/no callback"));
        if (block.equals("CHEST") || block.equals("DISPENSER")) {
            Object key = call(NativeAccess.type("resources.ResourceKey"), "create",
                constant("core.registries.Registries", "LOOT_TABLE"),
                call(NativeAccess.type("resources.Identifier"), "parse", "minecraft:chests/jungle_temple"));
            call(NativeAccess.type("world.RandomizableContainer"), "setBlockEntityLootTable", region.world,
                make("world.level.levelgen.LegacyRandomSource", 12345L), p, key);
        } else if (block.equals("BEE_NEST")) {
            Object entity = call(region.world, "getBlockEntity", p);
            for (int ticks : new int[]{0, 599, -7}) call(entity, "storeBee",
                call(NativeAccess.type("world.level.block.entity.BeehiveBlockEntity$Occupant"), "create", ticks));
        } else if (block.equals("SCULK_SENSOR")) call(call(region.world, "getBlockEntity", p), "setLastVibrationFrequency", 9);
        else throw new IllegalArgumentException(block);
        observations.add(save(chunk, p, "source getBlockEntity/mutation"));
        call(region.world, "setBlock", p, s, 18, 512);
        observations.add(save(chunk, p, "rewrite after callback"));
        observations.add(diskRoundTrip(chunk, p));
        Object converted = make("world.level.chunk.LevelChunk", level, chunk, null);
        observations.add(save(converted, p, "converted callback data"));
        cases.add(map("name", block + "/source-callback", "kind", "callback", "block", block,
            "source", region.source(), "owner", List.of(region.x + 1, 2), "flags", 2, "observations", observations));
    }

    private static void template(String block, String loadSnbt) throws Exception {
        Region region = region();
        Object p = region.position(31, 80), chunk = region.owner(p);
        String blockId = "minecraft:" + block.toLowerCase(Locale.ROOT);
        String input = "{size:[1,1,1],palette:[{Name:\"" + blockId + "\"}],blocks:[{pos:[0,0,0],state:0"
            + (loadSnbt == null ? "" : ",nbt:" + loadSnbt) + "}],entities:[]}";
        Object template = make("world.level.levelgen.structure.templatesystem.StructureTemplate");
        call(template, "load", constant("core.registries.BuiltInRegistries", "BLOCK"), tag(input));
        Object settings = make("world.level.levelgen.structure.templatesystem.StructurePlaceSettings");
        call(settings, "setKnownShape", true); call(settings, "setIgnoreEntities", true);
        Object random = make("world.level.levelgen.LegacyRandomSource", 12345L);
        boolean placed = (boolean)call(template, "placeInWorld", region.world, p, p, settings, random, 18);
        List<Object> observations = new ArrayList<>();
        observations.add(observe(chunk, p, "after native template placement"));
        observations.add(save(chunk, p, "template proto save"));
        observations.add(diskRoundTrip(chunk, p));
        Object converted = make("world.level.chunk.LevelChunk", level, chunk, null);
        observations.add(save(converted, p, "template level save"));
        cases.add(map("name", block + "/template-" + (loadSnbt == null ? "absent" : loadSnbt.equals("{}") ? "empty" : "payload"),
            "kind", "template", "block", block, "source", region.source(), "owner", List.of(region.x + 1, 2),
            "flags", 18, "placed", placed, "template_input", encoded(tag(input)), "observations", observations));
    }

    private static void saved(String block) throws Exception {
        Region region = region();
        Object p = region.position(31, 80), chunk = region.owner(p), s = state(block);
        call(chunk, "setBlockState", p, s, 18);
        Object entity = call(call(s, "getBlock"), "newBlockEntity", p, s);
        if (block.equals("CHEST") || block.equals("DISPENSER")) {
            Object key = call(NativeAccess.type("resources.ResourceKey"), "create",
                constant("core.registries.Registries", "LOOT_TABLE"),
                call(NativeAccess.type("resources.Identifier"), "parse", "minecraft:chests/jungle_temple"));
            call(entity, "setLootTable", key, Long.MIN_VALUE + 17);
        } else if (block.equals("SCULK_SENSOR")) call(entity, "setLastVibrationFrequency", 9);
        else if (block.equals("BEE_NEST")) call(entity, "storeBee",
            call(NativeAccess.type("world.level.block.entity.BeehiveBlockEntity$Occupant"), "create", 17));
        Object load = call(entity, "saveWithFullMetadata", registries);
        call(load, "putBoolean", "keepPacked", true);
        call(chunk, "setBlockEntityNbt", load);
        List<Object> observations = new ArrayList<>();
        observations.add(observe(chunk, p, "installed saved pending NBT"));
        observations.add(save(chunk, p, "saved pending proto save"));
        boolean directNull = call(chunk, "getBlockEntity", p) == null;
        lookupRestored(chunk, p);
        observations.add(save(chunk, p, "saved pending region lookup"));
        observations.add(diskRoundTrip(chunk, p));
        cases.add(map("name", block + "/saved-pending", "kind", "saved", "block", block,
            "source", region.source(), "owner", List.of(region.x + 1, 2), "flags", 18,
            "proto_lookup_null", directNull, "observations", observations));
    }

    public static void run(Object serverLevel) {
        try {
            level = serverLevel;
            registries = call(level, "registryAccess");
            factory = call(NativeAccess.type("world.level.chunk.PalettedContainerFactory"), "create", registries);
            light = call(level, "getLightEngine");
            long gameTime = (long)call(level, "getGameTime");
            for (String block : List.of("CHEST", "DISPENSER", "SCULK_SENSOR", "SCULK_CATALYST", "SCULK_SHRIEKER", "BEE_NEST", "BEEHIVE", "COMPARATOR"))
                for (int flags : new int[]{2, 18}) basic(block, flags);
            for (String block : List.of("CHEST", "DISPENSER", "BEE_NEST", "SCULK_SENSOR")) callback(block);
            for (String block : List.of("CHEST", "DISPENSER", "SCULK_SENSOR", "BEE_NEST", "COMPARATOR")) {
                template(block, null);
                template(block, "{}");
            }
            template("CHEST", "{LootTable:\"minecraft:chests/jungle_temple\",LootTableSeed:17L}");
            template("DISPENSER", "{LootTable:\"minecraft:chests/jungle_temple_dispenser\",LootTableSeed:-17L}");
            template("SCULK_SENSOR", "{last_vibration_frequency:9}");
            template("BEE_NEST", "{bees:[{entity_data:{id:\"minecraft:bee\"},ticks_in_hive:17,min_ticks_in_hive:600}]}");
            template("SUSPICIOUS_GRAVEL", "{LootTable:\"minecraft:archaeology/trail_ruins_rare\",LootTableSeed:17L,hit_direction:3b}");
            for (String block : List.of("CHEST", "DISPENSER", "SCULK_SENSOR", "BEE_NEST")) saved(block);
            if ((long)call(level, "getGameTime") != gameTime) throw new IllegalStateException("game time advanced");
            Object gson = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
            gson = call(gson, "serializeNulls"); gson = call(gson, "create");
            String json = (String)call(gson, "toJson", map("schema", 1, "minecraft", "26.1", "protocol", 775,
                "seed", Long.toString((long)call(level, "getSeed")), "game_time", gameTime, "gameplay_ticks", 0,
                "bootstrap", "real ServerLevel at MinecraftServer.setInitialSpawn entry", "cases", cases));
            Files.writeString(output.resolve("observations.raw.json"), json + "\n", StandardOpenOption.CREATE_NEW);
            System.out.println("LIFECYCLE_COMPLETE cases=" + cases.size());
            Runtime.getRuntime().halt(0);
        } catch (Throwable error) { fatal(error); }
    }
}
