use std::env;
use std::mem;
use std::process;

// ============================================================
// macOS mach 内核层常量
// ============================================================
const VM_REGION_BASIC_INFO_64: i32 = 9;
const VM_PROT_READ: i32 = 1;
const VM_PROT_WRITE: i32 = 2;

/// 与 C 结构体 vm_region_basic_info_64 内存布局完全一致
///
/// macOS SDK 中这个结构体被 #pragma pack(push, 4) 包裹，
/// 即 4 字节对齐（非默认的 8 字节）。所以 u64 的 offset 字段
/// 可以在非 8 对齐的位置（offset 20）。
///
/// 对应的 C 定义（来自 /usr/include/mach/vm_region.h）：
///   struct vm_region_basic_info_64 {
///       vm_prot_t               protection;     // int
///       vm_prot_t               max_protection; // int
///       vm_inherit_t            inheritance;    // int
///       boolean_t               shared;         // int
///       boolean_t               reserved;       // int  ← 容易漏掉！
///       memory_object_offset_t  offset;         // uint64_t
///       vm_behavior_t           behavior;       // int
///       unsigned short          user_wired_count;
///   };
///
/// 总大小 = 36 字节，info_count = 36/4 = 9
#[repr(C, packed(4))]
struct VmRegionBasicInfo64 {
    protection: i32,
    max_protection: i32,
    inheritance: i32,
    shared: i32,
    reserved: i32, // ← SDK 有这个字段，不能漏
    offset: u64,
    behavior: i32,
    user_wired_count: u16,
}

// ============================================================
// 手动 FFI 声明
//
// 这些 mach 虚拟内存操作函数存在于 macOS 系统库
// (/usr/lib/system/libsystem_kernel.dylib) 中，但 libc crate
// 没有导出它们，所以需要手动用 extern "C" 声明，链接器会自动找到。
// ============================================================
extern "C" {
    /// 我们自己进程的 task 端口（全局变量，由动态链接器初始化）
    static mach_task_self_: u32;

    /// 获取目标进程的 task 端口（调试器的必经之路）
    fn task_for_pid(
        target_tport: u32,     // 调用者的 task 端口
        pid: i32,              // 目标进程 PID
        t: *mut u32,           // 【输出】接收目标 task 端口
    ) -> i32;

    /// 从目标进程虚拟地址空间读取内存到本进程
    fn mach_vm_read(
        target_task: u32,      // 目标 task 端口
        address: u64,          // 目标进程的虚拟地址
        size: u64,             // 要读取的字节数
        data: *mut u64,        // 【输出】接收缓冲区地址（内核在本进程分配）
        outsize: *mut u32,     // 【输出】实际读取字节数
    ) -> i32;

    /// 把本进程的数据写入目标进程虚拟地址空间
    fn mach_vm_write(
        target_task: u32,      // 目标 task 端口
        address: u64,          // 目标进程的虚拟地址
        data: u64,             // 本进程中数据的地址（以整数形式传递）
        size: u32,             // 要写入的字节数
    ) -> i32;

    /// 枚举目标进程的虚拟内存区域（region）
    fn mach_vm_region(
        target_task: u32,
        address: *mut u64,     // 【输入/输出】起始查找地址 → 区域实际起始地址
        size: *mut u64,        // 【输出】区域大小
        flavor: i32,           // 查询类型
        info: *mut i32,        // 【输出】区域属性结构体
        info_cnt: *mut u32,    // 【输入/输出】info 的 int32 个数
        object_name: *mut u32, // 【输出】内存对象端口
    ) -> i32;

    /// 释放 mach_vm_read 分配的缓冲区
    fn vm_deallocate(
        target_task: u32,
        address: u64,
        size: u32,
    ) -> i32;
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("用法: sudo {} <Java进程PID>", args[0]);
        process::exit(1);
    }
    let pid: i32 = args[1].parse().expect("PID 必须是数字");
    let target_value: i32 = 100;
    let new_value: i32 = 999999;

    // ============================================================
    // 第一步：获取目标进程的 task 端口
    // ============================================================
    //
    // 【跨进程操作的前提：task 端口】
    //
    // macOS 的 mach 微内核中，每个进程由一个 "task" 对象代表。
    // task 是进程所有资源的容器：虚拟内存空间、线程列表、端口权限等。
    //
    // 要操作另一个进程的内存，首先需要拿到它的 task 端口（一个整数句柄）。
    // 这就像「获得了进入目标进程领地的钥匙」。
    //
    // task_for_pid() 向内核申请：把 PID=xxx 那个进程的 task 端口给我。
    // 类比 Linux：相当于 ptrace(PTRACE_ATTACH, pid)。
    //
    // 【安全门槛】macOS 限制谁可以调用：
    //   - 必须是 root（sudo）或有调试 entitlement
    //   - 目标进程不能有更严格的代码签名保护
    //   - SIP 可能额外阻止
    //
    let mut task_port: u32 = 0;
    let self_task = unsafe { mach_task_self_ };
    let kr = unsafe {
        task_for_pid(
            self_task,
            pid,
            &mut task_port,
            //  ^^^^^^^^^^^^^^
            // 【裸指针使用 #1】隐式 &mut u32 → *mut u32
            // 内核把目标进程的端口句柄写入这个地址。
            // 为什么 unsafe？因为我们在请求内核授予对另一个进程的完全控制权，
            // Rust 编译器无法验证返回的端口是否有效、调用者是否有权限。
        )
    };

    if kr != 0 {
        eprintln!("task_for_pid 失败 (错误码: {})", kr);
        eprintln!("请用 sudo 运行，并确认 PID {} 存在", pid);
        process::exit(1);
    }
    println!("[ok] 已获取进程 {} 的 task 端口: {}", pid, task_port);

    // ============================================================
    // 第二步：扫描虚拟内存，查找值 100
    // ============================================================
    //
    // 【虚拟地址空间的概念】
    //
    // 每个进程都认为自己独占了一整块 0 到 2^48 的内存空间。
    // 进程看到的地址不是物理内存地址，而是经过内核页表映射后的虚拟地址。
    //
    // 进程 A 的 0x1234 和进程 B 的 0x1234 指向完全不同的物理位置。
    // 要读写另一个进程的虚拟地址，必须通过内核的 mach API。
    //
    // 虚拟地址空间被划分为连续的「区域 region」，每个区域有：
    //   - 起始地址和大小
    //   - 权限标志 (读 / 写 / 执行)
    //   - 映射类型 (匿名内存、文件映射、共享内存等)
    //
    // 策略：
    //   1. mach_vm_region 枚举所有区域
    //   2. 筛选「可读+可写」区域（排除代码段等只读区域）
    //   3. mach_vm_read 把区域内容复制到本进程
    //   4. 在副本中逐字节搜索 4 字节模式
    //
    let needle = target_value.to_le_bytes(); // 100 → [0x64, 0x00, 0x00, 0x00]
    let mut matches: Vec<u64> = Vec::new();
    let mut scan_addr: u64 = 0;
    let mut region_count: u32 = 0;
    let mut total_scanned: u64 = 0;

    println!("\n--- 扫描虚拟内存 ---");
    println!("搜索目标: {} (字节模式: {:?})", target_value, needle);

    loop {
        let mut region_size: u64 = 0;
        let mut info: VmRegionBasicInfo64 = unsafe { mem::zeroed() };
        let mut info_count = mem::size_of::<VmRegionBasicInfo64>() as u32 / 4;
        let mut object_name: u32 = 0;

        // ----------------------------------------------------------
        // mach_vm_region：获取下一个虚拟内存区域
        // ----------------------------------------------------------
        //
        // 调用方式：
        //   scan_addr 传入起始查找地址，返回时变为区域实际起始地址
        //   region_size 返回区域大小
        //   info 返回区域属性（权限等）
        //
        // 每次调用后 scan_addr += region_size，即可遍历下一个区域。
        // 返回非零表示已到达地址空间末尾。
        //
        let kr = unsafe {
            mach_vm_region(
                task_port,
                &mut scan_addr,
                &mut region_size,
                VM_REGION_BASIC_INFO_64,
                &mut info as *mut _ as *mut i32,
                //  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                // 【裸指针使用 #2】结构体指针强制转型
                //
                // vm_region_basic_info_64 是 C 结构体，内核直接写入。
                // mach API 的 info 参数声明为 integer_t*（即 int32_t*），
                // 但实际指向的是具体的 info 结构体。
                //
                // 这是 C 语言常见的「类型擦除」模式：
                //   不同 flavor 对应不同的 info 结构体，
                //   但函数签名统一用 integer_t* 接收。
                //   Rust 中需要 as 强转，属于 unsafe 操作。
                &mut info_count,
                &mut object_name,
            )
        };

        if kr != 0 {
            break;
        }
        region_count += 1;

        // 只扫描「可读 + 可写」区域
        // Java 堆（存放 hp 变量的地方）的权限是 RW
        let can_read = (info.protection & VM_PROT_READ) != 0;
        let can_write = (info.protection & VM_PROT_WRITE) != 0;
        if !can_read || !can_write {
            scan_addr += region_size;
            continue;
        }

        // 跳过过大的区域（> 1GB），避免 mach_vm_read 失败或卡顿
        if region_size > 1024 * 1024 * 1024 {
            scan_addr += region_size;
            continue;
        }

        // ----------------------------------------------------------
        // mach_vm_read：把目标进程的内存复制到本进程
        // ----------------------------------------------------------
        //
        // 【跨进程读取的核心原理】
        //
        //   scan_addr 存在于「目标进程」的虚拟地址空间。
        //   在我们的进程中，这个地址可能指向完全不同的数据（甚至无效）。
        //   我们不能直接用指针访问它！
        //
        //   mach_vm_read 做的事情：
        //     1. 告诉内核：把目标进程 task_port 虚拟地址 scan_addr
        //        处的 region_size 字节复制出来
        //     2. 内核切换到目标进程的页表，找到对应的物理页
        //     3. 内核在「我们的」地址空间分配一块缓冲区
        //     4. 把物理页的内容复制到我们的缓冲区
        //     5. 通过 data 指针返回缓冲区地址
        //
        //   之后我们在自己的进程里就能正常访问这块数据了。
        //   注意：这是一份「快照副本」，修改它不会影响目标进程。
        //
        let mut data_addr: u64 = 0;
        let mut data_size: u32 = 0;

        let read_kr = unsafe {
            mach_vm_read(
                task_port,
                scan_addr,
                region_size,
                &mut data_addr,
                //  ^^^^^^^^^^^^
                // 【裸指针使用 #3】&mut u64 → *mut u64
                //
                // data_addr 接收内核分配的缓冲区地址。
                // 这是一个 u64 整数值，表示我们进程空间中的一个地址。
                //
                // 为什么 mach API 用 u64 而非指针类型？
                //   因为 mach API 是跨语言、跨平台的 C 接口，
                //   用整数表示地址可以避免指针大小差异。
                //   32 位系统上 vm_offset_t 是 u32，64 位上是 u64。
                &mut data_size,
            )
        };

        if read_kr == 0 && data_size >= 4 {
            // 在复制出来的数据中搜索
            //
            // 【整数 → 裸指针转换】
            //
            //   data_addr (u64) 需要转为指针才能访问内存：
            //     data_addr as *const u8
            //
            //   这是 unsafe 的核心操作：把内核返回的整数当作内存地址。
            //   Rust 编译器无法验证这个地址是否有效、是否已释放。
            //   我们必须相信内核的返回值是正确、已分配的。
            //
            unsafe {
                let ptr = data_addr as *const u8;
                let slice = std::slice::from_raw_parts(ptr, data_size as usize);
                //                     ^^^^^^^^^^^^^^^^^
                // 【裸指针使用 #4】从裸指针创建 Rust 切片引用
                //
                // 这告诉编译器：「从 ptr 开始的 data_size 字节，当作 [u8] 来用」。
                // 为什么 unsafe？编译器无法保证：
                //   - ptr 指向的内存是否有效
                //   - data_size 是否准确（不会越界）
                //   - 底层内存是否被并发修改
                //
                // 但这里我们是安全的：
                //   - ptr 由内核 mach_vm_read 分配，保证有效
                //   - data_size 由内核返回，是实际读取的字节数
                //   - 这是本进程的私有缓冲区，没有并发访问

                // 逐字节搜索 4 字节模式
                for i in 0..(slice.len().saturating_sub(3)) {
                    if slice[i..i + 4] == needle {
                        matches.push(scan_addr + i as u64);
                    }
                }
            }

            total_scanned += data_size as u64;

            // 释放 mach_vm_read 在本进程分配的缓冲区
            unsafe {
                vm_deallocate(self_task, data_addr, data_size);
            }
        }

        // 移动到下一个区域
        scan_addr += region_size;
    }

    println!(
        "扫描了 {} 个 RW 区域，共 {:.1} MB",
        region_count,
        total_scanned as f64 / 1024.0 / 1024.0
    );
    println!("找到 {} 个匹配地址", matches.len());

    if matches.is_empty() {
        eprintln!("未找到目标值，请确认 Java 进程中 hp 变量值确实为 {}", target_value);
        process::exit(1);
    }

    // 打印匹配结果
    for (i, &addr) in matches.iter().enumerate() {
        if i < 30 || i >= matches.len().saturating_sub(3) {
            println!("  [{:>3}] 0x{:016X}", i, addr);
        } else if i == 30 {
            println!("  ... (省略 {} 个) ...", matches.len() - 33);
        }
    }

    // ============================================================
    // 第三步：写入新值
    // ============================================================
    //
    // 【跨进程写入的核心】
    //
    // mach_vm_write 做的事情：
    //   1. 从「我们进程」的 data 地址复制 size 字节
    //   2. 写入「目标进程」虚拟地址 address 处
    //   3. 目标进程毫不知情——没有任何通知或回调
    //   4. 目标进程下次访问该地址时，就会读到新值
    //
    // 这等价于：有人偷偷改了目标进程的全局变量，进程本身完全不知道。
    //
    // 为什么内核能做到？因为 mach 内核控制着所有的页表映射。
    // 内核可以随时修改任何进程的任何虚拟地址对应的物理页内容。
    //
    println!("\n--- 写入 ---");
    println!("将所有匹配地址 {} → {}", target_value, new_value);

    let new_bytes = new_value.to_le_bytes(); // 999999 → [0x3F, 0x42, 0x0F, 0x00]
    let mut success = 0;

    for &addr in &matches {
        let write_kr = unsafe {
            mach_vm_write(
                task_port,
                addr,                        // 目标进程的虚拟地址
                new_bytes.as_ptr() as u64,    // 本进程中数据的地址
                //  ^^^^^^^^^^^^^^^^^^^^^^^
                // 【裸指针使用 #5】指针 → 整数
                //
                // new_bytes.as_ptr() 是 *const u8（我们进程中的地址）
                // as u64 转为整数，传给 mach API 的 data 参数。
                //
                // 这是最关键的跨进程指针操作：
                //   我们进程的地址 → 内核从中读取 → 写入目标进程的地址
                //   两个进程的地址空间完全不重叠，由内核中转。
                //
                // 内核做的事：
                //   1. 在我们的地址空间，从 new_bytes.as_ptr() 复制 4 字节
                //   2. 在目标进程的地址空间，把这 4 字节写入 addr 处
                //   3. 修改目标进程的页表映射（如有必要）
                4,
            )
        };

        if write_kr == 0 {
            success += 1;
        }
    }

    println!("写入完成: 成功 {}/{} 个地址", success, matches.len());
    if success > 0 {
        println!("\n>>> 观察 Java 进程输出，hp 应该从 {} 变为 {} <<<", target_value, new_value);
    }
}
