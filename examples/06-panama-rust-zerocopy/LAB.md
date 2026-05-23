# 实验 06: Java (Panama) + Rust 零拷贝

## 要点

Java 通过 Project Panama 在堆外开辟 1GB 内存 (`MemorySegment`)，
将**裸指针**传给 Rust dylib。Rust 用 Rayon 多线程并行做 XOR 加密 + FNV-1a 哈希，
Java 顺着原指针读取结果。全程数据不进 JVM 堆。

对比 Rust 内部:
- **顺序处理**: 单线程逐块 XOR + 哈希
- **Rayon 并行**: 多线程同时处理不同块

## 运行

```bash
cd examples/06-panama-rust-zerocopy
./run.sh
```

需要 JDK 21+ (`--enable-preview --enable-native-access=ALL-UNNAMED`)。

## 架构

```
Java (Panama)                    Rust (.dylib)
┌─────────────────┐              ┌──────────────┐
│ Arena.ofConfined │  裸指针     │ xor_encrypt   │ ← unsafe 裸指针
│ MemorySegment    │ ──────────→ │ fnv1a_hash    │
│ (1GB 堆外)       │ ←────────── │ rayon::par    │ ← 多线程并行
└─────────────────┘  顺着原指针  └──────────────┘
     读取结果
```

## Rust 关键技巧

裸指针 `*mut T` 不是 `Send + Sync`，无法直接被 Rayon 闭包捕获。
解决: 转为 `usize` 地址值 (天然 `Send + Sync`)，闭包内再转回指针。
各线程通过 offset 访问互不重叠的内存区域，无数据竞争。

## 实测结果 (macOS aarch64, 1GB, 10 核)

| 方式 | 耗时 | 吞吐量 |
|------|------|--------|
| Rust 顺序 | ~1379 ms | ~0.72 GB/s |
| Rust 并行 (Rayon) | ~188 ms | ~5.3 GB/s |
| 并行加速比 | **7.3x** (10 核) | |
