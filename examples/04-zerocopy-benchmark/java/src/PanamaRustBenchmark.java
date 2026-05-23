import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;
import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryMXBean;

/**
 * 实验 2: Java (Panama) + Rust 零拷贝基准测试
 *
 * Java 侧通过 Project Panama (Foreign Function & Memory API) 在堆外开辟 1GB 内存，
 * 填充测试数据后，将【绝对物理地址指针】传给 Rust dylib。
 *
 * Rust 侧用裸指针 + unsafe + Rayon 多线程对这段内存做:
 *   Phase 1: XOR 加密 (position-dependent key)
 *   Phase 2: FNV-1a 64-bit 哈希
 *
 * 全程数据不进入 JVM 堆，GC 压力为零。
 */
public class PanamaRustBenchmark {

    static final long DATA_SIZE = 1024L * 1024 * 1024; // 1 GB (与 mmap 实验一致)
    static final long BLOCK_SIZE = 1024 * 1024;         // 1 MB per block

    record Result(
            long elapsedNs,
            long rustTimeNs,
            long sequentialTimeNs,
            long dataSize,
            int blockCount,
            int threadCount,
            long[] hashes,
            long heapDeltaBytes,
            long gcCountDelta,
            long gcTimeDeltaMs
    ) {
        double elapsedMs() { return elapsedNs / 1e6; }
        double rustTimeMs() { return rustTimeNs / 1e6; }
        double seqTimeMs() { return sequentialTimeNs / 1e6; }
        double throughputGBs() { return dataSize / (elapsedNs / 1e9) / (1024*1024*1024); }
        double parallelSpeedup() { return sequentialTimeNs == 0 ? 0 : (double) sequentialTimeNs / rustTimeNs; }
    }

    static Result run() throws Throwable {
        System.gc();
        try { Thread.sleep(200); } catch (InterruptedException ignored) {}

        MemoryMXBean memBean = ManagementFactory.getMemoryMXBean();
        long heapBefore = memBean.getHeapMemoryUsage().getUsed();
        long gcCountBefore = totalGcCount();
        long gcTimeBefore = totalGcTimeMs();

        // ---- 加载 Rust 动态库 ----
        System.loadLibrary("rust_hasher");
        SymbolLookup lookup = SymbolLookup.loaderLookup();
        Linker linker = Linker.nativeLinker();

        // rust_thread_count() -> usize
        MethodHandle getThreadCount = linker.downcallHandle(
                lookup.find("rust_thread_count").orElseThrow(),
                FunctionDescriptor.of(ValueLayout.JAVA_LONG)
        );
        int threadCount = (int) (long) getThreadCount.invokeExact();

        // rust_sequential_process(*mut u8, usize, *mut u64, usize) -> u64
        MethodHandle sequentialFn = linker.downcallHandle(
                lookup.find("rust_sequential_process").orElseThrow(),
                FunctionDescriptor.of(
                        ValueLayout.JAVA_LONG,    // return: nanos
                        ValueLayout.ADDRESS,      // data_ptr
                        ValueLayout.JAVA_LONG,    // data_len
                        ValueLayout.ADDRESS,      // result_ptr
                        ValueLayout.JAVA_LONG     // block_size
                )
        );

        // rust_parallel_process(*mut u8, usize, *mut u64, usize) -> u64
        MethodHandle parallelFn = linker.downcallHandle(
                lookup.find("rust_parallel_process").orElseThrow(),
                FunctionDescriptor.of(
                        ValueLayout.JAVA_LONG,
                        ValueLayout.ADDRESS,
                        ValueLayout.JAVA_LONG,
                        ValueLayout.ADDRESS,
                        ValueLayout.JAVA_LONG
                )
        );

        int blockCount = (int) (DATA_SIZE / BLOCK_SIZE);

        // ---- 顺序基线测试 ----
        long sequentialTime;
        try (Arena seqArena = Arena.ofConfined()) {
            MemorySegment seqData = seqArena.allocate(DATA_SIZE);
            fillTestData(seqData);
            MemorySegment seqResults = seqArena.allocate((long) blockCount * 8);

            sequentialTime = (long) sequentialFn.invokeExact(
                    seqData, DATA_SIZE, seqResults, BLOCK_SIZE);
        }

        // ---- 并行主测试 ----
        long parallelTime;
        long[] hashes;
        long totalElapsed;

        try (Arena arena = Arena.ofConfined()) {
            // 核心: 在 JVM 堆外直接开辟 1GB 真实物理内存
            MemorySegment data = arena.allocate(DATA_SIZE);
            fillTestData(data);

            MemorySegment results = arena.allocate((long) blockCount * 8);

            // === 零拷贝跨界调用: Java 直接把堆外内存指针传给 Rust ===
            long start = System.nanoTime();
            parallelTime = (long) parallelFn.invokeExact(
                    data, DATA_SIZE, results, BLOCK_SIZE);
            totalElapsed = System.nanoTime() - start;

            // 顺着原指针直接读取 Rust 写入的哈希结果
            hashes = new long[blockCount];
            for (int i = 0; i < blockCount; i++) {
                hashes[i] = results.get(ValueLayout.JAVA_LONG, i * 8L);
            }
        }

        long heapAfter = memBean.getHeapMemoryUsage().getUsed();

        return new Result(
                totalElapsed, parallelTime, sequentialTime,
                DATA_SIZE, blockCount, threadCount, hashes,
                heapAfter - heapBefore,
                totalGcCount() - gcCountBefore,
                totalGcTimeMs() - gcTimeBefore
        );
    }

    /** 用 1MB 模板批量填充，避免逐字节写入 1GB */
    private static void fillTestData(MemorySegment seg) {
        int templateSize = 1024 * 1024;
        // 先在堆上构建 1MB 模板
        byte[] template = new byte[templateSize];
        for (int i = 0; i < templateSize; i++) {
            template[i] = (byte) ('A' + (i % 26));
        }
        MemorySegment templateSeg = MemorySegment.ofArray(template);

        long size = seg.byteSize();
        for (long offset = 0; offset < size; offset += templateSize) {
            long chunk = Math.min(templateSize, size - offset);
            MemorySegment.copy(templateSeg, 0, seg, offset, chunk);
        }
    }

    static long totalGcCount() {
        long n = 0;
        for (GarbageCollectorMXBean gc : ManagementFactory.getGarbageCollectorMXBeans()) {
            if (gc.getCollectionCount() >= 0) n += gc.getCollectionCount();
        }
        return n;
    }

    static long totalGcTimeMs() {
        long ms = 0;
        for (GarbageCollectorMXBean gc : ManagementFactory.getGarbageCollectorMXBeans()) {
            if (gc.getCollectionTime() >= 0) ms += gc.getCollectionTime();
        }
        return ms;
    }
}
