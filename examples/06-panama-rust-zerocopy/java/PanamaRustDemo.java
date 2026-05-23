import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;

/**
 * 实验: Java (Panama) + Rust 零拷贝
 *
 * Java 通过 Project Panama 在堆外开辟 1GB 内存 (MemorySegment)，
 * 将裸指针传给 Rust dylib。Rust 用 Rayon 并行做 XOR 加密 + FNV-1a 哈希，
 * Java 顺着原指针读取结果。全程数据不进 JVM 堆。
 *
 * 对比: Rust 顺序 vs Rayon 并行
 */
public class PanamaRustDemo {

    static final long DATA_SIZE = 1024L * 1024 * 1024; // 1 GB
    static final long BLOCK_SIZE = 1024 * 1024;         // 1 MB

    public static void main(String[] args) throws Throwable {
        System.loadLibrary("rust_offheap");
        SymbolLookup lookup = SymbolLookup.loaderLookup();
        Linker linker = Linker.nativeLinker();

        // ---- 查找导出函数 ----
        MethodHandle getThreads = linker.downcallHandle(
                lookup.find("rust_thread_count").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.JAVA_LONG));

        MethodHandle seqFn = linker.downcallHandle(
                lookup.find("rust_sequential_process").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.JAVA_LONG,
                        ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                        ValueLayout.ADDRESS, ValueLayout.JAVA_LONG));

        MethodHandle parFn = linker.downcallHandle(
                lookup.find("rust_parallel_process").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.JAVA_LONG,
                        ValueLayout.ADDRESS, ValueLayout.JAVA_LONG,
                        ValueLayout.ADDRESS, ValueLayout.JAVA_LONG));

        int threads = (int) (long) getThreads.invokeExact();
        int blockCount = (int) (DATA_SIZE / BLOCK_SIZE);

        System.out.println("=== Panama + Rust 零拷贝示例 ===");
        System.out.printf("数据量: %d MB | Block: %d MB | 线程: %d%n%n",
                DATA_SIZE / 1024 / 1024, BLOCK_SIZE / 1024 / 1024, threads);

        // ---- 预热 ----
        System.out.println("预热...");
        try (Arena warmup = Arena.ofConfined()) {
            MemorySegment d = warmup.allocate(BLOCK_SIZE);
            MemorySegment r = warmup.allocate(8);
            long s1 = (long) seqFn.invokeExact(d, BLOCK_SIZE, r, BLOCK_SIZE);
            long p1 = (long) parFn.invokeExact(d, BLOCK_SIZE, r, BLOCK_SIZE);
        }

        // ---- 顺序处理 ----
        System.out.println();
        System.out.println("--- Rust 顺序处理 (单线程) ---");
        long seqNs;
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment data = arena.allocate(DATA_SIZE);
            MemorySegment results = arena.allocate((long) blockCount * 8);
            fillData(data);

            seqNs = (long) seqFn.invokeExact(data, DATA_SIZE, results, BLOCK_SIZE);
            printResult("顺序", seqNs, data, results, blockCount);
        }

        // ---- 并行处理 ----
        System.out.println();
        System.out.println("--- Rust 并行处理 (Rayon) ---");
        long parNs;
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment data = arena.allocate(DATA_SIZE);
            MemorySegment results = arena.allocate((long) blockCount * 8);
            fillData(data);

            parNs = (long) parFn.invokeExact(data, DATA_SIZE, results, BLOCK_SIZE);
            printResult("并行", parNs, data, results, blockCount);
        }

        System.out.printf("%nRayon 并行加速比: %.2fx (%d 线程)%n",
                (double) seqNs / parNs, threads);
    }

    static void printResult(String label, long ns, MemorySegment data,
                            MemorySegment results, int blockCount) {
        double ms = ns / 1e6;
        double gbs = DATA_SIZE / (ns / 1e9) / (1024 * 1024 * 1024);
        long h0 = results.get(ValueLayout.JAVA_LONG, 0);
        long hLast = results.get(ValueLayout.JAVA_LONG, (blockCount - 1) * 8L);
        System.out.printf("  %s: %.0f ms | %.2f GB/s | hash[0]=0x%016X | hash[%d]=0x%016X%n",
                label, ms, gbs, h0, blockCount - 1, hLast);
    }

    /** 用 1MB 模板批量填充 */
    static void fillData(MemorySegment seg) {
        int tplSize = 1024 * 1024;
        byte[] tpl = new byte[tplSize];
        for (int i = 0; i < tplSize; i++) tpl[i] = (byte) ('A' + (i % 26));
        MemorySegment tplSeg = MemorySegment.ofArray(tpl);

        long size = seg.byteSize();
        for (long off = 0; off < size; off += tplSize) {
            MemorySegment.copy(tplSeg, 0, seg, off, Math.min(tplSize, size - off));
        }
    }
}
