package com.geek.agent;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;

import org.objectweb.asm.*;

public class InjectAgent {

    private static final String TARGET_CLASS = "com/geek/auth/service/AuthService";

    public static void premain(String args, Instrumentation inst) {
        registerTransformer(inst);
    }

    public static void agentmain(String args, Instrumentation inst) {
        registerTransformer(inst);
    }

    private static void registerTransformer(Instrumentation inst) {
        System.out.println("[InjectAgent] Registering bytecode transformer...");

        inst.addTransformer(new ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String className, Class<?> classBeingRedefined,
                                    ProtectionDomain protectionDomain, byte[] classfileBuffer) {
                if (!TARGET_CLASS.equals(className)) return null;

                System.out.println("[InjectAgent] Found target class: " + className);

                try {
                    byte[] result = doTransform(classfileBuffer);
                    System.out.println("[InjectAgent] Patch applied successfully! (" + result.length + " bytes)");
                    return result;
                } catch (Exception e) {
                    System.err.println("[InjectAgent] Patch FAILED: " + e.getMessage());
                    e.printStackTrace();
                    return null; // 返回 null 表示不修改
                }
            }
        }, true);

        // 触发 retransform 已加载的类
        for (Class<?> clazz : inst.getAllLoadedClasses()) {
            if (inst.isModifiableClass(clazz) && clazz.getName().replace('.', '/').equals(TARGET_CLASS)) {
                try {
                    inst.retransformClasses(clazz);
                    System.out.println("[InjectAgent] Retransformed: " + clazz.getName());
                } catch (Exception e) {
                    System.err.println("[InjectAgent] Retransform failed: " + e.getMessage());
                    e.printStackTrace();
                }
            }
        }
    }

    private static byte[] doTransform(byte[] classfileBuffer) {
        ClassReader cr = new ClassReader(classfileBuffer);
        ClassWriter cw = new ClassWriter(cr, ClassWriter.COMPUTE_MAXS | ClassWriter.COMPUTE_FRAMES);

        ClassVisitor cv = new ClassVisitor(Opcodes.ASM9, cw) {
            @Override
            public MethodVisitor visitMethod(int access, String name, String descriptor,
                                             String signature, String[] exceptions) {
                MethodVisitor mv = super.visitMethod(access, name, descriptor, signature, exceptions);
                if ("login".equals(name)
                        && "(Ljava/lang/String;Ljava/lang/String;)Lcom/geek/auth/model/LoginResult;".equals(descriptor)) {
                    System.out.println("[InjectAgent] Patching login()...");
                    return new LoginMethodPatcher(mv, access, name, descriptor);
                }
                return mv;
            }
        };

        // 使用 EXPAND_FRAMES 让 ClassReader 展开栈帧，配合 COMPUTE_FRAMES
        cr.accept(cv, ClassReader.EXPAND_FRAMES);
        return cw.toByteArray();
    }

    /**
     * 替换 login 方法体为:
     *   public LoginResult login(String username, String password) {
     *       String token = this.tokenService.createToken(username);
     *       return LoginResult.ok(token);
     *   }
     *
     * 关键: 在 visitCode 中写入新指令，跳过所有原始指令，
     * visitMaxs/visitEnd 透传给 ClassWriter (COMPUTE_MAXS 会自动重算)
     */
    static class LoginMethodPatcher extends MethodVisitor {

        LoginMethodPatcher(MethodVisitor mv, int access, String name, String desc) {
            super(Opcodes.ASM9, mv);
        }

        @Override
        public void visitCode() {
            super.visitCode();

            // this.tokenService
            mv.visitVarInsn(Opcodes.ALOAD, 0);
            mv.visitFieldInsn(Opcodes.GETFIELD,
                    "com/geek/auth/service/AuthService",
                    "tokenService",
                    "Lcom/geek/auth/service/TokenService;");

            // .createToken(username)
            mv.visitVarInsn(Opcodes.ALOAD, 1);
            mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL,
                    "com/geek/auth/service/TokenService",
                    "createToken",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    false);
            mv.visitVarInsn(Opcodes.ASTORE, 3);

            // LoginResult.ok(token) → ARETURN
            mv.visitVarInsn(Opcodes.ALOAD, 3);
            mv.visitMethodInsn(Opcodes.INVOKESTATIC,
                    "com/geek/auth/model/LoginResult",
                    "ok",
                    "(Ljava/lang/String;)Lcom/geek/auth/model/LoginResult;",
                    false);
            mv.visitInsn(Opcodes.ARETURN);

            // 不在这里调用 visitMaxs / visitEnd — 让它们自然通过
        }

        // 跳过原始方法的所有指令 (不转发给 mv)
        @Override public void visitInsn(int opcode) {}
        @Override public void visitIntInsn(int opcode, int operand) {}
        @Override public void visitVarInsn(int opcode, int var) {}
        @Override public void visitTypeInsn(int opcode, String type) {}
        @Override public void visitFieldInsn(int opcode, String owner, String name, String desc) {}
        @Override public void visitMethodInsn(int opcode, String owner, String name, String desc, boolean itf) {}
        @Override public void visitJumpInsn(int opcode, Label label) {}
        @Override public void visitLdcInsn(Object value) {}
        @Override public void visitIincInsn(int var, int increment) {}
        @Override public void visitTableSwitchInsn(int min, int max, Label dflt, Label... labels) {}
        @Override public void visitLookupSwitchInsn(Label dflt, int[] keys, Label[] labels) {}
        @Override public void visitMultiANewArrayInsn(String desc, int dims) {}
        @Override public void visitLineNumber(int line, Label start) {}
        @Override public void visitLocalVariable(String name, String desc, String signature, Label start, Label end, int index) {}

        // visitMaxs 和 visitEnd 透传给 ClassWriter
        // COMPUTE_MAXS / COMPUTE_FRAMES 会自动重算，忽略传入的参数
    }
}
