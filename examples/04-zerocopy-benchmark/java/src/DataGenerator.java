import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.FileChannel;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

/**
 * 生成 1GB 测试文件，用于 mmap 实验。
 * 数据模式: 循环填充 'A'~'Z'（26 字符循环），其中 'A' 出现频率为 1/26。
 */
public class DataGenerator {

    static final long FILE_SIZE = 1024L * 1024 * 1024; // 1 GB

    /**
     * 若文件已存在且大小正确则跳过，否则重新生成。
     * 返回文件路径。
     */
    static Path ensureFile(Path file) throws IOException {
        if (Files.exists(file) && Files.size(file) == FILE_SIZE) {
            System.out.println("  [DataGen] 文件已存在，跳过生成: " + file.toAbsolutePath());
            return file;
        }

        System.out.println("  [DataGen] 正在生成 1GB 测试文件...");
        long start = System.nanoTime();

        try (FileChannel ch = FileChannel.open(file,
                StandardOpenOption.CREATE,
                StandardOpenOption.WRITE,
                StandardOpenOption.TRUNCATE_EXISTING)) {

            // 用 4MB 的 DirectByteBuffer 反复填充
            int blockSize = 4 * 1024 * 1024;
            ByteBuffer buf = ByteBuffer.allocateDirect(blockSize);
            long written = 0;

            while (written < FILE_SIZE) {
                buf.clear();
                int toWrite = (int) Math.min(blockSize, FILE_SIZE - written);
                buf.limit(toWrite);

                // 填充循环字母表
                for (int i = 0; i < toWrite; i++) {
                    buf.put((byte) ('A' + (int) ((written + i) % 26)));
                }

                buf.flip();
                while (buf.hasRemaining()) {
                    ch.write(buf);
                }
                written += toWrite;
            }
        }

        double sec = (System.nanoTime() - start) / 1e9;
        System.out.printf("  [DataGen] 完成 (%.2f 秒)%n", sec);
        return file;
    }
}
