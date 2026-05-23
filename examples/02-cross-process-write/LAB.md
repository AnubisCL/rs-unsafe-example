# 实验 2：跨进程内存修改器

用 `task_for_pid` + `mach_vm_read` + `mach_vm_write` 跨进程搜索并修改一个 Java 进程的变量值（100 → 999999）。

## 运行

```bash
# 终端 1：启动目标 Java 进程
cd examples/02-cross-process-write
javac TargetApp.java && java TargetApp

# 终端 2：运行修改器（需要 sudo）
sudo ../../target/debug/cross-process-write <Java进程PID>
```

## 踩过的坑

### 坑 1：`host_statistics64` 需要的是 host 端口，不是 task 端口

`mach_task_self()` 返回的是进程 task 端口，但 `host_statistics64` 需要的是 `mach_host_self()` 返回的主机端口。传错会返回 `EINVAL (22)`。

### 坑 2：`vm_region_basic_info_64` 结构体有一个 `reserved` 字段

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

### 坑 3：结构体使用了 `#pragma pack(push, 4)`

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
printf("sizeof = %zu, count = %zu\n",
       sizeof(struct vm_region_basic_info_64),
       sizeof(struct vm_region_basic_info_64) / sizeof(int));
// 输出: sizeof = 36, count = 9
```

### 坑 4：盲目写入 3616 个地址会崩溃 Java 进程

值 100 在内存中出现了 3616 次（常量池、JIT 代码、栈帧等），全部写入 999999 导致 JVM 内部数据被破坏，最终 SIGSEGV 崩溃：

```
# A fatal error has been detected by the Java Runtime Environment:
#  SIGSEGV (0xb) at pc=0x0000000108e8485c
# Problematic frame:
# V  [libjvm.dylib+0x74c85c]  SignatureStream::SignatureStream(...)
```

JVM 的 C1 编译器线程在 JIT 编译时读到了被篡改的 `Symbol` 数据，把它当作指针解引用，触发空指针访问（`si_addr: 0x4`）。

真正的变量只是其中 1 个地址，其余都是误伤。改进思路：做两次扫描——先扫 100，然后让 Java 修改值后再扫新值，两次的交集就是真正的变量地址。

## 涉及的 mach API

| 函数 | 作用 | 类比 Linux |
|------|------|-----------|
| `mach_task_self_` | 获取本进程 task 端口 | `getpid()` 级别 |
| `task_for_pid()` | 获取目标进程的 task 端口 | `ptrace(PTRACE_ATTACH)` |
| `mach_vm_region()` | 枚举目标进程的虚拟内存区域 | `/proc/pid/maps` |
| `mach_vm_read()` | 从目标进程复制内存到本进程 | `process_vm_readv()` |
| `mach_vm_write()` | 把本进程数据写入目标进程 | `process_vm_writev()` |

## 跨进程指针操作总结

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

## 运行结果

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
