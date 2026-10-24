import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;

/** Observe location predicates at the initialized server boundary before ticks. */
public final class LootLocationAgent implements ClassFileTransformer {
    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        LootCatalogReference.initialize(config);
        instrumentation.addTransformer(new LootLocationAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        if (!"net/minecraft/server/MinecraftServer".equals(name)
                && !"net/minecraft/server/level/ServerLevel".equals(name)) return null;
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
                    boolean boundary = name.endsWith("/MinecraftServer") && method.equals("setInitialSpawn");
                    boolean tick = method.equals("tickServer") || name.endsWith("/ServerLevel") && method.equals("tick");
                    if (!boundary && !tick) return delegate;
                    if (boundary && (Type.getArgumentTypes(descriptor).length == 0
                            || !Type.getArgumentTypes(descriptor)[0].getDescriptor().equals("Lnet/minecraft/server/level/ServerLevel;")))
                        throw new IllegalStateException("setInitialSpawn signature changed: " + descriptor);
                    changed[0] = true;
                    return new MethodVisitor(Opcodes.ASM9, delegate) {
                        @Override public void visitCode() {
                            super.visitCode();
                            if (boundary) {
                                super.visitVarInsn(Opcodes.ALOAD, (access & Opcodes.ACC_STATIC) != 0 ? 0 : 1);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootLocationReference", "capture", "(Ljava/lang/Object;)V", false);
                            } else {
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootProbe", "unexpectedTick", "()V", false);
                            }
                        }
                    };
                }
            }, 0);
            return changed[0] ? writer.toByteArray() : null;
        } catch (Throwable failure) {
            LootProbe.fatal(failure);
            return null;
        }
    }
}
