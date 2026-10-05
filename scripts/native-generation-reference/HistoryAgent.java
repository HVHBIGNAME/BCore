import java.lang.instrument.*;
import java.security.ProtectionDomain;
import java.util.*;
import org.objectweb.asm.*;

/** Observes native methods; only the selected bootstrap boundary terminates control flow. */
public final class HistoryAgent implements ClassFileTransformer {
    private static final String HOOK = "HistoryProbe";
    private static final String MC = "net/minecraft/";
    private static final Map<String, Set<String>> METHODS = Map.ofEntries(
        Map.entry("server/MinecraftServer", Set.of("setInitialSpawn", "loadLevel", "tickServer")),
        Map.entry("server/level/ServerLevel", Set.of("tick")),
        Map.entry("server/level/ServerChunkCache", Set.of("getChunk", "getChunkFutureMainThread")),
        Map.entry("server/level/ChunkMap", Set.of("applyStep", "scheduleGenerationTask", "prepareTickingChunk")),
        Map.entry("server/level/ChunkGenerationTask", Set.of("create", "runUntilWait", "scheduleLayer", "scheduleChunkInLayer", "markForCancellation")),
        Map.entry("server/level/GenerationChunkHolder", Set.of("scheduleChunkGenerationTask", "acquireStatusBump", "completeFuture")),
        Map.entry("server/level/ChunkTaskDispatcher", Set.of("submit", "scheduleForExecution")),
        Map.entry("world/level/chunk/status/ChunkStep", Set.of("apply")),
        Map.entry("world/level/levelgen/placement/PlacedFeature", Set.of("placeWithBiomeCheck")),
        Map.entry("world/level/levelgen/WorldgenRandom", Set.of("setDecorationSeed", "setFeatureSeed")),
        Map.entry("server/level/WorldGenRegion", Set.of("setCurrentlyGenerating", "setBlock", "ensureCanWrite", "addFreshEntity", "markPosForPostprocessing", "getHeight")),
        Map.entry("world/ticks/WorldGenTickAccess", Set.of("schedule")),
        Map.entry("world/level/chunk/ProtoChunk", Set.of("markPosForPostprocessing")),
        Map.entry("world/level/chunk/LevelChunk", Set.of("postProcessGeneration"))
    );

    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        HistoryProbe.initialize(config);
        instrumentation.addTransformer(new HistoryAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        if (!name.startsWith(MC) || !METHODS.containsKey(name.substring(MC.length()))) return null;
        String owner = name.substring(MC.length());
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            reader.accept(new ClassVisitor(Opcodes.ASM8, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String descriptor,
                                                 String signature, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, descriptor, signature, exceptions);
                    if (!METHODS.get(owner).contains(method) || (access & Opcodes.ACC_ABSTRACT) != 0) return delegate;
                    HistoryProbe.instrumented(owner + "." + method + descriptor);
                    return new MethodVisitor(Opcodes.ASM8, delegate) {
                        final boolean isStatic = (access & Opcodes.ACC_STATIC) != 0;
                        final Type result = Type.getReturnType(descriptor);

                        void args() {
                            Type[] types = Type.getArgumentTypes(descriptor);
                            visitLdcInsn(types.length);
                            visitTypeInsn(Opcodes.ANEWARRAY, "java/lang/Object");
                            int local = isStatic ? 0 : 1;
                            for (int i = 0; i < types.length; i++) {
                                visitInsn(Opcodes.DUP);
                                visitLdcInsn(i);
                                visitVarInsn(types[i].getOpcode(Opcodes.ILOAD), local);
                                box(types[i]);
                                visitInsn(Opcodes.AASTORE);
                                local += types[i].getSize();
                            }
                        }

                        void box(Type type) {
                            String wrapper = switch (type.getSort()) {
                                case Type.BOOLEAN -> "Boolean";
                                case Type.BYTE -> "Byte";
                                case Type.SHORT -> "Short";
                                case Type.INT -> "Integer";
                                case Type.LONG -> "Long";
                                case Type.FLOAT -> "Float";
                                case Type.DOUBLE -> "Double";
                                case Type.CHAR -> "Character";
                                default -> null;
                            };
                            if (wrapper != null) visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/" + wrapper,
                                "valueOf", "(" + type.getDescriptor() + ")Ljava/lang/" + wrapper + ";", false);
                        }

                        void context() {
                            visitLdcInsn(owner + "." + method);
                            if (isStatic) visitInsn(Opcodes.ACONST_NULL);
                            else visitVarInsn(Opcodes.ALOAD, 0);
                            args();
                        }

                        @Override
                        public void visitCode() {
                            super.visitCode();
                            context();
                            visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "enter",
                                "(Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)V", false);
                        }

                        @Override
                        public void visitInsn(int opcode) {
                            if (opcode >= Opcodes.IRETURN && opcode <= Opcodes.RETURN) {
                                boolean stageFuture = owner.equals("world/level/chunk/status/ChunkStep")
                                    || owner.equals("server/level/ChunkMap") && method.equals("applyStep");
                                if (stageFuture && opcode == Opcodes.ARETURN) {
                                    context();
                                    visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "future",
                                        "(Ljava/util/concurrent/CompletableFuture;Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)Ljava/util/concurrent/CompletableFuture;", false);
                                } else {
                                    if (opcode == Opcodes.RETURN) super.visitInsn(Opcodes.ACONST_NULL);
                                    else {
                                        super.visitInsn(result.getSize() == 2 ? Opcodes.DUP2 : Opcodes.DUP);
                                        box(result);
                                    }
                                    context();
                                    visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "exit",
                                        "(Ljava/lang/Object;Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)V", false);
                                }
                            }
                            super.visitInsn(opcode);
                        }
                    };
                }
            }, 0);
            return writer.toByteArray();
        } catch (Throwable error) {
            HistoryProbe.fatal(error);
            return null;
        }
    }
}
