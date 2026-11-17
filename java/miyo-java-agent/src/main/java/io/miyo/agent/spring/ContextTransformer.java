package io.miyo.agent.spring;

import java.lang.instrument.ClassFileTransformer;
import java.security.ProtectionDomain;

// 编译期引用原始包名；shade 阶段统一重定位到 io.miyo.agent.shaded.org.objectweb.asm
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

/**
 * Spring 上下文捕获字节码转换器（ASM 插桩）。
 *
 * 只处理 {@code org/springframework/context/support/AbstractApplicationContext}：
 * 在 publishEvent / getBean(String) / getBean(Class) 方法体入口插入
 * {@code SpringCapture.capture(this)}（栈平衡、无分支，不改任何既有帧语义）。
 *
 * 安全兜底：任何解析 / 写回失败一律返回 null（不修改该类）——插桩绝不
 * 因转换器自身问题影响目标应用。
 */
public final class ContextTransformer implements ClassFileTransformer {

    /** 目标类内部名。 */
    public static final String TARGET_INTERNAL =
            "org/springframework/context/support/AbstractApplicationContext";

    private static final String CAPTURE_OWNER = "io/miyo/agent/spring/SpringCapture";
    private static final String CAPTURE_DESC = "(Ljava/lang/Object;)V";

    @Override
    public byte[] transform(ClassLoader loader, String className, Class<?> classBeingRedefined,
                            ProtectionDomain protectionDomain, byte[] classfileBuffer) {
        if (!TARGET_INTERNAL.equals(className)) {
            return null;
        }
        try {
            ClassReader reader = new ClassReader(classfileBuffer);
            ClassWriter writer = new ClassWriter(reader, 0); // 不重算帧：仅在方法入口插桩
            ClassVisitor visitor = new ClassVisitor(Opcodes.ASM9, writer) {
                @Override
                public MethodVisitor visitMethod(int access, String name, String desc,
                                                 String signature, String[] exceptions) {
                    MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
                    if (!isInstrumentable(name, desc)) {
                        return mv;
                    }
                    return new MethodVisitor(Opcodes.ASM9, mv) {
                        @Override
                        public void visitCode() {
                            super.visitCode();
                            // 方法入口：ALOAD 0 (this) → INVOKESTATIC capture(Object)V
                            mv.visitVarInsn(Opcodes.ALOAD, 0);
                            mv.visitMethodInsn(Opcodes.INVOKESTATIC, CAPTURE_OWNER,
                                    "capture", CAPTURE_DESC, false);
                        }
                    };
                }
            };
            // 不重算帧（flags=0 复制原帧；入口插桩不改变栈形状，后续帧仍有效）
            reader.accept(visitor, 0);
            return writer.toByteArray();
        } catch (Throwable t) {
            // 转换失败 = 不修改（返回 null 让原字节码通过）
            System.err.println("[miyo-agent] Spring 上下文插桩失败（原类放行）: " + t);
            return null;
        }
    }

    /** 仅插桩实例热路径方法（static / 构造器 / 名字不含目标方法直接放行）。 */
    private static boolean isInstrumentable(String name, String desc) {
        if (name.equals("publishEvent")
                && (desc.equals("(Ljava/lang/Object;)V")
                        || desc.equals("(Ljava/lang/Object;Lorg/springframework/core/ResolvableType;)V"))) {
            return true;
        }
        if (name.equals("getBean")
                && (desc.equals("(Ljava/lang/String;)Ljava/lang/Object;")
                        || desc.equals("(Ljava/lang/Class;)Ljava/lang/Object;"))) {
            return true;
        }
        return false;
    }
}
