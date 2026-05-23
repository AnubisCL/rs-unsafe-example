import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * 零拷贝性能对比实验 — 主运行器
 *
 * 实验 1: Java 原生 mmap 零拷贝 (磁盘 → 用户态)
 * 实验 2: Java (Panama) + Rust 零拷贝 (JVM ↔ 底层计算)
 */
public class BenchmarkRunner {

    static final int WARMUP_RUNS = 2;
    static final int MEASURE_RUNS = 3;

    public static void main(String[] args) throws Throwable {
        printBanner();

        // =============== 准备测试数据 ===============
        Path dataFile = Path.of("data", "testdata_1gb.bin");
        System.out.println("  [准备] 生成/检查 1GB 测试文件...");
        DataGenerator.ensureFile(dataFile);
        System.out.println();

        // =============== 实验 1: mmap 零拷贝 ===============
        System.out.println("===== 实验 1: Java 原生 mmap 零拷贝 =====");
        System.out.println("  (磁盘 → OS Page Cache → mmap 虚拟地址，数据不进 JVM 堆)");
        System.out.println();

        // 预热
        System.out.println("  [预热] " + WARMUP_RUNS + " 轮...");
        for (int i = 0; i < WARMUP_RUNS; i++) {
            MmapBenchmark.run(dataFile);
        }

        // 正式测量
        System.out.println("  [测量] " + MEASURE_RUNS + " 轮，取最优...");
        List<MmapBenchmark.Result> mmapResults = new ArrayList<>();
        for (int i = 0; i < MEASURE_RUNS; i++) {
            MmapBenchmark.Result r = MmapBenchmark.run(dataFile);
            mmapResults.add(r);
            System.out.printf("    Run %d: %8.2f ms | throughput %5.2f GB/s | count('A') = %d%n",
                    i + 1, r.elapsedMs(), r.throughputGBs(), r.countA());
        }
        MmapBenchmark.Result mmapBest = bestMmap(mmapResults);
        System.out.printf("  [最优] %.2f ms | %.2f GB/s%n", mmapBest.elapsedMs(), mmapBest.throughputGBs());
        System.out.println();

        // =============== 实验 2: Panama + Rust 零拷贝 ===============
        System.out.println("===== 实验 2: Java (Panama) + Rust 零拷贝 =====");
        System.out.println("  (Java 堆外 1GB → 裸指针传 Rust → Rayon 并行 XOR 加密 + FNV-1a 哈希)");
        System.out.println();

        // 预热
        System.out.println("  [预热] " + WARMUP_RUNS + " 轮...");
        for (int i = 0; i < WARMUP_RUNS; i++) {
            PanamaRustBenchmark.run();
        }

        // 正式测量
        System.out.println("  [测量] " + MEASURE_RUNS + " 轮，取最优...");
        List<PanamaRustBenchmark.Result> panamaResults = new ArrayList<>();
        for (int i = 0; i < MEASURE_RUNS; i++) {
            PanamaRustBenchmark.Result r = PanamaRustBenchmark.run();
            panamaResults.add(r);
            System.out.printf("    Run %d: total %7.2f ms | Rust并行 %7.2f ms | Rust顺序 %7.2f ms | 并行加速 %.2fx%n",
                    i + 1, r.elapsedMs(), r.rustTimeMs(), r.seqTimeMs(), r.parallelSpeedup());
        }
        PanamaRustBenchmark.Result panamaBest = bestPanama(panamaResults);
        System.out.printf("  [最优] 总耗时 %.2f ms | Rust并行 %.2f ms | 加速 %.2fx%n",
                panamaBest.elapsedMs(), panamaBest.rustTimeMs(), panamaBest.parallelSpeedup());
        System.out.println();

        // =============== 对比表格 ===============
        printComparisonTable(mmapBest, panamaBest);
    }

    // ---- 选出最优结果 (elapsedNs 最小) ----

    static MmapBenchmark.Result bestMmap(List<MmapBenchmark.Result> list) {
        return list.stream().min((a, b) -> Long.compare(a.elapsedNs(), b.elapsedNs())).orElseThrow();
    }

    static PanamaRustBenchmark.Result bestPanama(List<PanamaRustBenchmark.Result> list) {
        return list.stream().min((a, b) -> Long.compare(a.elapsedNs(), b.elapsedNs())).orElseThrow();
    }

    // ---- 打印横幅 ----

    static void printBanner() {
        System.out.println();
        System.out.println("  ╔═══════════════════════════════════════════════════════════════╗");
        System.out.println("  ║       Zero-Copy Performance Benchmark  (macOS)              ║");
        System.out.println("  ║       Java mmap  vs  Java(Panama) + Rust                    ║");
        System.out.println("  ╚═══════════════════════════════════════════════════════════════╝");
        System.out.println();
        System.out.printf("  JDK: %s | OS: %s | Arch: %s%n",
                System.getProperty("java.version"),
                System.getProperty("os.name"),
                System.getProperty("os.arch"));
        System.out.printf("  CPU 核心数: %d%n", Runtime.getRuntime().availableProcessors());
        System.out.println();
    }

    // ---- 打印对比表格 ----

    static void printComparisonTable(MmapBenchmark.Result mmap, PanamaRustBenchmark.Result panama) {
        String line = "  +" + "─".repeat(30) + "+" + "─".repeat(24) + "+" + "─".repeat(24) + "+";
        String header = "  | %-28s | %-22s | %-22s |";

        System.out.println("  ╔════════════════════════════════════════════════════════════════════════════════════╗");
        System.out.println("  ║                         性 能 对 比 表                                            ║");
        System.out.println("  ╠════════════════════════════════════════════════════════════════════════════════════╣");

        System.out.println(line);
        System.out.printf(header, "指标", "实验1: Java mmap", "实验2: Panama+Rust");
        System.out.println();
        System.out.println(line.replace('─', '═'));

        row("数据量", formatBytes(mmap.fileSize()), formatBytes(panama.dataSize()));
        row("操作类型", "统计字符 'A' 出现次数", "XOR加密 + FNV-1a 哈希");
        row("数据路径", "磁盘→PageCache→mmap", "堆外内存→裸指针→Rust");
        row("并行度", "单线程 (Java 主线程)", panama.threadCount() + " 线程 (Rayon)");
        System.out.println(line.replace('─', '─'));

        row("总耗时 (ms)", String.format("%.2f", mmap.elapsedMs()), String.format("%.2f", panama.elapsedMs()));
        row("吞吐量 (GB/s)", String.format("%.2f", mmap.throughputGBs()), String.format("%.2f", panama.throughputGBs()));
        row("Rust 内部耗时 (ms)", "N/A", String.format("%.2f", panama.rustTimeMs()));
        row("并行加速比", "N/A", String.format("%.2fx", panama.parallelSpeedup()));
        System.out.println(line.replace('─', '─'));

        row("JVM 堆增量", formatBytes(mmap.heapDeltaBytes()), formatBytes(panama.heapDeltaBytes()));
        row("GC 次数增量", String.valueOf(mmap.gcCountDelta()), String.valueOf(panama.gcCountDelta()));
        row("GC 耗时增量 (ms)", String.valueOf(mmap.gcTimeDeltaMs()), String.valueOf(panama.gcTimeDeltaMs()));
        row("堆/GC 压力", "极低 (DirectByteBuffer)", "零 (完全堆外)");
        System.out.println(line.replace('─', '─'));

        // 验证信息
        row("验证: count('A')", String.valueOf(mmap.countA()), "N/A");
        row("验证: hash[0]", "N/A", String.format("0x%016X", panama.hashes()[0]));
        row("验证: hash[last]", "N/A", String.format("0x%016X", panama.hashes()[panama.hashes().length - 1]));

        System.out.println(line);
        System.out.println();
        System.out.println("  ── 关键结论 ──────────────────────────────────────────────────");
        System.out.println("  1. mmap 零拷贝: 解决 磁盘→用户态 I/O 瓶颈，DMA 直接搬运，不进 JVM 堆");
        System.out.println("  2. Panama+Rust: 解决 JVM↔底层生态 计算瓶颈，裸指针直通，Rayon 并行压榨 CPU");
        System.out.println("  3. 两者叠加: mmap 读盘 + Panama+Rust 计算 = 全链路零拷贝极简架构");
        System.out.println();
    }

    static void row(String label, String val1, String val2) {
        System.out.printf("  | %-28s | %-22s | %-22s |%n", label, val1, val2);
    }

    static String formatBytes(long bytes) {
        if (bytes < 0) return "-" + formatBytes(-bytes);
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1024 * 1024) return String.format("%.1f KB", bytes / 1024.0);
        if (bytes < 1024L * 1024 * 1024) return String.format("%.1f MB", bytes / (1024.0 * 1024));
        return String.format("%.2f GB", bytes / (1024.0 * 1024 * 1024));
    }
}
