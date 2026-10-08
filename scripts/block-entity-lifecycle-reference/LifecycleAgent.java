import java.lang.instrument.*;
import java.security.ProtectionDomain;
import org.objectweb.asm.*;

/** Enter a real ServerLevel before spawn search; no native BE method is replaced. */
public final class LifecycleAgent implements ClassFileTransformer {
    public static void premain(String output, Instrumentation instrumentation) throws Exception {
        LifecycleProbe.initialize(output);
        instrumentation.addTransformer(new LifecycleAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        boolean server = name.equals("net/minecraft/server/MinecraftServer");
        boolean level = name.equals("net/minecraft/server/level/ServerLevel");
        if (!server && !level) return null;
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            reader.accept(new ClassVisitor(Opcodes.ASM8, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String descriptor,
                                                 String signature, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, descriptor, signature, exceptions);
                    boolean boundary = server && method.equals("setInitialSpawn");
                    boolean tick = server && method.equals("tickServer") || level && method.equals("tick");
                    if (!boundary && !tick) return delegate;
                    return new MethodVisitor(Opcodes.ASM8, delegate) {
                        @Override public void visitCode() {
                            super.visitCode();
                            if (boundary) {
                                // setInitialSpawn is static; argument zero is the original ServerLevel.
                                visitVarInsn(Opcodes.ALOAD, 0);
                                visitMethodInsn(Opcodes.INVOKESTATIC, "LifecycleProbe", "run",
                                    "(Ljava/lang/Object;)V", false);
                            } else {
                                visitMethodInsn(Opcodes.INVOKESTATIC, "LifecycleProbe", "unexpectedTick", "()V", false);
                            }
                        }
                    };
                }
            }, 0);
            return writer.toByteArray();
        } catch (Throwable error) {
            LifecycleProbe.fatal(error);
            return null;
        }
    }
}
