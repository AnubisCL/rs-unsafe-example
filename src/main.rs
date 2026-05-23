use libc::{c_int, c_uint, host_statistics64};
use std::mem;

fn main() {
    //
    // 第一步：获取当前系统的内存页大小（Page Size）
    //
    // macOS 内核以"页"为最小单位管理物理内存。
    // sysconf 返回的是 POSIX 标准的系统配置值，这里获取每页字节数。
    // 虽然这一步本身不涉及裸指针，但它属于 POSIX C API 调用，Rust 无法验证其安全性 → unsafe。
    let page_size: u64 = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };

    //
    // 第二步：准备接收内核数据的结构体
    //
    // vm_statistics64 是 macOS 内核用来返回内存统计信息的 C 结构体。
    // Rust 的安全保证不覆盖 FFI（外部函数接口）调用，所以整个流程需要 unsafe。
    let mut vm_stats: libc::vm_statistics64 = unsafe { mem::zeroed() };
    //                                    ^^^^^^^^^^^^^^^^^^
    // 【裸指针使用 #1】mem::zeroed() 返回一个全零初始化的值。
    //   为什么 unsafe？因为某些类型（如拥有堆内存的 String）全零是非法状态。
    //   但 vm_statistics64 是纯数据的 C 结构体（Plain Old Data），全零安全。
    //   我们必须手动保证这一点，编译器不会替我们检查。

    //
    // 第三步：调用 mach 内核函数 host_statistics64
    //
    // 函数签名（C）：
    //   kern_return_t host_statistics64(
    //       host_t          host_priv,       // 目标主机的端口
    //       host_flavor_t   flavor,          // 要查询的信息类型
    //       host_info64_t   host_info64_out, // 【输出参数】内核写入数据的指针
    //       mach_msg_type_number_t *count    // 【输入/输出参数】结构体大小（按 int32 计）
    //   );
    //
    let mut count: c_uint = mem::size_of::<libc::vm_statistics64>() as c_uint
        / mem::size_of::<c_int>() as c_uint;
    //  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //  count 的单位是"多少个 int32"，不是字节数。
    //  这是 mach API 的约定，内核靠这个值知道用户空间准备了多大的缓冲区。

    // 【为什么整个块要 unsafe？】
    // 1. mach_host_self() 是 mach 内核 trap（系统调用），Rust 无法验证返回值的安全性。
    // 2. host_statistics64 是 C FFI 函数，接受裸指针参数。
    // 3. 裸指针 &mut vm_stats 被内核写入，Rust 无法静态验证写入范围。
    //    以上任何一条都要求 unsafe。
    let host_port = unsafe { libc::mach_host_self() } as c_uint;

    let result: c_int = unsafe {
        host_statistics64(
            // 【关键】host_statistics64 需要的是 host 端口，不是 task 端口。
            // mach_host_self() 返回当前主机的特权端口。
            host_port,
            libc::HOST_VM_INFO64,      // 告诉内核：我要虚拟内存统计信息
            &mut vm_stats as *mut _ as *mut c_int,
            //  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
            //  【裸指针使用 #2】将 Rust 结构体的可变引用强制转型为 *mut c_int
            //  （即 mach 的 host_info64_t 类型）。
            //
            //  这里做了什么？
            //    &mut vm_stats              → &mut vm_statistics64（Rust 可变引用）
            //    as *mut _                  → *mut vm_statistics64（裸指针）
            //    as *mut c_int              → *mut c_int（匹配 C 函数签名的指针类型）
            //
            //  为什么必须用裸指针？
            //    C 函数不知道 Rust 的引用和生命周期，它只接受原始地址。
            //    这个指针会被内核写入数据，属于"通过指针修改外部状态"，
            //    Rust 编译器无法静态验证内核会不会越界写入 → 必须 unsafe。
            &mut count,
            //  ^^^^^^^^
            //  【裸指针使用 #3】隐式转换：&mut c_uint → *mut c_uint
            //  内核会更新这个值为实际写入的 int32 个数。
        )
    };

    //
    // 检查调用是否成功
    //
    // macOS 的 kern_return_t: 0 表示 KERN_SUCCESS，非零为错误码。
    if result != 0 {
        eprintln!("host_statistics64 调用失败，错误码: {}", result);
        std::process::exit(1);
    }

    //
    // 第四步：从原始数据计算人类可读的内存信息
    //
    // macOS 物理内存页分类（这是理解"剩余内存"的关键）：
    //
    //   wire_count        - 被内核锁住、不可换出的页（如内核数据结构）
    //   active_count      - 正在被进程使用的页
    //   inactive_count    - 最近用过但当前未活跃的页
    //                        macOS 不会急于释放这些页，而是保留内容作为缓存。
    //                        当系统需要内存时，这些页可以被直接回收复用。
    //                        → 相当于 Linux 的 "cached/buffers"，是"隐性空闲"。
    //   speculative_count - 内核推测性预读的页，优先级最低，可随时回收
    //   free_count        - 完全空闲、未分配的页
    //
    // 总物理页数 ≈ wire + active + inactive + speculative + free
    //
    // 【关键误解】直接看 free_count 会发现很小（可能只有几百 MB），
    //   但这并不意味着内存不够！macOS 的设计哲学是"闲置内存就是浪费"——
    //   它会主动把空闲内存用来做缓存（变成 inactive），需要时秒回收。
    //   所以"真正可用内存"= free + inactive + speculative。
    //   这和 macOS"活动监视器"（Activity Monitor）显示的"可用内存"一致。
    //
    // 【计算公式】
    //   可用字节数 = (free_count + inactive_count + speculative_count) × page_size
    //   可用 GB    = 可用字节数 / (1024 × 1024 × 1024)
    //
    let free_pages: u64 = vm_stats.free_count as u64;
    let inactive_pages: u64 = vm_stats.inactive_count as u64;
    let speculative_pages: u64 = vm_stats.speculative_count as u64;

    // 仅完全空闲的内存（数值通常很小，不代表真实可用量）
    let free_bytes: u64 = free_pages * page_size;
    let free_gb: f64 = free_bytes as f64 / (1024.0_f64 * 1024.0 * 1024.0);

    // 真正的"可用内存"= 空闲 + 可回收缓存 + 推测性预读
    let available_pages: u64 = free_pages + inactive_pages + speculative_pages;
    let available_bytes: u64 = available_pages * page_size;
    let available_gb: f64 = available_bytes as f64 / (1024.0_f64 * 1024.0 * 1024.0);

    // 已使用内存（不可回收的部分）
    let used_pages: u64 = (vm_stats.active_count as u64) + (vm_stats.wire_count as u64);
    let used_bytes: u64 = used_pages * page_size;
    let used_gb: f64 = used_bytes as f64 / (1024.0_f64 * 1024.0 * 1024.0);

    // 总物理内存
    let total_pages: u64 = used_pages + inactive_pages + speculative_pages + free_pages;
    let total_bytes: u64 = total_pages * page_size;
    let total_gb: f64 = total_bytes as f64 / (1024.0_f64 * 1024.0 * 1024.0);

    println!("内存页大小:       {} bytes", page_size);
    println!();
    println!("--- 原始页数（来自内核 vm_statistics64） ---");
    println!("wire (不可换出):  {:>8} 页", vm_stats.wire_count);
    println!("active (使用中):  {:>8} 页", vm_stats.active_count);
    println!("inactive (缓存): {:>8} 页", vm_stats.inactive_count);
    println!("speculative:      {:>8} 页", vm_stats.speculative_count);
    println!("free (完全空闲): {:>8} 页", vm_stats.free_count);
    println!();
    println!("--- 换算结果 ---");
    println!("总物理内存:       {:.2} GB", total_gb);
    println!("已使用(不可回收): {:.2} GB  (active + wire)", used_gb);
    println!("仅完全空闲:       {:.2} GB  (free)", free_gb);
    println!("真正可用内存:     {:.2} GB  (free + inactive + speculative)", available_gb);
    println!("占用百分比:       {:.2}%", used_gb * 100.0 / total_gb)
}
