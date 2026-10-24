import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;
import java.util.Map;
import java.util.Set;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

/** Count original native operations within explicit saved-chest lifecycle windows. */
public final class ChestLifecycleAgent implements ClassFileTransformer {
    private static final String MC = "net/minecraft/";
    private static final Map<String, Set<String>> METHODS = Map.ofEntries(
        Map.entry("world/level/block/entity/BlockEntity", Set.of("loadStatic", "loadWithComponents", "saveWithFullMetadata")),
        Map.entry("world/level/block/entity/ChestBlockEntity", Set.of("loadAdditional", "saveAdditional")),
        Map.entry("world/RandomizableContainer", Set.of("unpackLootTable", "tryLoadLootTable", "trySaveLootTable")),
        Map.entry("world/ContainerHelper", Set.of("loadAllItems", "saveAllItems")),
        Map.entry("world/level/storage/loot/LootTable", Set.of("fill")),
        Map.entry("world/level/storage/loot/LootContext$Builder", Set.of("create")),
        Map.entry("server/ReloadableServerRegistries$Holder", Set.of("getLootTable")),
        Map.entry("server/MinecraftServer", Set.of("getRandomSequence")),
        Map.entry("util/RandomSource", Set.of("create", "createThreadSafe", "createNewThreadLocalInstance")),
        Map.entry("world/level/levelgen/LegacyRandomSource", Set.of("next", "setSeed", "nextGaussian")),
        Map.entry("world/level/levelgen/BitRandomSource", Set.of("nextInt", "nextLong", "nextBoolean", "nextFloat", "nextDouble")),
        Map.entry("world/level/levelgen/XoroshiroRandomSource", Set.of("nextInt", "nextLong", "nextBoolean", "nextFloat", "nextDouble", "nextGaussian", "setSeed")),
        Map.entry("world/level/levelgen/ThreadSafeLegacyRandomSource", Set.of("next", "setSeed")),
        Map.entry("world/level/levelgen/SingleThreadedRandomSource", Set.of("next", "setSeed"))
    );

    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        ChestLifecycleProbe.initialize(config);
        instrumentation.addTransformer(new ChestLifecycleAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined, ProtectionDomain domain, byte[] bytes) {
        if (name == null || !name.startsWith(MC)) return null;
        String owner = name.substring(MC.length());
        boolean server = owner.equals("server/MinecraftServer");
        boolean level = owner.equals("server/level/ServerLevel");
        if (!METHODS.containsKey(owner) && !level) return null;
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String desc, String sig, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, desc, sig, exceptions);
                    if ((access & (Opcodes.ACC_ABSTRACT | Opcodes.ACC_NATIVE | Opcodes.ACC_BRIDGE)) != 0) return delegate;
                    boolean bootstrap = server && method.equals("setInitialSpawn");
                    boolean tick = server && method.equals("tickServer") || level && method.equals("tick");
                    boolean count = METHODS.getOrDefault(owner, Set.of()).contains(method);
                    if (!bootstrap && !tick && !count) return delegate;
                    String key = owner + "." + method + desc;
                    ChestLifecycleProbe.instrumented(key);
                    return new MethodVisitor(Opcodes.ASM9, delegate) {
                        @Override public void visitCode() {
                            super.visitCode();
                            if (bootstrap) {
                                super.visitVarInsn(Opcodes.ALOAD, 0);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "ChestLifecycleProbe", "run", "(Ljava/lang/Object;)V", false);
                            } else if (tick) {
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "ChestLifecycleProbe", "unexpectedTick", "()V", false);
                            } else {
                                super.visitLdcInsn(key);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "ChestLifecycleProbe", "enter", "(Ljava/lang/String;)V", false);
                                if (owner.equals("world/level/storage/loot/LootTable") && method.equals("fill")) {
                                    super.visitVarInsn(Opcodes.LLOAD, 3);
                                    super.visitMethodInsn(Opcodes.INVOKESTATIC, "ChestLifecycleProbe", "nativeFill", "(J)V", false);
                                }
                            }
                        }

                        @Override public void visitInsn(int opcode) {
                            if (opcode == Opcodes.ARETURN && method.equals("create") && owner.equals("world/level/storage/loot/LootContext$Builder")) {
                                super.visitInsn(Opcodes.DUP);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "ChestLifecycleProbe", "createdContext", "(Ljava/lang/Object;)V", false);
                            }
                            if (opcode == Opcodes.ARETURN && method.equals("getLootTable") && owner.equals("server/ReloadableServerRegistries$Holder")) {
                                super.visitInsn(Opcodes.DUP);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "ChestLifecycleProbe", "resolvedTable", "(Ljava/lang/Object;)V", false);
                            }
                            super.visitInsn(opcode);
                        }
                    };
                }
            }, 0);
            byte[] changed = writer.toByteArray();
            ChestLifecycleProbe.transformed(name, bytes, changed);
            return changed;
        } catch (Throwable error) {
            ChestLifecycleProbe.fatal(error);
            return null;
        }
    }
}
