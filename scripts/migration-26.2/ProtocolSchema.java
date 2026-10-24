import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.concurrent.*;
import java.util.function.*;
import java.util.stream.Stream;

/** Decode recorded wire bytes with the authenticated target's own stream codecs. */
public final class ProtocolSchema {
    private static final String MC = "net.minecraft.";
    private static final Map<String, Method> METHODS = new HashMap<>();
    private static final Map<String, Object> PROTOCOLS = new HashMap<>();

    private static Class<?> type(String name) throws Exception {
        return Class.forName(MC + name);
    }

    private static Object field(String owner, String name) throws Exception {
        Field selected = type(owner).getDeclaredField(name);
        selected.setAccessible(true);
        return selected.get(null);
    }

    private static boolean compatible(Class<?>[] types, Object[] args) {
        if (types.length != args.length) return false;
        for (int i = 0; i < types.length; i++) {
            Class<?> expected = types[i];
            if (expected == int.class) expected = Integer.class;
            if (expected == long.class) expected = Long.class;
            if (expected == boolean.class) expected = Boolean.class;
            if (expected == float.class) expected = Float.class;
            if (expected == double.class) expected = Double.class;
            if (expected == byte.class) expected = Byte.class;
            if (expected == short.class) expected = Short.class;
            if (expected == char.class) expected = Character.class;
            if (args[i] == null && types[i].isPrimitive()) return false;
            if (args[i] != null && !expected.isInstance(args[i])) return false;
        }
        return true;
    }

    private static Object call(Object target, String name, Object... args) throws Exception {
        Class<?> owner = target instanceof Class<?> c ? c : target.getClass();
        boolean isStatic = target instanceof Class<?>;
        String key = owner.getName() + ":" + isStatic + ":" + name + Arrays.toString(
            Arrays.stream(args).map(a -> a == null ? null : a.getClass()).toArray());
        Method selected = METHODS.get(key);
        if (selected == null) {
            List<Method> matches = new ArrayList<>();
            for (Class<?> current = owner; current != null; current = current.getSuperclass()) {
                for (Method method : current.getDeclaredMethods()) {
                    if (!method.isBridge() && Modifier.isStatic(method.getModifiers()) == isStatic
                            && method.getName().equals(name) && compatible(method.getParameterTypes(), args)) {
                        matches.add(method);
                    }
                }
                if (!matches.isEmpty()) break;
            }
            if (matches.isEmpty()) {
                for (Method method : owner.getMethods()) {
                    if (!method.isBridge() && Modifier.isStatic(method.getModifiers()) == isStatic
                            && method.getName().equals(name) && compatible(method.getParameterTypes(), args)) {
                        matches.add(method);
                    }
                }
            }
            List<Method> candidates = List.copyOf(matches);
            matches.removeIf(method -> candidates.stream().anyMatch(other -> method != other
                && Arrays.equals(method.getParameterTypes(), other.getParameterTypes())
                && method.getDeclaringClass().isAssignableFrom(other.getDeclaringClass())
                && method.getReturnType().isAssignableFrom(other.getReturnType())
                && (method.getDeclaringClass() != other.getDeclaringClass() || method.getReturnType() != other.getReturnType())));
            if (matches.size() != 1) throw new NoSuchMethodException(key + " matched " + matches);
            selected = matches.get(0);
            selected.setAccessible(true);
            METHODS.put(key, selected);
        }
        return selected.invoke(target instanceof Class<?> ? null : target, args);
    }

    private static Object make(String name, Object... args) throws Exception {
        List<Constructor<?>> matches = new ArrayList<>();
        for (Constructor<?> constructor : type(name).getDeclaredConstructors()) {
            if (compatible(constructor.getParameterTypes(), args)) matches.add(constructor);
        }
        if (matches.size() != 1) throw new NoSuchMethodException(name + " constructors matched " + matches.size());
        Constructor<?> selected = matches.get(0);
        selected.setAccessible(true);
        return selected.newInstance(args);
    }

    private static Map<String, String> reflectionContract() throws Exception {
        Map<String, String> result = new TreeMap<>();
        for (String owner : List.of("network.RegistryFriendlyByteBuf", "resources.RegistryDataLoader$NetworkedRegistryData",
                "core.RegistryAccess$ImmutableRegistryAccess", "world.level.chunk.LevelChunkSection")) {
            Class<?>[] parameters = switch (owner) {
                case "network.RegistryFriendlyByteBuf" -> new Class<?>[]{Class.forName("io.netty.buffer.ByteBuf"), type("core.RegistryAccess")};
                case "resources.RegistryDataLoader$NetworkedRegistryData" -> new Class<?>[]{List.class, type("tags.TagNetworkSerialization$NetworkPayload")};
                case "core.RegistryAccess$ImmutableRegistryAccess" -> new Class<?>[]{Stream.class};
                default -> new Class<?>[]{type("world.level.chunk.PalettedContainerFactory")};
            };
            Constructor<?> constructor = type(owner).getDeclaredConstructor(parameters);
            constructor.setAccessible(true);
            result.put(owner + ":<init>", constructor.toGenericString());
        }
        for (String selector : List.of("core.HolderLookup$Provider:listRegistries", "resources.ResourceKey:identifier",
                "resources.RegistryDataLoader$RegistryData:key", "core.RegistryAccess:registries", "core.RegistryAccess:freeze",
                "network.protocol.configuration.ClientboundRegistryDataPacket:registry",
                "network.protocol.configuration.ClientboundRegistryDataPacket:entries",
                "network.protocol.common.ClientboundUpdateTagsPacket:getTags", "core.Registry$PendingTags:apply",
                "core.Registry$PendingTags:size",
                "core.component.DataComponentInitializers$PendingComponents:apply",
                "network.protocol.game.GameProtocols$Context:hasInfiniteMaterials",
                "network.protocol.game.ClientboundLoginPacket:commonPlayerSpawnInfo",
                "network.protocol.game.CommonPlayerSpawnInfo:dimensionType", "core.Holder:value",
                "world.level.dimension.DimensionType:height", "world.level.dimension.DimensionType:minY",
                "world.level.chunk.LevelChunkSection:getStates", "world.level.chunk.LevelChunkSection:getBiomes")) {
            String[] parts = selector.split(":", 2);
            result.put(selector, type(parts[0]).getMethod(parts[1]).toGenericString());
        }
        for (String selector : List.of("core.RegistryAccess:lookup", "core.RegistryAccess:lookupOrThrow")) {
            String[] parts = selector.split(":", 2);
            result.put(selector, type(parts[0]).getMethod(parts[1], type("resources.ResourceKey")).toGenericString());
        }
        result.put("RegistryDataLoader:load", type("resources.RegistryDataLoader").getMethod("load", Map.class,
            type("server.packs.resources.ResourceProvider"), List.class, List.class, Executor.class).toGenericString());
        result.put("PalettedContainerFactory:create", type("world.level.chunk.PalettedContainerFactory")
            .getMethod("create", type("core.RegistryAccess")).toGenericString());
        result.put("LevelChunkSection:read", type("world.level.chunk.LevelChunkSection")
            .getMethod("read", type("network.FriendlyByteBuf")).toGenericString());
        result.put("DataComponentInitializers:build", type("core.component.DataComponentInitializers")
            .getMethod("build", type("core.HolderLookup$Provider")).toGenericString());
        result.put("NetworkPayload:resolve", type("tags.TagNetworkSerialization$NetworkPayload")
            .getMethod("resolve", type("core.Registry")).toGenericString());
        result.put("Registry:prepareTagReload", type("core.Registry")
            .getMethod("prepareTagReload", type("tags.TagLoader$LoadResult")).toGenericString());
        result.put("TagLoader:buildUpdatedLookups", type("tags.TagLoader")
            .getMethod("buildUpdatedLookups", type("core.RegistryAccess$Frozen"), List.class).toGenericString());
        return result;
    }

    private static Object parseJson(String text) throws Exception {
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        return gson.getClass().getMethod("fromJson", String.class, Class.class).invoke(gson, text, Map.class);
    }

    private static void writeJson(Path path, Object value) throws Exception {
        Object builder = Class.forName("com.google.gson.GsonBuilder").getConstructor().newInstance();
        Object gson = call(call(builder, "setPrettyPrinting"), "create");
        Files.writeString(path, call(gson, "toJson", value) + "\n", StandardOpenOption.CREATE_NEW);
    }

    private static String sha(byte[] bytes) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }

    private static Object proxy(String name, BiFunction<Method, Object[], Object> action) throws Exception {
        return Proxy.newProxyInstance(ProtocolSchema.class.getClassLoader(), new Class<?>[]{type(name)},
            (target, method, args) -> action.apply(method, args));
    }

    private static Object template(String state, String direction) throws Exception {
        String owner = switch (state) {
            case "handshake" -> "handshake.HandshakeProtocols";
            case "status" -> "status.StatusProtocols";
            case "login" -> "login.LoginProtocols";
            case "configuration" -> "configuration.ConfigurationProtocols";
            case "play" -> "game.GameProtocols";
            default -> throw new IllegalArgumentException("unknown state " + state);
        };
        return field("network.protocol." + owner, direction.toUpperCase(Locale.ROOT) + "_TEMPLATE");
    }

    private static Map<String, Object> packetIds(String state, String direction) throws Exception {
        Map<String, Object> rows = new TreeMap<>();
        Object visitor = proxy("network.ProtocolInfo$Details$PacketVisitor", (method, args) -> {
            if (!method.getName().equals("accept")) throw new UnsupportedOperationException(method.toString());
            try {
                rows.put(call(args[0], "id").toString(), Map.of("protocol_id", args[1]));
                return null;
            } catch (Exception error) { throw new IllegalStateException(error); }
        });
        call(call(template(state, direction), "details"), "listPackets", visitor);
        return rows;
    }

    private static Map<String, Object> inventory() throws Exception {
        Map<String, Object> result = new TreeMap<>();
        for (String state : List.of("handshake", "status", "login", "configuration", "play")) {
            Map<String, Object> directions = new TreeMap<>();
            for (String direction : state.equals("handshake") ? List.of("serverbound") : List.of("clientbound", "serverbound")) {
                directions.put(direction, packetIds(state, direction));
            }
            result.put(state, directions);
        }
        return result;
    }

    private static List<String> synchronizedRegistries() throws Exception {
        List<String> names = new ArrayList<>();
        for (Object definition : (List<?>) field("resources.RegistryDataLoader", "SYNCHRONIZED_REGISTRIES")) {
            names.add(call(call(definition, "key"), "identifier").toString());
        }
        return names;
    }

    private static Object protocol(String state, String direction, Object registries) throws Exception {
        String key = state + ":" + direction;
        Object existing = PROTOCOLS.get(key);
        if (existing != null) return existing;
        Object info;
        if (state.equals("play")) {
            Object decorator = call(type("network.RegistryFriendlyByteBuf"), "decorator", registries);
            if (direction.equals("clientbound")) {
                info = call(template(state, direction), "bind", decorator);
            } else {
                Object context = proxy("network.protocol.game.GameProtocols$Context", (method, args) -> {
                    if (!method.getName().equals("hasInfiniteMaterials")) throw new UnsupportedOperationException(method.toString());
                    return false;
                });
                info = call(template(state, direction), "bind", decorator, context);
            }
        } else {
            String owner = switch (state) {
                case "handshake" -> "handshake.HandshakeProtocols";
                case "status" -> "status.StatusProtocols";
                case "login" -> "login.LoginProtocols";
                case "configuration" -> "configuration.ConfigurationProtocols";
                default -> throw new IllegalArgumentException(state);
            };
            info = field("network.protocol." + owner, direction.toUpperCase(Locale.ROOT));
        }
        PROTOCOLS.put(key, info);
        return info;
    }

    private record Decoded(Object packet, Map<String, Object> report) {}

    private static Decoded decode(byte[] raw, String state, String direction, Object registries) throws Exception {
        Object buffer = call(Class.forName("io.netty.buffer.Unpooled"), "wrappedBuffer", raw);
        try {
            Object info = protocol(state, direction, registries);
            Object stream = state.equals("play") ? make("network.RegistryFriendlyByteBuf", buffer, registries) : buffer;
            Object packet = call(call(info, "codec"), "decode", stream);
            int remaining = (int) call(buffer, "readableBytes");
            if (remaining != 0) throw new IllegalStateException("native decoder left " + remaining + " bytes");
            String name = call(call(packet, "type"), "id").toString();
            Map<String, Object> report = new TreeMap<>();
            report.put("packet", name);
            report.put("class", packet.getClass().getName());
            report.put("consumed_bytes", raw.length);
            report.put("trailing_bytes", remaining);
            return new Decoded(packet, report);
        } finally {
            call(buffer, "release");
        }
    }

    private static byte[] unframe(byte[] framed) throws IOException {
        int length = 0, prefix = 0;
        for (; prefix < 3 && prefix < framed.length; prefix++) {
            int value = framed[prefix] & 255;
            length |= (value & 127) << (7 * prefix);
            if ((value & 128) == 0) {
                prefix++;
                if (length < 1 || length != framed.length - prefix || prefix > 1 && length < (1 << (7 * (prefix - 1)))) {
                    throw new IOException("invalid or noncanonical frame length");
                }
                return Arrays.copyOfRange(framed, prefix, framed.length);
            }
        }
        throw new IOException("invalid frame length");
    }

    private static byte[] recordedFrame(Path root, Map<?, ?> event) throws Exception {
        String name = (String) event.get("file");
        Path path = root.resolve(name).normalize();
        if (!path.startsWith(root) || Files.isSymbolicLink(path)) throw new IOException("frame path escapes capture");
        long offset = ((Number) event.get("offset")).longValue();
        int length = ((Number) event.get("frame_length")).intValue();
        if (offset < 0 || length < 1 || length > 2097154) throw new IOException("frame range outside budget");
        byte[] raw = new byte[length];
        try (RandomAccessFile file = new RandomAccessFile(path.toFile(), "r")) {
            file.seek(offset);
            file.readFully(raw);
        }
        if (!sha(raw).equals(event.get("frame_sha256"))) throw new IOException("recorded frame digest differs");
        return unframe(raw);
    }

    private static Object loadRegistries(List<Object> packets, Object tags, Executor executor,
            Map<String, Object> report) throws Exception {
        Map<Object, Object> network = new HashMap<>();
        @SuppressWarnings("unchecked")
        Map<Object, Object> tagMap = (Map<Object, Object>) call(tags, "getTags");
        for (Object packet : packets) {
            Object key = call(packet, "registry");
            Object payload = tagMap.getOrDefault(key, field("tags.TagNetworkSerialization$NetworkPayload", "EMPTY"));
            List<?> entries = (List<?>) call(packet, "entries");
            for (Object entry : entries) {
                if (((Optional<?>) call(entry, "data")).isEmpty()) throw new IOException("registry data was omitted");
            }
            Object value = make("resources.RegistryDataLoader$NetworkedRegistryData", entries, payload);
            if (network.put(key, value) != null) throw new IOException("duplicate registry packet");
        }
        Set<String> received = new TreeSet<>();
        for (Object key : network.keySet()) received.add(call(key, "identifier").toString());
        if (!received.equals(new TreeSet<>(synchronizedRegistries()))) {
            throw new IOException("synchronized registry membership differs: " + received);
        }
        Object builtins = call(type("core.RegistryAccess"), "fromRegistryOfRegistries", field("core.registries.BuiltInRegistries", "REGISTRY"));
        List<Object> pendingTags = new ArrayList<>();
        Map<String, Integer> preparedTags = new TreeMap<>();
        for (var entry : tagMap.entrySet()) {
            Optional<?> registry = (Optional<?>) call(builtins, "lookup", entry.getKey());
            if (registry.isPresent()) {
                Object pending = call(registry.get(), "prepareTagReload", call(entry.getValue(), "resolve", registry.get()));
                pendingTags.add(pending);
                preparedTags.put(call(entry.getKey(), "identifier").toString(), (int) call(pending, "size"));
            }
        }
        // Network registry codecs resolve built-in tag references through these pending lookups.
        Object contexts = call(type("tags.TagLoader"), "buildUpdatedLookups", builtins, pendingTags);
        report.put("builtin_tag_context", Map.of("source", "recorded_update_tags", "registries", preparedTags,
            "lookup_api", "TagLoader.buildUpdatedLookups", "applied", false));
        Object future = call(type("resources.RegistryDataLoader"), "load", network,
            field("server.packs.resources.ResourceProvider", "EMPTY"), contexts,
            field("resources.RegistryDataLoader", "SYNCHRONIZED_REGISTRIES"), executor);
        Object loaded = ((CompletableFuture<?>) future).get(120, TimeUnit.SECONDS);
        List<Object> all = new ArrayList<>();
        try (Stream<?> stream = (Stream<?>) call(builtins, "registries")) { all.addAll(stream.toList()); }
        try (Stream<?> stream = (Stream<?>) call(loaded, "registries")) { all.addAll(stream.toList()); }
        Object combined = call(make("core.RegistryAccess$ImmutableRegistryAccess", all.stream()), "freeze");
        for (Object pending : pendingTags) call(pending, "apply");
        report.put("builtin_tag_context", Map.of("source", "recorded_update_tags", "registries", preparedTags,
            "lookup_api", "TagLoader.buildUpdatedLookups", "applied", true));
        // Packet item stacks may refer to default components initialized from these exact registries.
        for (Object pending : (List<?>) call(field("core.registries.BuiltInRegistries", "DATA_COMPONENT_INITIALIZERS"), "build", combined)) {
            call(pending, "apply");
        }
        return combined;
    }

    private static int[] countContainer(Object container, boolean blocks, Object biomeRegistry) throws Exception {
        int[] result = new int[2];
        Object consumer = proxy("world.level.chunk.PalettedContainer$CountConsumer", (method, args) -> {
            if (!method.getName().equals("accept")) throw new UnsupportedOperationException(method.toString());
            try {
                Object value = Objects.requireNonNull(args[0], "unknown palette entry");
                int count = (int) args[1];
                if (blocks) {
                    if (!(boolean) call(value, "isAir")) result[0] += count;
                    if (!(boolean) call(call(value, "getFluidState"), "isEmpty")) result[1] += count;
                    if ((int) call(type("world.level.block.Block"), "getId", value) < 0) throw new IOException("unknown block state");
                } else if ((int) call(biomeRegistry, "getId", call(value, "value")) < 0) {
                    throw new IOException("unknown biome holder");
                }
                return null;
            } catch (Exception error) { throw new IllegalStateException(error); }
        });
        call(container, "count", consumer);
        return result;
    }

    private static Map<String, Object> terrain(Object packet, Object registries, Object dimensionType) throws Exception {
        int height = (int) call(dimensionType, "height"), minY = (int) call(dimensionType, "minY");
        if (height < 16 || height > 4096 || height % 16 != 0 || minY % 16 != 0) throw new IOException("invalid dimension geometry");
        Object chunkData = call(packet, "getChunkData");
        Object buffer = call(chunkData, "getReadBuffer");
        int blocks = 0;
        try {
            Object factory = call(type("world.level.chunk.PalettedContainerFactory"), "create", registries);
            Object biomes = call(registries, "lookupOrThrow", field("core.registries.Registries", "BIOME"));
            for (int index = 0; index < height / 16; index++) {
                int at = (int) call(buffer, "readerIndex");
                int expectedBlocks = ((Number) call(buffer, "getShort", at)).intValue();
                int expectedFluids = ((Number) call(buffer, "getShort", at + 2)).intValue();
                Object section = make("world.level.chunk.LevelChunkSection", factory);
                call(section, "read", buffer);
                int[] actual = countContainer(call(section, "getStates"), true, biomes);
                countContainer(call(section, "getBiomes"), false, biomes);
                if (actual[0] != expectedBlocks || actual[1] != expectedFluids) {
                    throw new IOException("section " + index + " native block/fluid counts differ: " + Arrays.toString(actual)
                        + " vs " + expectedBlocks + "/" + expectedFluids);
                }
                blocks += actual[0];
            }
            if ((int) call(buffer, "readableBytes") != 0) throw new IOException("trailing chunk-section bytes");
        } finally {
            call(buffer, "release");
        }
        Object light = call(packet, "getLightData");
        for (String kind : List.of("Sky", "Block")) {
            BitSet present = (BitSet) call(light, "get" + kind + "YMask");
            BitSet empty = (BitSet) call(light, "getEmpty" + kind + "YMask");
            BitSet overlap = (BitSet) present.clone();
            overlap.and(empty);
            List<?> updates = (List<?>) call(light, "get" + kind + "Updates");
            if (!overlap.isEmpty() || present.length() > height / 16 + 2 || empty.length() > height / 16 + 2
                    || updates.size() != present.cardinality()) throw new IOException("invalid native light mask/count");
            for (Object update : updates) if (((byte[]) update).length != 2048) throw new IOException("invalid light layer");
        }
        return Map.of("x", call(packet, "getX"), "z", call(packet, "getZ"), "sections", height / 16,
            "min_y", minY, "non_air_blocks", blocks, "native_section_decode", true,
            "native_block_fluid_counts_match", true, "native_biome_ids_resolve", true);
    }

    private static Map<String, Object> verify(Path root, Map<String, Object> ids) throws Exception {
        @SuppressWarnings("unchecked")
        List<Map<String, Object>> events = (List<Map<String, Object>>) ((Map<?, ?>) parseJson(Files.readString(root.resolve("wire.json")))).get("events");
        List<Map<String, Object>> reports = new ArrayList<>();
        List<Object> registries = new ArrayList<>();
        Object tags = null, loaded = null, dimensionType = null;
        Map<String, Integer> states = new TreeMap<>();
        int decodedCount = 0, terrainCount = 0;
        boolean loginFinished = false, loginAcknowledged = false, configurationAcknowledged = false;
        boolean statusResponse = false, statusPong = false;
        String failure = null;
        ExecutorService executor = Executors.newFixedThreadPool(2);
        try {
            PROTOCOLS.clear();
            for (Map<String, Object> event : events) {
                String state = (String) event.get("state"), direction = (String) event.get("direction");
                Map<String, Object> report = new TreeMap<>();
                report.put("sequence", event.get("sequence"));
                report.put("state", state);
                report.put("direction", direction);
                report.put("packet_id", event.get("packet_id"));
                if (event.containsKey("connection")) report.put("connection", event.get("connection"));
                String phase = "packet_decode";
                try {
                    Decoded decoded = decode(recordedFrame(root, event), state, direction, loaded);
                    report.putAll(decoded.report());
                    @SuppressWarnings("unchecked")
                    Map<String, Map<String, Number>> mapping = (Map<String, Map<String, Number>>) ((Map<?, ?>) ids.get(state)).get(direction);
                    Map<String, Number> expected = mapping.get(decoded.report().get("packet"));
                    if (expected == null || expected.get("protocol_id").intValue()
                            != ((Number) event.get("packet_id")).intValue()) throw new IOException("packet identity differs from native inventory");
                    if (direction.equals("clientbound") && state.equals("configuration")) {
                        if (decoded.packet().getClass().getSimpleName().equals("ClientboundRegistryDataPacket")) registries.add(decoded.packet());
                        if (decoded.packet().getClass().getSimpleName().equals("ClientboundUpdateTagsPacket")) tags = decoded.packet();
                        if (decoded.packet().getClass().getSimpleName().equals("ClientboundFinishConfigurationPacket")) {
                            phase = "network_registry_load";
                            if (tags == null) throw new IOException("configuration has no tags");
                            loaded = loadRegistries(registries, tags, executor, report);
                            report.put("native_network_registry_load", true);
                        }
                    }
                    if (direction.equals("clientbound") && state.equals("play")) {
                        if (decoded.packet().getClass().getSimpleName().equals("ClientboundLoginPacket")) {
                            dimensionType = call(call(call(decoded.packet(), "commonPlayerSpawnInfo"), "dimensionType"), "value");
                        }
                        if (decoded.packet().getClass().getSimpleName().equals("ClientboundLevelChunkWithLightPacket")) {
                            phase = "native_terrain_decode";
                            if (dimensionType == null) throw new IOException("chunk before native play login");
                            report.put("terrain", terrain(decoded.packet(), loaded, dimensionType));
                            terrainCount++;
                        }
                    }
                    String packet = (String) decoded.report().get("packet");
                    loginFinished |= state.equals("login") && direction.equals("clientbound") && packet.equals("minecraft:login_finished");
                    loginAcknowledged |= state.equals("login") && direction.equals("serverbound") && packet.equals("minecraft:login_acknowledged");
                    configurationAcknowledged |= state.equals("configuration") && direction.equals("serverbound") && packet.equals("minecraft:finish_configuration");
                    statusResponse |= state.equals("status") && direction.equals("clientbound") && packet.equals("minecraft:status_response");
                    statusPong |= state.equals("status") && direction.equals("clientbound") && packet.equals("minecraft:pong_response");
                    report.put("status", "pass");
                    decodedCount++;
                    states.merge(state + ":" + direction, 1, Integer::sum);
                } catch (Exception error) {
                    Throwable cause = error;
                    while (cause.getCause() != null) cause = cause.getCause();
                    failure = cause.getClass().getName() + ": " + cause.getMessage();
                    report.put("status", "fail");
                    report.put("error", failure);
                    report.put("failure_phase", phase);
                }
                reports.add(report);
                if (failure != null) break;
            }
        } finally {
            executor.shutdownNow();
            if (!executor.awaitTermination(5, TimeUnit.SECONDS)) throw new IOException("native registry executor cleanup unknown");
        }
        Map<String, Object> result = new TreeMap<>();
        result.put("status", failure == null && decodedCount == events.size() && !events.isEmpty() ? "pass" : "fail");
        result.put("decoded", decodedCount);
        result.put("processed", reports.size());
        result.put("expected", events.size());
        result.put("coverage", Map.of("states", states, "login_finished", loginFinished,
            "login_acknowledged", loginAcknowledged, "network_registry_load", loaded != null,
            "configuration_acknowledged", configurationAcknowledged, "play_login", dimensionType != null,
            "native_terrain_packets", terrainCount, "status_response", statusResponse, "status_pong", statusPong));
        result.put("packets", reports);
        result.put("error", failure == null ? (events.isEmpty() ? "capture has no recorded frames" : "") : failure);
        result.put("capture_path", root.toString());
        result.put("wire_index_sha256", sha(Files.readAllBytes(root.resolve("wire.json"))));
        result.put("authority", "26.2 native StreamCodec; registry/terrain validation only where coverage records it");
        result.put("validation_scope", "recorded_frames_only");
        result.put("actual_full_client", false);
        return result;
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 2) throw new IllegalArgumentException("inventory OUT | verify OUT SESSION...");
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        Map<String, Object> ids = inventory();
        Map<String, Object> result = new TreeMap<>();
        result.put("minecraft", "26.2");
        result.put("protocol", 776);
        result.put("packets", ids);
        result.put("synchronized_registries", synchronizedRegistries());
        result.put("reflection_contract", reflectionContract());
        if (args[0].equals("verify")) {
            List<Object> sessions = new ArrayList<>();
            for (int index = 2; index < args.length; index++) sessions.add(verify(Path.of(args[index]).toRealPath(), ids));
            result.put("sessions", sessions);
        } else if (!args[0].equals("inventory")) {
            throw new IllegalArgumentException(args[0]);
        }
        writeJson(Path.of(args[1]), result);
    }
}
