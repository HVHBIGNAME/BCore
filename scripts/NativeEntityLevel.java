import java.lang.constant.*;
import java.lang.invoke.MethodHandles;
import java.lang.reflect.*;
import java.util.function.Consumer;

/** Minimal Level for native entity construction; no server, ticks or networking.
 * The subclass supplies the fixture's feature flags and vertical bounds. Native
 * entity constructors still run normally. Its uninitialized server machinery
 * must never be queried; an unexpected query fails the probe.
 */
public class NativeEntityLevel extends TreeReference {
    static Object api(String owner, Object object, String name, Object... args) throws Exception {
        for (Method m : Class.forName(owner).getMethods()) {
            if (m.getName().equals(name) && matches(m.getParameterTypes(),args)) return m.invoke(object,args);
        }
        throw new NoSuchMethodException(owner + "." + name);
    }
    static Object create() throws Exception {
        Object classFile = api("java.lang.classfile.ClassFile",null,"of");
        Consumer<Object> build = builder -> {
            try {
                api("java.lang.classfile.ClassBuilder",builder,"withFlags",1);
                api("java.lang.classfile.ClassBuilder",builder,"withSuperclass",ClassDesc.of("net.minecraft.server.level.ServerLevel"));
                Consumer<Object> flags = code -> {
                    try {
                        api("java.lang.classfile.CodeBuilder",code,"getstatic",ClassDesc.of("net.minecraft.world.flag.FeatureFlags"),"VANILLA_SET",ClassDesc.of("net.minecraft.world.flag.FeatureFlagSet"));
                        api("java.lang.classfile.CodeBuilder",code,"areturn");
                    } catch (Exception e) { throw new RuntimeException(e); }
                };
                api("java.lang.classfile.ClassBuilder",builder,"withMethodBody","enabledFeatures",MethodTypeDesc.of(ClassDesc.of("net.minecraft.world.flag.FeatureFlagSet")),1,flags);
                for (String name : new String[]{"getMinY","getMaxY","getHeight"}) {
                    int value = name.equals("getMinY") ? -64 : name.equals("getMaxY") ? 319 : 384;
                    Consumer<Object> body = code -> {
                        try { api("java.lang.classfile.CodeBuilder",code,"loadConstant",value); api("java.lang.classfile.CodeBuilder",code,"ireturn"); }
                        catch (Exception e) { throw new RuntimeException(e); }
                    };
                    api("java.lang.classfile.ClassBuilder",builder,"withMethodBody",name,MethodTypeDesc.of(ConstantDescs.CD_int),1,body);
                }
            } catch (Exception e) { throw new RuntimeException(e); }
        };
        byte[] bytes = (byte[]) api("java.lang.classfile.ClassFile",classFile,"build",ClassDesc.of("NativeEntityProbeLevel"),build);
        Class<?> level = MethodHandles.lookup().defineClass(bytes);
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        Field field = unsafeClass.getDeclaredField("theUnsafe"); field.setAccessible(true);
        return unsafeClass.getMethod("allocateInstance",Class.class).invoke(field.get(null),level);
    }
}
