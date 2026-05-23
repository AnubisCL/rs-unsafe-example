# 实验 1：读取 macOS 物理内存信息

用 `host_statistics64` 读取内核的内存统计结构体，计算物理内存总量和可用内存。

## 运行

```bash
cargo run -p read-meminfo
```

## 涉及的 mach API

| 函数 | 作用 |
|------|------|
| `sysconf(_SC_PAGESIZE)` | 获取内存页大小 |
| `mach_host_self()` | 获取当前主机的 mach 端口 |
| `host_statistics64()` | 读取内核的 vm_statistics64 结构体 |

## unsafe 出现的位置

**1. `sysconf` — POSIX C API 调用**

```rust
let page_size: u64 = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };
```

虽然是简单的系统调用，但 Rust 无法验证 C 函数的返回值语义（返回 -1 表示错误），必须标记 unsafe。

**2. `mem::zeroed()` — 零初始化 C 结构体**

```rust
let mut vm_stats: libc::vm_statistics64 = unsafe { mem::zeroed() };
```

`mem::zeroed()` 对某些 Rust 类型（如 `String`）会产生非法状态，但对 C 结构体（Plain Old Data）是安全的。编译器把这个判断责任交给了我们。

**3. `&mut vm_stats as *mut _ as *mut c_int` — 结构体指针强制转型**

```rust
host_statistics64(
    host_port,
    libc::HOST_VM_INFO64,
    &mut vm_stats as *mut _ as *mut c_int,  // 引用 → 裸指针 → 类型擦除
    &mut count,
)
```

mach API 的 info 参数声明为 `integer_t*`（即 `int32_t*`），但实际指向 `vm_statistics64` 结构体。这是 C 的类型擦除模式，Rust 中需要 `as` 强转，编译器无法验证类型安全。

## macOS 内存分类

实验中发现一个关键概念：macOS 的 "free" 内存数值通常很小，但这不代表内存不足。

| 页类型 | 含义 | 类比 Linux |
|--------|------|-----------|
| `wire_count` | 内核锁定，不可换出 | — |
| `active_count` | 正在被进程使用 | — |
| `inactive_count` | 最近用过，可随时回收 | buff/cache |
| `speculative_count` | 推测性预读，可回收 | — |
| `free_count` | 完全空闲 | free |

**真正可用内存 = free + inactive + speculative**。macOS 的哲学是"闲置内存就是浪费"，主动用空闲内存做缓存（变成 inactive），需要时立即回收。

## 运行结果

```
内存页大小:       16384 bytes              ← Apple Silicon 的 16KB 大页

--- 原始页数（来自内核 vm_statistics64） ---
wire (不可换出):   136881 页
active (使用中):   346031 页
inactive (缓存):   343810 页
speculative:         1707 页
free (完全空闲):     8657 页

--- 换算结果 ---
总物理内存:       12.77 GB
已使用(不可回收): 7.37 GB
仅完全空闲:       0.13 GB                  ← 看起来很少！
真正可用内存:     5.40 GB                  ← 这才是真实可用量
占用百分比:       57.69%
```
