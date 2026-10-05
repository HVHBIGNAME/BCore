import java.lang.instrument.*;
import java.security.ProtectionDomain;
import java.util.*;
import org.objectweb.asm.*;

/** Observe actual methods; replace only the explicit entity entropy input. */
public final class GenerationSpawnAgent implements ClassFileTransformer {
    static final String HOOK = "GenerationSpawnProbe";
    static final String MC = "net/minecraft/";
    static final Map<String, Set<String>> METHODS = Map.ofEntries(
        Map.entry("server/MinecraftServer", Set.of("setInitialSpawn", "tickServer")),
        Map.entry("world/level/NaturalSpawner", Set.of("spawnMobsForChunkGeneration", "getTopNonCollidingPos")),
        Map.entry("world/entity/EntityType", Set.of("create")),
        Map.entry("world/entity/SpawnPlacements", Set.of("isSpawnPositionOk", "checkSpawnRules")),
        Map.entry("world/entity/Mob", Set.of("checkSpawnRules", "checkSpawnObstruction", "finalizeSpawn"))
        ,Map.entry("world/entity/PathfinderMob", Set.of("checkSpawnRules"))
    );

    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        GenerationSpawnProbe.initialize(config);
        instrumentation.addTransformer(new GenerationSpawnAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        if (!name.startsWith(MC)) return null;
        String owner = name.substring(MC.length());
        boolean entity = owner.equals("world/entity/Entity");
        if (!entity && !METHODS.containsKey(owner) && !owner.startsWith("world/entity/animal/")) return null;
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String descriptor,
                                                 String signature, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, descriptor, signature, exceptions);
                    boolean entropy = entity && method.equals("<init>");
                    boolean observed = METHODS.getOrDefault(owner, Set.of()).contains(method)
                        || owner.startsWith("world/entity/animal/") && Set.of("finalizeSpawn", "checkSpawnRules", "checkSpawnObstruction").contains(method);
                    if ((!observed && !entropy) || (access & Opcodes.ACC_ABSTRACT) != 0) return delegate;
                    GenerationSpawnProbe.instrumented(owner + "." + method + descriptor);
                    return new MethodVisitor(Opcodes.ASM9, delegate) {
                        final boolean isStatic = (access & Opcodes.ACC_STATIC) != 0;
                        final Type result = Type.getReturnType(descriptor);
                        void box(Type type) {
                            String wrapper = switch(type.getSort()) {
                                case Type.BOOLEAN -> "Boolean"; case Type.INT -> "Integer";
                                case Type.FLOAT -> "Float"; case Type.LONG -> "Long";
                                case Type.DOUBLE -> "Double"; case Type.BYTE -> "Byte";
                                case Type.SHORT -> "Short"; case Type.CHAR -> "Character";
                                default -> null;
                            };
                            if (wrapper != null) super.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/" + wrapper,
                                "valueOf", "(" + type.getDescriptor() + ")Ljava/lang/" + wrapper + ";", false);
                        }
                        void context() {
                            super.visitLdcInsn(owner + "." + method);
                            if (isStatic) super.visitInsn(Opcodes.ACONST_NULL);
                            else super.visitVarInsn(Opcodes.ALOAD, 0);
                            Type[] args = Type.getArgumentTypes(descriptor);
                            super.visitLdcInsn(args.length);
                            super.visitTypeInsn(Opcodes.ANEWARRAY, "java/lang/Object");
                            int local = isStatic ? 0 : 1;
                            for (int i = 0; i < args.length; i++) {
                                super.visitInsn(Opcodes.DUP);
                                super.visitLdcInsn(i);
                                super.visitVarInsn(args[i].getOpcode(Opcodes.ILOAD), local);
                                box(args[i]);
                                super.visitInsn(Opcodes.AASTORE);
                                local += args[i].getSize();
                            }
                        }
                        @Override public void visitCode() {
                            super.visitCode();
                            if (observed) {
                                context();
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "enter",
                                    "(Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)V", false);
                                if(owner.equals("world/level/NaturalSpawner") && method.equals("spawnMobsForChunkGeneration")) {
                                    super.visitVarInsn(Opcodes.ALOAD,0);
                                    super.visitMethodInsn(Opcodes.INVOKESTATIC,HOOK,"worldArgument","(Ljava/lang/Object;)Ljava/lang/Object;",false);
                                    super.visitTypeInsn(Opcodes.CHECKCAST,MC+"world/level/ServerLevelAccessor");
                                    super.visitVarInsn(Opcodes.ASTORE,0);
                                    super.visitVarInsn(Opcodes.ALOAD,3);
                                    super.visitMethodInsn(Opcodes.INVOKESTATIC,HOOK,"placementArgument","(Ljava/lang/Object;)Ljava/lang/Object;",false);
                                    super.visitTypeInsn(Opcodes.CHECKCAST,MC+"util/RandomSource");
                                    super.visitVarInsn(Opcodes.ASTORE,3);
                                }
                            }
                        }
                        @Override public void visitMethodInsn(int opcode, String target, String called, String desc, boolean iface) {
                            if (entropy && target.equals(MC + "util/RandomSource") && called.equals("create") && desc.equals("()Lnet/minecraft/util/RandomSource;")) {
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "entityRandom", "()Ljava/lang/Object;", false);
                                super.visitTypeInsn(Opcodes.CHECKCAST, MC + "util/RandomSource");
                            } else super.visitMethodInsn(opcode, target, called, desc, iface);
                        }
                        @Override public void visitInsn(int opcode) {
                            if (observed && opcode >= Opcodes.IRETURN && opcode <= Opcodes.RETURN) {
                                if (opcode == Opcodes.RETURN) super.visitInsn(Opcodes.ACONST_NULL);
                                else {
                                    super.visitInsn(result.getSize() == 2 ? Opcodes.DUP2 : Opcodes.DUP);
                                    box(result);
                                }
                                context();
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "exit",
                                    "(Ljava/lang/Object;Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)V", false);
                            }
                            super.visitInsn(opcode);
                        }
                    };
                }
            }, 0);
            return writer.toByteArray();
        } catch (Throwable failure) { GenerationSpawnProbe.fatal(failure); return null; }
    }
}
