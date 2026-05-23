use libc::{c_int, c_uint, host_statistics64};
use std::mem;

fn main() {
    let page_size: u64 = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };

    let mut vm_stats: libc::vm_statistics64 = unsafe { mem::zeroed() };

    let mut count: c_uint = mem::size_of::<libc::vm_statistics64>() as c_uint
        / mem::size_of::<c_int>() as c_uint;

    let host_port = unsafe { libc::mach_host_self() } as c_uint;

    let result: c_int = unsafe {
        host_statistics64(
            host_port,
            libc::HOST_VM_INFO64,
            &mut vm_stats as *mut _ as *mut c_int,
            &mut count,
        )
    };

    if result != 0 {
        eprintln!("host_statistics64 调用失败，错误码: {}", result);
        std::process::exit(1);
    }

    let free_pages: u64 = vm_stats.free_count as u64;
    let inactive_pages: u64 = vm_stats.inactive_count as u64;
    let speculative_pages: u64 = vm_stats.speculative_count as u64;

    let free_bytes: u64 = free_pages * page_size;
    let free_gb: f64 = free_bytes as f64 / (1024.0_f64 * 1024.0 * 1024.0);

    let available_pages: u64 = free_pages + inactive_pages + speculative_pages;
    let available_bytes: u64 = available_pages * page_size;
    let available_gb: f64 = available_bytes as f64 / (1024.0_f64 * 1024.0 * 1024.0);

    let used_pages: u64 = (vm_stats.active_count as u64) + (vm_stats.wire_count as u64);
    let used_bytes: u64 = used_pages * page_size;
    let used_gb: f64 = used_bytes as f64 / (1024.0_f64 * 1024.0 * 1024.0);

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
    println!("占用百分比:       {:.2}%", used_gb * 100.0 / total_gb);
}
