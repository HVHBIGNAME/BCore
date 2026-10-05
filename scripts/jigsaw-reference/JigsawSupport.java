import java.io.*;
import java.lang.reflect.*;
import java.util.*;

/** Reflection-only native harness; the implementation under test lives in the JAR. */
public class JigsawSupport extends TreeReference {
    static Object registries, resources, templateSource, templateManager, blockRegistry;

    static Object identifier(String name) throws Exception {
        return call(type("resources.Identifier"), "parse", name);
    }

    static Object key(String registry, String name) throws Exception {
        return call(type("resources.ResourceKey"), "create", field("core.registries.Registries", registry), identifier(name));
    }

    static Object registry(String name) throws Exception {
        return call(registries, "lookupOrThrow", field("core.registries.Registries", name));
    }

    static Object member(Object object, String name) throws Exception {
        for (Class<?> c = object.getClass(); c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f.get(object);
            } catch (NoSuchFieldException ignored) { }
        }
        throw new NoSuchFieldException(name);
    }

    static void setMember(Object object, String name, Object value) throws Exception {
        Field f = object.getClass().getDeclaredField(name);
        f.setAccessible(true);
        f.set(object, value);
    }

    static Object allocate(Class<?> type) throws Exception {
        Class<?> u = Class.forName("sun.misc.Unsafe");
        Field f = u.getDeclaredField("theUnsafe");
        f.setAccessible(true);
        return u.getMethod("allocateInstance", Class.class).invoke(f.get(null), type);
    }

    static void bootstrap() throws Exception {
        call(type("SharedConstants"), "tryDetectVersion");
        call(type("server.Bootstrap"), "bootStrap");
        registries = NativeWorldgenRegistries.load();
        blockRegistry = field("core.registries.BuiltInRegistries", "BLOCK");
        Object pack = call(type("server.packs.repository.ServerPacksSource"), "createVanillaPackSource");
        resources = make("server.packs.resources.MultiPackResourceManager", field("server.packs.PackType", "SERVER_DATA"), List.of(pack));
        templateSource = make("world.level.levelgen.structure.templatesystem.loader.ResourceManagerTemplateSource",
            call(type("util.datafix.DataFixers"), "getDataFixer"), blockRegistry, resources,
            make("resources.FileToIdConverter", "structure", ".nbt"));
        // The manager's filesystem paths are intentionally absent. Only its real
        // get/getOrCreate cache and the native resource-pack source are exercised.
        templateManager = allocate(type("world.level.levelgen.structure.templatesystem.StructureTemplateManager"));
        setMember(templateManager, "structureRepository", new HashMap<>());
        setMember(templateManager, "sources", List.of(templateSource));
        setMember(templateManager, "resourceManagerSource", templateSource);
    }

    static Object template(String name) throws Exception {
        return ((Optional<?>) call(templateManager, "get", identifier(name))).orElseThrow();
    }

    static String nbt64(Object nbt) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        call(type("nbt.NbtIo"), "write", nbt, new DataOutputStream(bytes));
        return Base64.getEncoder().encodeToString(bytes.toByteArray());
    }

    static Object jsonValue(Object tag) throws Exception {
        Object ops = Class.forName("com.mojang.serialization.JsonOps").getField("INSTANCE").get(null);
        Object json = call(field("nbt.NbtOps", "INSTANCE"), "convertTo", ops, tag);
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        return call(gson, "fromJson", json.toString(), Object.class);
    }

    static int stateId(Object state) throws Exception {
        return (int) call(type("world.level.block.Block"), "getId", state);
    }

    static List<Integer> xyz(Object pos) throws Exception {
        return List.of((int) call(pos, "getX"), (int) call(pos, "getY"), (int) call(pos, "getZ"));
    }

    static List<Integer> bounds(Object box) throws Exception {
        List<Integer> result = new ArrayList<>();
        for (String method : List.of("minX", "minY", "minZ", "maxX", "maxY", "maxZ")) result.add((int) call(box, method));
        return result;
    }

    static void output(String marker, Object result) throws Exception {
        Object gson = Class.forName("com.google.gson.Gson").getConstructor().newInstance();
        System.out.println(marker + "=" + call(gson, "toJson", result));
    }
}
