# 实验 05: Java 原生 mmap 零拷贝

## 要点

通过 `FileChannel.map()` 将文件映射到用户态虚拟地址空间，数据经 OS Page Cache + DMA 加载，**不经过 JVM 堆**。

对比两种遍历策略:
- **逐字节**: `buffer.get(i)` — 简单直观，每次有边界检查开销
- **SWAR**: `buffer.getLong(i)` + 位运算 — 8 字节一组并行判定，减少 8 倍迭代

## 运行

```bash
cd examples/05-mmap-zerocopy
./run.sh
```

纯 Java，无需 Rust 或 JVM 特殊参数。

## SWAR 原理

```
目标: 在一个 long (8字节) 中找出值等于 0x41 ('A') 的字节

1. chunk ^ 0x4141414141414141   → 匹配的字节变 0x00，其余非零
2. (xor - 0x01..01) & ~xor & 0x80..80   → 匹配位置的高位 = 1
3. Long.bitCount()              → 直接得到匹配的字节数
```

## 实测结果 (macOS aarch64, 1GB)

| 方式 | 耗时 | 吞吐量 |
|------|------|--------|
| 逐字节 buffer.get | ~710 ms | ~1.4 GB/s |
| SWAR getLong | ~367 ms | ~2.7 GB/s |
| 加速比 | **1.9x** | |
