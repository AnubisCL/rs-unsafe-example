# 实验 04: Java mmap vs Panama+Rust 零拷贝性能基准测试

## 实验目标

对比两种零拷贝架构在大数据处理场景下的性能代差：

| 维度 | 实验 1: Java mmap | 实验 2: Panama + Rust |
|------|-------------------|----------------------|
| 解决的瓶颈 | 磁盘 → 用户态 I/O | JVM ↔ 底层 C/Rust 生态计算 |
| 数据路径 | Disk → Page Cache → mmap 虚拟地址 | Java Off-Heap → 裸指针 → Rust |
| 数据量 | 1 GB | 1 GB (控制变量) |
| JVM 堆 | 不进入 (DirectByteBuffer) | 完全不经过 (MemorySegment) |
| GC 压力 | 极低 | 极低 |

## 架构

```
实验 1: Java mmap 零拷贝
┌──────────┐    DMA     ┌─────────────┐   mmap    ┌───────────────┐
│   SSD    │ ────────→  │ OS Page     │ ────────→ │ Java          │
│  (1GB)   │            │ Cache       │           │ MappedByteBuf │
└──────────┘            └─────────────┘           └───────────────┘
                                                        │
                                                   count('A')
                                                   不进 JVM 堆!

实验 2: Panama + Rust 零拷贝
┌──────────────────────────────────────────────────────────┐
│  Java (JDK 21+ Panama)                                  │
│  ┌─────────────────┐    绝对物理地址指针     ┌────────┐  │
│  │ Arena.ofConfined│ ──────────────────────→ │ Rust   │  │
│  │ MemorySegment   │    零拷贝/零序列化      │ dylib  │  │
│  │ (1GB 堆外)      │ ←────────────────────── │ .dylib │  │
│  └─────────────────┘    顺着原指针读结果     └────────┘  │
│         ↑                               Rayon 并行       │
│    不进 JVM 堆                       XOR 加密 + FNV-1a   │
└──────────────────────────────────────────────────────────┘
```

## 目录结构

```
04-zerocopy-benchmark/
├── rust-hasher/           # Rust 动态链接库
│   ├── Cargo.toml         # crate-type = ["cdylib"]
│   └── src/lib.rs         # 裸指针 + unsafe + Rayon 并行
├── java/src/              # Java 源码 (无包名，JDK 21+)
│   ├── DataGenerator.java # 生成 1GB 测试文件
│   ├── MmapBenchmark.java # 实验 1: mmap 零拷贝
│   ├── PanamaRustBenchmark.java # 实验 2: Panama FFI
│   └── BenchmarkRunner.java     # 主入口 + 对比表格
├── data/                  # 测试数据目录 (运行时生成)
├── run.sh                 # 一键构建运行脚本
└── LAB.md                 # 本文件
```

## 构建与运行

### 一键运行

```bash
cd examples/04-zerocopy-benchmark
./run.sh
```

### 分步操作

```bash
# 1. 编译 Rust 动态链接库 (workspace 根目录)
cd rs-unsafe/
cargo build --release -p rust-hasher

# 2. 编译 Java
cd examples/04-zerocopy-benchmark/
mkdir -p java/out
javac --enable-preview --release 21 -d java/out java/src/*.java

# 3. 运行
java \
    --enable-preview \
    --enable-native-access=ALL-UNNAMED \
    -Djava.library.path=../../../target/release \
    -cp java/out \
    BenchmarkRunner
```

### 关键 JVM 参数说明

| 参数 | 作用 |
|------|------|
| `--enable-preview` | JDK 21 中 FFM API 为 preview 特性，需显式启用 |
| `--enable-native-access=ALL-UNNAMED` | 允许未命名模块执行原生内存操作 (Panama 要求) |
| `-Djava.library.path=...` | 指定 `librust_hasher.dylib` 搜索路径 |

## Rust 核心代码解析

### 导出函数

```rust
#[no_mangle]
pub unsafe extern "C" fn rust_parallel_process(
    data_ptr: *mut u8,    // Java 堆外内存的裸指针
    data_len: usize,      // 内存长度
    result_ptr: *mut u64, // 哈希结果输出缓冲区
    block_size: usize,    // 每个 block 大小
) -> u64                  // 返回 Rust 内部耗时 (纳秒)
```

### 跨线程安全

裸指针 `*mut T` 默认不是 `Send + Sync`，无法直接被 Rayon 闭包捕获。
解决方案: 将指针转为 `usize` (整数类型天然 `Send + Sync`)，在闭包内部再转回指针：

```rust
let data_addr = data_ptr as usize;
let result_addr = result_ptr as usize;

(0..block_count).into_par_iter().for_each(|idx| {
    let data_ptr = data_addr as *mut u8;
    // 每个线程通过 offset 访问互不重叠的内存区域
});
```

安全性依据: 每个 Rayon 线程操作不同 offset 的 block，无数据竞争。

## 实测结果 (macOS aarch64, M1 Pro, 10 核, 数据量 1GB)

```
┌──────────────────────────────┬────────────────────────┬────────────────────────┐
│ 指标                         │ 实验1: Java mmap       │ 实验2: Panama+Rust     │
╞══════════════════════════════╪════════════════════════╪════════════════════════╡
│ 数据量                       │ 1.00 GB                │ 1.00 GB                │
│ 操作类型                     │ 统计字符 'A' 出现次数  │ XOR加密 + FNV-1a 哈希  │
│ 数据路径                     │ 磁盘→PageCache→mmap    │ 堆外内存→裸指针→Rust   │
│ 并行度                       │ 单线程 (Java 主线程)   │ 10 线程 (Rayon)        │
├──────────────────────────────┼────────────────────────┼────────────────────────┤
│ 总耗时 (ms)                  │ 255.46                 │ 174.18                 │
│ 吞吐量 (GB/s)                │ 3.91                   │ 5.74                   │
│ Rust 内部耗时 (ms)           │ N/A                    │ 174.15                 │
│ 并行加速比                   │ N/A                    │ 7.92x                  │
├──────────────────────────────┼────────────────────────┼────────────────────────┤
│ JVM 堆增量                   │ 0 B                    │ ~2 MB                  │
│ GC 次数增量                  │ 0                      │ ~6                     │
│ GC 耗时增量 (ms)             │ 0                      │ ~4                     │
│ 堆/GC 压力                   │ 极低 (DirectByteBuffer)│ 极低 (几乎完全堆外)    │
└──────────────────────────────┴────────────────────────┴────────────────────────┘
```

## 关键结论

1. **mmap 零拷贝**: 解决 磁盘→用户态 I/O 瓶颈。OS 通过 DMA 将数据直接搬入 Page Cache，
   Java 通过 mmap 虚拟地址直接访问，数据完全不经过 JVM 堆。

2. **Panama+Rust 零拷贝**: 解决 JVM ↔ 底层 C/Rust 生态的计算瓶颈。
   Java 通过 Panama 在堆外开辟内存，将裸指针直接传给 Rust dylib。
   Rust 用 Rayon 多线程并行执行 XOR 加密 + FNV-1a 哈希，10 核并行加速 7.82x。

3. **全链路零拷贝**: 两者叠加 = mmap 读盘 + Panama+Rust 计算 = 完整的零拷贝极简架构，
   从磁盘到最终计算结果，全程对 JVM GC 几乎零压力。
