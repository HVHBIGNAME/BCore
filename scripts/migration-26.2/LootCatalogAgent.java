import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;

/** Versioned server boundary with the original optional loot observation hooks. */
public final class LootCatalogAgent implements ClassFileTransformer {
    private static final String MC = "net/minecraft/";
    private final LootAgent observations = new LootAgent();

    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        LootCatalogReference.initialize(config);
        instrumentation.addTransformer(new LootCatalogAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        if (name == null) return null;
        boolean server = name.equals(MC + "server/MinecraftServer");
        boolean level = name.equals(MC + "server/level/ServerLevel");
        if (!server && !level) {
            return LootCatalogReference.fillsEnabled()
                ? observations.transform(loader, name, redefined, domain, bytes) : null;
        }
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            boolean[] changed = {false};
            reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String descriptor,
                                                 String signature, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, descriptor, signature, exceptions);
                    if ((access & (Opcodes.ACC_ABSTRACT | Opcodes.ACC_NATIVE)) != 0) return delegate;
                    boolean boundary = server && method.equals("setInitialSpawn");
                    boolean tick = server && method.equals("tickServer") || level && method.equals("tick");
                    if (!boundary && !tick) return delegate;
                    if (boundary) {
                        Type[] arguments = Type.getArgumentTypes(descriptor);
                        if (arguments.length == 0
                                || !arguments[0].getDescriptor().equals("Lnet/minecraft/server/level/ServerLevel;")) {
                            throw new IllegalStateException("setInitialSpawn signature changed: " + descriptor);
                        }
                    }
                    changed[0] = true;
                    LootProbe.instrumented(name.substring(MC.length()) + "." + method + descriptor);
                    return new MethodVisitor(Opcodes.ASM9, delegate) {
                        @Override public void visitCode() {
                            super.visitCode();
                            if (boundary) {
                                super.visitVarInsn(Opcodes.ALOAD, (access & Opcodes.ACC_STATIC) != 0 ? 0 : 1);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootCatalogReference", "capture",
                                    "(Ljava/lang/Object;)V", false);
                            } else {
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootProbe", "unexpectedTick", "()V", false);
                            }
                        }
                    };
                }
            }, 0);
            if (!changed[0]) return null;
            byte[] transformed = writer.toByteArray();
            LootProbe.transformed(name, bytes, transformed);
            return transformed;
        } catch (Throwable failure) {
            LootProbe.fatal(failure);
            return null;
        }
    }
}
