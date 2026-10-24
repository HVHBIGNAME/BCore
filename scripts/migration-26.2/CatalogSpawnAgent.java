import java.lang.instrument.*;
import java.security.ProtectionDomain;
import org.objectweb.asm.*;

/** Stop at the original server bootstrap boundary and retain explicit constructor entropy. */
public final class CatalogSpawnAgent implements ClassFileTransformer {
    static final String MC = "net/minecraft/";

    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        GenerationSpawnProbe.initialize(config);
        instrumentation.addTransformer(new CatalogSpawnAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        if (name == null) return null;
        boolean server = name.equals(MC + "server/MinecraftServer");
        boolean entity = name.equals(MC + "world/entity/Entity");
        if (!server && !entity) return null;
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String descriptor,
                                                 String signature, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, descriptor, signature, exceptions);
                    if ((access & Opcodes.ACC_ABSTRACT) != 0) return delegate;
                    boolean bootstrap = server && method.equals("setInitialSpawn");
                    boolean tick = server && method.equals("tickServer");
                    boolean entropy = entity && method.equals("<init>");
                    if (!bootstrap && !tick && !entropy) return delegate;
                    if (bootstrap) {
                        Type[] arguments = Type.getArgumentTypes(descriptor);
                        if (arguments.length == 0 || !arguments[0].getDescriptor().equals("Lnet/minecraft/server/level/ServerLevel;"))
                            throw new IllegalStateException("setInitialSpawn signature changed: " + descriptor);
                    }
                    GenerationSpawnProbe.instrumented(name + "." + method + descriptor);
                    return new MethodVisitor(Opcodes.ASM9, delegate) {
                        @Override public void visitCode() {
                            super.visitCode();
                            if (bootstrap) {
                                super.visitVarInsn(Opcodes.ALOAD, (access & Opcodes.ACC_STATIC) != 0 ? 0 : 1);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "SpawnCatalogReference", "capture", "(Ljava/lang/Object;)V", false);
                            }
                            if (tick) super.visitMethodInsn(Opcodes.INVOKESTATIC, "SpawnCatalogReference", "gameplayTick", "()V", false);
                        }

                        @Override public void visitMethodInsn(int opcode, String owner, String called, String desc, boolean iface) {
                            if (entropy && opcode == Opcodes.INVOKESTATIC && owner.equals(MC + "util/RandomSource")
                                && called.equals("create") && desc.equals("()Lnet/minecraft/util/RandomSource;")) {
                                GenerationSpawnProbe.instrumented(name + "." + method + "#RandomSource.create");
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "GenerationSpawnProbe", "entityRandom", "()Ljava/lang/Object;", false);
                                super.visitTypeInsn(Opcodes.CHECKCAST, MC + "util/RandomSource");
                            } else super.visitMethodInsn(opcode, owner, called, desc, iface);
                        }
                    };
                }
            }, 0);
            return writer.toByteArray();
        } catch (Throwable failure) {
            GenerationSpawnProbe.fatal(failure);
            throw new IllegalStateException("catalog instrumentation failed", failure);
        }
    }
}
