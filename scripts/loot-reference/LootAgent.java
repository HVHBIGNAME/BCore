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
import org.objectweb.asm.Type;

/** Observe original loot methods on the actual server's main thread. */
public final class LootAgent implements ClassFileTransformer {
    private static final String MC = "net/minecraft/";
    private static final Map<String, Set<String>> METHODS = Map.ofEntries(
        Map.entry("world/level/storage/loot/LootTable", Set.of("fill", "getRandomItems", "getRandomItemsRaw",
            "getAvailableSlots", "shuffleAndSplitItems", "lambda$createStackSplitter$0")),
        Map.entry("world/level/storage/loot/LootContext$Builder", Set.of("create", "withOptionalRandomSeed")),
        Map.entry("world/RandomizableContainer", Set.of("unpackLootTable")),
        Map.entry("world/level/block/entity/RandomizableContainerBlockEntity", Set.of("setLootTable")),
        Map.entry("world/SimpleContainer", Set.of("setItem")),
        Map.entry("world/level/block/entity/BaseContainerBlockEntity", Set.of("setItem")),
        Map.entry("world/level/levelgen/LegacyRandomSource", Set.of("next", "setSeed", "nextGaussian")),
        Map.entry("world/level/levelgen/BitRandomSource", Set.of("nextInt", "nextLong", "nextBoolean", "nextFloat", "nextDouble")),
        Map.entry("util/Util", Set.of("shuffle")),
        Map.entry("util/Mth", Set.of("nextInt", "nextFloat")),
        Map.entry("world/level/storage/loot/LootPool", Set.of("addRandomItem", "addRandomItems")),
        Map.entry("world/item/ItemStack", Set.of("split"))
    );

    public static void premain(String config, Instrumentation instrumentation) throws Exception {
        LootProbe.initialize(config);
        instrumentation.addTransformer(new LootAgent());
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined,
                            ProtectionDomain domain, byte[] bytes) {
        if (name == null || !name.startsWith(MC)) return null;
        String owner = name.substring(MC.length());
        boolean server = owner.equals("server/MinecraftServer");
        boolean level = owner.equals("server/level/ServerLevel");
        boolean functions = owner.startsWith("world/level/storage/loot/functions/");
        boolean numbers = owner.startsWith("world/level/storage/loot/providers/number/");
        boolean conditions = owner.startsWith("world/level/storage/loot/predicates/");
        if (!server && !level && !functions && !numbers && !conditions && !METHODS.containsKey(owner)) return null;
        try {
            ClassReader reader = new ClassReader(bytes);
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_MAXS);
            boolean[] changed = {false};
            reader.accept(new ClassVisitor(Opcodes.ASM9, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String method, String descriptor,
                                                 String signature, String[] exceptions) {
                    MethodVisitor delegate = super.visitMethod(access, method, descriptor, signature, exceptions);
                    if ((access & (Opcodes.ACC_ABSTRACT | Opcodes.ACC_NATIVE | Opcodes.ACC_BRIDGE)) != 0) return delegate;
                    boolean boundary = server && method.equals("setInitialSpawn");
                    boolean tick = server && method.equals("tickServer") || level && method.equals("tick");
                    boolean observe = METHODS.getOrDefault(owner, Set.of()).contains(method)
                        || functions && method.equals("run") || conditions && method.equals("test")
                        || numbers && (method.equals("getInt") || method.equals("getFloat"));
                    if (!boundary && !tick && !observe) return delegate;
                    changed[0] = true;
                    String key = owner + "." + method + descriptor;
                    LootProbe.instrumented(key);
                    return new MethodVisitor(Opcodes.ASM9, delegate) {
                        final boolean isStatic = (access & Opcodes.ACC_STATIC) != 0;
                        final Type result = Type.getReturnType(descriptor);

                        private void box(Type type) {
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

                        private void context() {
                            super.visitLdcInsn(key);
                            if (isStatic) super.visitInsn(Opcodes.ACONST_NULL);
                            else super.visitVarInsn(Opcodes.ALOAD, 0);
                            Type[] types = Type.getArgumentTypes(descriptor);
                            super.visitLdcInsn(types.length);
                            super.visitTypeInsn(Opcodes.ANEWARRAY, "java/lang/Object");
                            int local = isStatic ? 0 : 1;
                            for (int i = 0; i < types.length; i++) {
                                super.visitInsn(Opcodes.DUP);
                                super.visitLdcInsn(i);
                                super.visitVarInsn(types[i].getOpcode(Opcodes.ILOAD), local);
                                box(types[i]);
                                super.visitInsn(Opcodes.AASTORE);
                                local += types[i].getSize();
                            }
                        }

                        @Override public void visitCode() {
                            super.visitCode();
                            if (boundary) {
                                super.visitVarInsn(Opcodes.ALOAD, 0);
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootProbe", "run", "(Ljava/lang/Object;)V", false);
                            } else if (tick) {
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootProbe", "unexpectedTick", "()V", false);
                            } else {
                                context();
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootProbe", "enter",
                                    "(Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)V", false);
                            }
                        }

                        @Override public void visitInsn(int opcode) {
                            if (observe && opcode >= Opcodes.IRETURN && opcode <= Opcodes.RETURN) {
                                if (opcode == Opcodes.RETURN) super.visitInsn(Opcodes.ACONST_NULL);
                                else {
                                    super.visitInsn(result.getSize() == 2 ? Opcodes.DUP2 : Opcodes.DUP);
                                    box(result);
                                }
                                context();
                                super.visitMethodInsn(Opcodes.INVOKESTATIC, "LootProbe", "exit",
                                    "(Ljava/lang/Object;Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)V", false);
                            }
                            super.visitInsn(opcode);
                        }
                    };
                }
            }, 0);
            if (!changed[0]) return null;
            byte[] transformed = writer.toByteArray();
            LootProbe.transformed(name, bytes, transformed);
            return transformed;
        } catch (Throwable error) {
            LootProbe.fatal(error);
            return null;
        }
    }
}
