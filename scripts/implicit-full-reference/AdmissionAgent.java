import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.MessageDigest;
import java.security.ProtectionDomain;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;

/** Companion to the unchanged HistoryAgent; observes native arguments and returns. */
public final class AdmissionAgent implements ClassFileTransformer {
    private static final String MC = "net/minecraft/";
    private static final String HOOK = "AdmissionProbe";
    private static final Map<String, Set<String>> METHODS = Map.of(
        "server/level/ServerChunkCache", Set.of("getChunk", "getChunkFutureMainThread", "storeInCache", "clearCache", "runDistanceManagerUpdates"),
        "server/level/ServerChunkCache$MainThreadExecutor", Set.of("pollTask", "doRunTask"),
        "util/thread/BlockableEventLoop", Set.of("schedule", "managedBlock"),
        "server/level/ChunkHolder", Set.of("updateFutures"),
        "server/level/GenerationChunkHolder", Set.of("scheduleChunkGenerationTask"),
        "server/level/ChunkMap", Set.of("prepareAccessibleChunk"),
        "world/level/chunk/status/ChunkStatusTasks", Set.of("full", "lambda$full$0"),
        "world/level/chunk/LevelChunk", Set.of("setFullStatus")
    );

    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        AdmissionProbe.initialize();
        instrumentation.addTransformer(new AdmissionAgent());
    }

    private static String sha(byte[] bytes) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        if (name == null || !name.startsWith(MC)) return null;
        String owner = name.substring(MC.length());
        if (!METHODS.containsKey(owner)) return null;
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            List<String> methods = new ArrayList<>();
            reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String descriptor,
                                                 String signature, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, descriptor, signature, exceptions);
                    if (!METHODS.get(owner).contains(method) || (access & (Opcodes.ACC_ABSTRACT | Opcodes.ACC_NATIVE)) != 0)
                        return delegate;
                    methods.add(method + descriptor);
                    return new MethodVisitor(Opcodes.ASM9, delegate) {
                        final boolean isStatic = (access & Opcodes.ACC_STATIC) != 0;
                        final Type result = Type.getReturnType(descriptor);

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
                            if (wrapper != null) super.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/" + wrapper,
                                "valueOf", "(" + type.getDescriptor() + ")Ljava/lang/" + wrapper + ";", false);
                        }

                        void context() {
                            super.visitLdcInsn(owner + "." + method);
                            if (isStatic) super.visitInsn(Opcodes.ACONST_NULL);
                            else super.visitVarInsn(Opcodes.ALOAD, 0);
                            Type[] arguments = Type.getArgumentTypes(descriptor);
                            super.visitLdcInsn(arguments.length);
                            super.visitTypeInsn(Opcodes.ANEWARRAY, "java/lang/Object");
                            int local = isStatic ? 0 : 1;
                            for (int i = 0; i < arguments.length; i++) {
                                super.visitInsn(Opcodes.DUP);
                                super.visitLdcInsn(i);
                                super.visitVarInsn(arguments[i].getOpcode(Opcodes.ILOAD), local);
                                box(arguments[i]);
                                super.visitInsn(Opcodes.AASTORE);
                                local += arguments[i].getSize();
                            }
                        }

                        @Override
                        public void visitCode() {
                            super.visitCode();
                            context();
                            super.visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "enter",
                                "(Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)V", false);
                        }

                        @Override
                        public void visitMethodInsn(int opcode, String calledOwner, String calledName,
                                                    String calledDescriptor, boolean isInterface) {
                            if (owner.equals("world/level/chunk/status/ChunkStatusTasks") && method.equals("full")
                                    && opcode == Opcodes.INVOKESTATIC && calledOwner.equals("java/util/concurrent/CompletableFuture")
                                    && calledName.equals("supplyAsync")
                                    && calledDescriptor.equals("(Ljava/util/function/Supplier;Ljava/util/concurrent/Executor;)Ljava/util/concurrent/CompletableFuture;")) {
                                super.visitInsn(Opcodes.DUP2);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, HOOK, "fullSubmission",
                                    "(Ljava/lang/Object;Ljava/lang/Object;)V", false);
                            }
                            super.visitMethodInsn(opcode, calledOwner, calledName, calledDescriptor, isInterface);
                        }

                        @Override
                        public void visitInsn(int opcode) {
                            if (opcode >= Opcodes.IRETURN && opcode <= Opcodes.RETURN) {
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
            byte[] transformed = writer.toByteArray();
            AdmissionProbe.transformed(owner, methods, sha(bytes), sha(transformed));
            return transformed;
        } catch (Throwable failure) {
            HistoryProbe.fatal(failure);
            return null;
        }
    }
}
