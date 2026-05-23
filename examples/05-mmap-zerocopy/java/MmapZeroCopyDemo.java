import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.channels.FileChannel;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

/**
 * 实验: Java 原生 mmap 零拷贝
 *
 * 通过 FileChannel.map() 将文件映射到用户态虚拟地址空间，
 * 数据通过 OS Page Cache + DMA 机制加载，不经过 JVM 堆。
 *
 * 对比两种遍历策略:
 *   1. 逐字节遍历 (buffer.get)
 *   2. SWAR 批量统计 (buffer.getLong + 位运算)
 */
public class MmapZeroCopyDemo {

    static final long FILE_SIZE = 1024L * 1024 * 1024; // 1 GB

    public static void main(String[] args) throws IOException {
        Path file = Path.of("data", "testdata_1gb.bin");
        generateIfAbsent(file);

        System.out.println("预热...");
        countNaive(file);
        countSWAR(file);

        // ---- 逐字节 ----
        System.out.println();
        System.out.println("--- 逐字节遍历 (buffer.get) ---");
        long start = System.nanoTime();
        long naive = countNaive(file);
        long naiveNs = System.nanoTime() - start;
        printResult(naive, naiveNs);

        // ---- SWAR ----
        System.out.println();
        System.out.println("--- SWAR 批量统计 (getLong + 位运算) ---");
        start = System.nanoTime();
        long swar = countSWAR(file);
        long swarNs = System.nanoTime() - start;
        printResult(swar, swarNs);

        System.out.println();
        System.out.printf("SWAR 加速比: %.2fx%n", (double) naiveNs / swarNs);
        System.out.printf("结果一致: %b%n", naive == swar);
    }

    static void printResult(long count, long ns) {
        double ms = ns / 1e6;
        double gbs = FILE_SIZE / (ns / 1e9) / (1024 * 1024 * 1024);
        System.out.printf("  count('A') = %d | %.0f ms | %.2f GB/s%n", count, ms, gbs);
    }

    // ============================================================
    //  方式 1: 逐字节 — 简单直观，每次调用有边界检查开销
    // ============================================================

    static long countNaive(Path file) throws IOException {
        try (FileChannel ch = FileChannel.open(file, StandardOpenOption.READ)) {
            var buf = ch.map(FileChannel.MapMode.READ_ONLY, 0, ch.size());
            long count = 0;
            byte target = (byte) 'A';
            for (int i = 0; i < buf.limit(); i++) {
                if (buf.get(i) == target) count++;
            }
            return count;
        }
    }

    // ============================================================
    //  方式 2: SWAR — 每次读 8 字节 long，位运算并行统计
    //
    //  原理:
    //    1. chunk ^ 0x4141...41 → 匹配的字节变 0x00，其余非零
    //    2. (xor - 0x01..01) & ~xor & 0x80..80 → 匹配位置的高位 = 1
    //    3. Long.bitCount 数 1 的个数 / 8 = 匹配的字节数
    // ============================================================

    static long countSWAR(Path file) throws IOException {
        try (FileChannel ch = FileChannel.open(file, StandardOpenOption.READ)) {
            var buf = ch.map(FileChannel.MapMode.READ_ONLY, 0, ch.size());
            buf.order(ByteOrder.LITTLE_ENDIAN);

            long pattern = 0x4141414141414141L; // 'A'=0x41 填充每个字节
            long loBit   = 0x0101010101010101L;
            long hiBit   = 0x8080808080808080L;

            long count = 0;
            int limit = buf.limit();
            int i = 0;

            // 8 字节一组
            for (; i <= limit - 8; i += 8) {
                long chunk = buf.getLong(i);
                long xor = chunk ^ pattern;
                long zeroDetect = (xor - loBit) & ~xor & hiBit;
                count += Long.bitCount(zeroDetect);
            }

            // 处理尾部不足 8 字节
            byte target = (byte) 'A';
            for (; i < limit; i++) {
                if (buf.get(i) == target) count++;
            }
            return count;
        }
    }

    // ---- 文件生成 ----

    static void generateIfAbsent(Path file) throws IOException {
        if (Files.exists(file) && Files.size(file) == FILE_SIZE) {
            System.out.println("测试文件已存在: " + file.toAbsolutePath());
            return;
        }
        Files.createDirectories(file.getParent());
        System.out.printf("正在生成 %.0f MB 测试文件...%n", FILE_SIZE / 1024.0 / 1024);

        try (FileChannel ch = FileChannel.open(file,
                StandardOpenOption.CREATE, StandardOpenOption.WRITE, StandardOpenOption.TRUNCATE_EXISTING)) {
            int blk = 4 * 1024 * 1024;
            ByteBuffer buf = ByteBuffer.allocateDirect(blk);
            long written = 0;

            while (written < FILE_SIZE) {
                buf.clear();
                int n = (int) Math.min(blk, FILE_SIZE - written);
                buf.limit(n);
                for (int i = 0; i < n; i++) {
                    buf.put((byte) ('A' + (int) ((written + i) % 26)));
                }
                buf.flip();
                while (buf.hasRemaining()) ch.write(buf);
                written += n;
            }
        }
        System.out.println("生成完成");
    }
}
