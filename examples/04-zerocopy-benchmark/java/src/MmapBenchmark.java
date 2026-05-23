import java.io.IOException;
import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryMXBean;
import java.nio.channels.FileChannel;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

/**
 * 实验 1: Java 原生 mmap 零拷贝基准测试
 *
 * 通过 FileChannel.map() 将 1GB 文件映射到用户态虚拟地址空间，
 * 数据通过 OS 的页缓存 (page cache) + DMA 机制加载，
 * 完全不经过 JVM 堆内存。
 *
 * 操作: 统计文件中 ASCII 字符 'A' 的出现次数。
 */
public class MmapBenchmark {

    record Result(
            long fileSize,
            long elapsedNs,
            long countA,
            long heapDeltaBytes,
            long gcCountDelta,
            long gcTimeDeltaMs
    ) {
        double elapsedMs() { return elapsedNs / 1e6; }
        double throughputGBs() { return fileSize / (elapsedNs / 1e9) / (1024*1024*1024); }
    }

    static Result run(Path file) throws IOException {
        // 强制 GC 以获取干净的基线
        System.gc();
        try { Thread.sleep(200); } catch (InterruptedException ignored) {}

        MemoryMXBean memBean = ManagementFactory.getMemoryMXBean();
        long heapBefore = memBean.getHeapMemoryUsage().getUsed();
        long gcCountBefore = totalGcCount();
        long gcTimeBefore = totalGcTimeMs();

        long fileSize;
        long count = 0;
        long start = System.nanoTime();

        try (FileChannel channel = FileChannel.open(file, StandardOpenOption.READ)) {
            fileSize = channel.size();

            // 核心操作: mmap 映射 → 数据留在 OS page cache，不进入 JVM 堆
            var buffer = channel.map(FileChannel.MapMode.READ_ONLY, 0, fileSize);
            byte target = (byte) 'A';
            int limit = buffer.limit();

            for (int i = 0; i < limit; i++) {
                if (buffer.get(i) == target) {
                    count++;
                }
            }
        }

        long elapsed = System.nanoTime() - start;
        long heapAfter = memBean.getHeapMemoryUsage().getUsed();

        return new Result(
                fileSize, elapsed, count,
                heapAfter - heapBefore,
                totalGcCount() - gcCountBefore,
                totalGcTimeMs() - gcTimeBefore
        );
    }

    // ---- GC 统计工具 ----

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
