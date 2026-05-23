//! Rust 动态链接库 — 零拷贝基准测试
//!
//! 通过 `no_mangle` + `extern "C"` 导出函数，供 Java Panama FFI 直接调用。
//! 所有操作均基于裸指针 + unsafe 块，数据始终在调用方（Java 堆外内存）中原地处理，
//! 零序列化、零额外拷贝。

use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::cmp::min;
use std::time::Instant;

// ---- FNV-1a 64-bit 常量 ----
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// 返回 Rayon 线程池大小（供 Java 端打印）
#[no_mangle]
pub extern "C" fn rust_thread_count() -> usize {
    rayon::current_num_threads()
}

// ============================================================
//  内部工具函数：XOR 加密 & FNV-1a 哈希（纯裸指针）
// ============================================================

/// 对 [ptr, ptr+len) 做逐字节 XOR 加密。
/// 密钥模式: key = ((block_seed + byte_offset) & 0xFF)
/// 其中 block_seed = block_idx * 0x517cc1b727220a95 (一个大的奇数常量)
#[inline(always)]
unsafe fn xor_encrypt_raw(ptr: *mut u8, len: usize, block_idx: usize) {
    let seed = (block_idx as u64).wrapping_mul(0x517cc1b727220a95);
    let mut i = 0;
    while i < len {
        let p = ptr.add(i);
        let key = ((seed.wrapping_add(i as u64)) & 0xFF) as u8;
        *p = (*p) ^ key;
        i += 1;
    }
}

/// 对 [ptr, ptr+len) 计算 FNV-1a 64-bit 哈希。
#[inline(always)]
unsafe fn fnv1a_hash_raw(ptr: *const u8, len: usize) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    let mut i = 0;
    while i < len {
        hash ^= (*ptr.add(i)) as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

/// 处理单个 block: 先 XOR 加密，再算哈希。
#[inline(always)]
unsafe fn process_block(data_ptr: *mut u8, result_ptr: *mut u64, block_idx: usize, block_size: usize, data_len: usize) {
    let offset = block_idx * block_size;
    let remaining = data_len - offset;
    let len = min(block_size, remaining);
    let block_ptr = data_ptr.add(offset);
    xor_encrypt_raw(block_ptr, len, block_idx);
    let hash = fnv1a_hash_raw(block_ptr, len);
    *result_ptr.add(block_idx) = hash;
}

// ============================================================
//  导出函数 1: 单线程顺序处理（Baseline）
// ============================================================

/// 顺序地处理数据：每个 block 先 XOR 加密，再算 FNV-1a 哈希。
///
/// # Safety
/// - `data_ptr` 须指向至少 `data_len` 字节的可读写内存
/// - `result_ptr` 须指向至少 `ceil(data_len/block_size) * 8` 字节的可写内存
#[no_mangle]
pub unsafe extern "C" fn rust_sequential_process(
    data_ptr: *mut u8,
    data_len: usize,
    result_ptr: *mut u64,
    block_size: usize,
) -> u64 {
    assert!(!data_ptr.is_null(), "data_ptr is null");
    assert!(!result_ptr.is_null(), "result_ptr is null");

    let block_count = (data_len + block_size - 1) / block_size;
    let start = Instant::now();

    for idx in 0..block_count {
        unsafe {
            process_block(data_ptr, result_ptr, idx, block_size, data_len);
        }
    }

    start.elapsed().as_nanos() as u64
}

// ============================================================
//  导出函数 2: Rayon 多线程并行处理（主实验）
// ============================================================

/// 与 `rust_sequential_process` 逻辑完全相同，但每个 block 由 Rayon 线程池并行处理。
/// Rust 侧收到的只是 Java 堆外内存的裸地址指针，没有任何拷贝或序列化。
///
/// # Safety
/// 同 `rust_sequential_process`。
#[no_mangle]
pub unsafe extern "C" fn rust_parallel_process(
    data_ptr: *mut u8,
    data_len: usize,
    result_ptr: *mut u64,
    block_size: usize,
) -> u64 {
    assert!(!data_ptr.is_null(), "data_ptr is null");
    assert!(!result_ptr.is_null(), "result_ptr is null");

    let block_count = (data_len + block_size - 1) / block_size;
    let start = Instant::now();

    // 将裸指针转为 usize 地址值 (usize 是 Send + Sync，可安全跨线程拷贝)
    // 各线程通过 offset 访问互不重叠的内存区域，无数据竞争
    let data_addr = data_ptr as usize;
    let result_addr = result_ptr as usize;

    (0..block_count).into_par_iter().for_each(|idx| {
        unsafe {
            process_block(data_addr as *mut u8, result_addr as *mut u64, idx, block_size, data_len);
        }
    });

    start.elapsed().as_nanos() as u64
}
