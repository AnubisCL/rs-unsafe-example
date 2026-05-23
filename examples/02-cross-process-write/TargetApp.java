public class TargetApp {
    private static int hp = 100; // 我们等会儿要用 Rust 爆破这个值

    public static void main(String[] args) throws Exception {
        // 打印 PID 方便 Rust 锁定
        long pid = ProcessHandle.current().pid();
        System.out.println("Java 进程启动！PID = " + pid);

        while (true) {
            System.out.println("当前 Java 内部 hp = " + hp);
            Thread.sleep(1000);
        }
    }
}
