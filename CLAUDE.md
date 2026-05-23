# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

Rust unsafe / 底层系统编程教学实验集合。通过一系列渐进式实验演示 macOS 平台上 Rust unsafe FFI、跨进程内存操作、mmap 零拷贝、JVM 字节码注入等技术。部分实验结合 Java（Spring Boot / Panama FFI）。

## 构建命令

```bash
# 构建全部 Rust workspace 成员
cargo build

# 构建单个 crate
cargo build -p read-meminfo
cargo build -p cross-process-write
cargo build -p rust-injector
cargo build -p rust-offheap    # cdylib，供 Java Panama 调用

# 运行
cargo run -p read-meminfo
sudo cargo run -p cross-process-write <PID>

# 实验 03 的 Java 部分需要 Maven
cd examples/03-bytecode-inject/java-agent && mvn clean package -q
cd examples/03-bytecode-inject/java-app && mvn clean package -q -DskipTests

# 实验 05/06 有独立 run.sh 脚本
cd examples/05-mmap-zerocopy && ./run.sh
cd examples/06-panama-rust-zerocopy && ./run.sh
```

## Workspace 结构

根 `Cargo.toml` 是 workspace，包含 4 个 Rust crate 成员。Java/Maven 项目独立构建，不在 workspace 内。

```
Cargo.toml              ← workspace 根
examples/
  01-read-meminfo/       ← Rust binary: macOS vm_statistics64 内存统计
  02-cross-process-write/ ← Rust binary + Java TargetApp: 跨进程搜索改写内存 (mach_vm_read/write)
  03-bytecode-inject/    ← Rust binary (rust-injector) + Java Agent (ASM) + Spring Boot app
    rust-injector/       ← workspace member
    java-agent/          ← Maven, ASM 字节码改写 agent
    java-app/            ← Maven, Spring Boot 登录系统
  05-mmap-zerocopy/      ← 纯 Java: FileChannel.map + SWAR 位运算
  06-panama-rust-zerocopy/
    rust-hasher/         ← workspace member, cdylib (librust_offheap.dylib)
    java/                ← JDK 21+ Panama FFI 调用 Rust dylib
```

## 关键架构要点

### macOS mach API FFI 模式

libc crate 未导出 mach 虚拟内存函数（`mach_vm_read`/`mach_vm_write`/`mach_vm_region`/`task_for_pid`），需要手动 `extern "C"` 声明，链接器自动从 `libsystem_kernel.dylib` 解析。

C 结构体布局对齐是常见陷阱：macOS SDK 的 `vm_region_basic_info_64` 被 `#pragma pack(push, 4)` 包裹，Rust 中对应 `#[repr(C, packed(4))]`。做 FFI 时用 C 程序打印 `sizeof`/`offsetof` 验证布局。

### Rust cdylib 导出模式

`rust-hasher` 编译为 `cdylib`（`crate-type = ["cdylib"]`），导出 `extern "C"` 函数供 Java Panama FFI 调用。裸指针 `*mut T` 不是 `Send + Sync`，Rayon 并行时转为 `usize` 地址值在闭包间传递。

### JVM Attach Protocol

Rust 通过 UNIX socket 实现 JVM Attach Protocol：创建 `.attach_pid{PID}` 触发文件 → 发 SIGQUIT → 等待 `$TMPDIR/.java_pid{PID}` socket → 发 `load instrument` 命令注入 agent JAR。macOS 的 TMPDIR 是 `/var/folders/xx/xxx/T/`，不是 `/tmp`。

## 技术栈

- Rust (edition 2021)，核心依赖：`libc`、`rayon`
- Java 21+（Panama 实验）、Java 8+（其余）
- macOS 专用 API（mach kernel、libproc）
- 实验 03: Spring Boot + ASM 字节码操作
