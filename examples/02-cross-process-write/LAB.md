# Rust Unsafe 实战：macOS mach 内核接口

通过两个实验，从零学习 Rust 的 `unsafe` 和裸指针操作——直接调用 macOS 内核底层 API，读写系统/进程内存。

---

## 项目结构

```
rs-unsafe/
├── Cargo.toml              # workspace 配置
├── LAB.md                  # 本文档
├── examples/
│   ├── 01-read-meminfo/    # 实验 1：读取本机物理内存信息
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   └── 02-cross-process-write/  # 实验 2：跨进程内存修改器
│       ├── Cargo.toml
│       ├── TargetApp.java       # 被修改的 Java 目标进程
│       └── src/main.rs
```

运行方式：
```bash
cargo run -p read-meminfo          # 实验 1
cargo run -p cross-process-write   # 实验 2（需要 sudo）
```

---

## 实验 1：读取 macOS 物理内存信息

### 目标

用 `host_statistics64` 读取内核的内存统计结构体，计算物理内存总量和可用内存。

### 涉及的 mach API

| 函数 | 作用 |
|------|------|
| `sysconf(_SC_PAGESIZE)` | 获取内存页大小 |
| `mach_host_self()` | 获取当前主机的 mach 端口 |
| `host_statistics64()` | 读取内核的 vm_statistics64 结构体 |

### unsafe 出现的位置

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

### macOS 内存分类

实验中发现一个关键概念：macOS 的 "free" 内存数值通常很小，但这不代表内存不足。

| 页类型 | 含义 | 类比 Linux |
|--------|------|-----------|
| `wire_count` | 内核锁定，不可换出 | — |
| `active_count` | 正在被进程使用 | — |
| `inactive_count` | 最近用过，可随时回收 | buff/cache |
| `speculative_count` | 推测性预读，可回收 | — |
| `free_count` | 完全空闲 | free |

**真正可用内存 = free + inactive + speculative**。macOS 的哲学是"闲置内存就是浪费"，主动用空闲内存做缓存（变成 inactive），需要时立即回收。

### 运行结果

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

---

## 实验 2：跨进程内存修改器

### 目标

用 `task_for_pid` + `mach_vm_read` + `mach_vm_write` 跨进程搜索并修改一个 Java 进程的变量值（100 → 999999）。

### 踩过的坑

#### 坑 1：`host_statistics64` 需要的是 host 端口，不是 task 端口

`mach_task_self()` 返回的是进程 task 端口，但 `host_statistics64` 需要的是 `mach_host_self()` 返回的主机端口。传错会返回 `EINVAL (22)`。

#### 坑 2：`vm_region_basic_info_64` 结构体有一个 `reserved` 字段

macOS SDK 的头文件（`/usr/include/mach/vm_region.h`）中，结构体定义有一个 `boolean_t reserved` 字段，容易漏掉：

```c
struct vm_region_basic_info_64 {
    vm_prot_t               protection;
    vm_prot_t               max_protection;
    vm_inherit_t            inheritance;
    boolean_t               shared;
    boolean_t               reserved;       // ← 这个！
    memory_object_offset_t  offset;
    vm_behavior_t           behavior;
    unsigned short          user_wired_count;
};
```

漏掉后结构体大小从 36 变成 32，`info_count` 从 9 变成 8，`mach_vm_region` 直接返回 `KERN_INVALID_ARGUMENT (4)`。

#### 坑 3：结构体使用了 `#pragma pack(push, 4)`

SDK 头文件整个被 `#pragma pack(push, 4)` 包裹，意味着所有字段按 4 字节对齐（不是默认的 8 字节）。所以 `u64` 的 `offset` 字段可以出现在非 8 对齐的位置（字节偏移 20）。Rust 中需要用 `#[repr(C, packed(4))]`：

```rust
#[repr(C, packed(4))]
struct VmRegionBasicInfo64 {
    protection: i32,        // offset 0
    max_protection: i32,    // offset 4
    inheritance: i32,       // offset 8
    shared: i32,            // offset 12
    reserved: i32,          // offset 16
    offset: u64,            // offset 20  ← 非 8 对齐！
    behavior: i32,          // offset 28
    user_wired_count: u16,  // offset 32
}                           // 总大小 = 36
```

经验教训：**做 FFI 时，用 C 程序打印 `sizeof` 和 `offsetof` 来验证结构体布局，不要猜。**

```c
// 用这个快速验证
printf("sizeof = %zu, count = %zu\n",
       sizeof(struct vm_region_basic_info_64),
       sizeof(struct vm_region_basic_info_64) / sizeof(int));
// 输出: sizeof = 36, count = 9
```

#### 坑 4：盲目写入 3616 个地址会崩溃 Java 进程

值 100 在内存中出现了 3616 次（常量池、JIT 代码、栈帧等），全部写入 999999 导致 JVM 内部数据被破坏，最终 SIGSEGV 崩溃：

```
# A fatal error has been detected by the Java Runtime Environment:
#  SIGSEGV (0xb) at pc=0x0000000108e8485c
# Problematic frame:
# V  [libjvm.dylib+0x74c85c]  SignatureStream::SignatureStream(...)
```

真正的变量只是其中 1 个地址，其余都是误伤。改进思路：做两次扫描——先扫 100，然后让 Java 修改值后再扫新值，两次的交集就是真正的变量地址。

### 涉及的 mach API

| 函数 | 作用 | 类比 Linux |
|------|------|-----------|
| `mach_task_self_` | 获取本进程 task 端口 | `getpid()` 级别 |
| `task_for_pid()` | 获取目标进程的 task 端口 | `ptrace(PTRACE_ATTACH)` |
| `mach_vm_region()` | 枚举目标进程的虚拟内存区域 | `/proc/pid/maps` |
| `mach_vm_read()` | 从目标进程复制内存到本进程 | `process_vm_readv()` |
| `mach_vm_write()` | 把本进程数据写入目标进程 | `process_vm_writev()` |

### 跨进程指针操作总结

两个进程的虚拟地址空间完全隔离——进程 A 的 `0x1234` 和进程 B 的 `0x1234` 指向不同的物理内存。所有跨进程操作都必须经过内核中转：

```
本进程                          内核                          目标进程
───────                        ────                        ────
new_bytes.as_ptr()
      │
      ├── mach_vm_write() ──→  读取本进程内存
      │                         查找目标页表
      │                         写入物理页  ──────→  addr 处被修改
      │                                                       │
      │                                            目标进程访问 addr
      │                                            读到新值，毫不知情
```

代码中 5 处裸指针操作：

| # | 操作 | unsafe 的原因 |
|---|------|--------------|
| 1 | `&mut task_port` 传给 `task_for_pid` | 请求内核授予另一个进程的控制权 |
| 2 | `&mut info as *mut _ as *mut i32` | C 类型擦除，结构体指针强转 |
| 3 | `&mut data_addr` 接收 `mach_vm_read` 缓冲区 | 内核返回的整数地址，需要信任 |
| 4 | `data_addr as *const u8` → `from_raw_parts` | 整数→裸指针→切片，编译器无法验证 |
| 5 | `new_bytes.as_ptr() as u64` 传给 `mach_vm_write` | 指针→整数，跨进程写入绕过所有安全保证 |

### 运行结果

```
$ java TargetApp
Java 进程启动！PID = 24451
当前 Java 内部 hp = 100
当前 Java 内部 hp = 100
...
当前 Java 内部 hp = 999999    ← 被修改！
当前 Java 内部 hp = 999999

$ sudo cargo run -p cross-process-write 24451
[ok] 已获取进程 24451 的 task 端口: 2563

--- 扫描虚拟内存 ---
搜索目标: 100 (字节模式: [100, 0, 0, 0])
扫描了 254 个 RW 区域，共 869.5 MB
找到 3616 个匹配地址

--- 写入 ---
将所有匹配地址 100 → 999999
写入完成: 成功 3610/3616 个地址

>>> 观察 Java 进程输出，hp 应该从 100 变为 999999 <<<
```

---

## 关键收获

1. **Rust 的 unsafe 不是"危险"，而是"编译器放弃了担保"**。在 FFI 场景下，unsafe 是我们向编译器承诺"我已确认这段操作是安全的"。

2. **跨进程操作的本质是"地址空间隔离"**。每个进程有独立的虚拟地址空间，必须通过内核 API 中转。裸指针只在同一个地址空间内有意义。

3. **FFI 结构体布局必须与 C 完全一致**。用 `#[repr(C)]` 保证字段顺序，用 `#[repr(C, packed(N))]` 匹配 `#pragma pack`，用 C 程序打印 `sizeof` / `offsetof` 验证。差一个字段或对齐方式不对，内核 API 就会返回 `KERN_INVALID_ARGUMENT`。

4. **macOS 内存管理不同于 Linux**。"可用内存"不能只看 free_count，必须加上 inactive_count 和 speculative_count。
