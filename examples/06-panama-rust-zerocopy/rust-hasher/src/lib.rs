//! Rust 动态链接库 — 接收 Java Panama 传来的堆外裸指针，直接操作内存
//!
//! 导出函数:
//!   - rust_thread_count()        → 返回 Rayon 线程数
//!   - rust_sequential_process()  → 单线程逐块处理
//!   - rust_parallel_process()    → Rayon 多线程并行处理
//!
//! 每个块的处理逻辑:
//!   Phase 1: XOR 加密 (position-dependent key)
//!   Phase 2: FNV-1a 64-bit 哈希

use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::cmp::min;
use std::time::Instant;

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[no_mangle]
pub extern "C" fn rust_thread_count() -> usize {
    rayon::current_num_threads()
}

/// 对 [ptr, ptr+len) 逐字节 XOR 加密
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

/// 对 [ptr, ptr+len) 计算 FNV-1a 64-bit 哈希
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

/// 处理单个 block
#[inline(always)]
unsafe fn process_block(data: usize, results: usize, idx: usize, block_size: usize, data_len: usize) {
    let offset = idx * block_size;
    let len = min(block_size, data_len - offset);
    let ptr = (data + offset) as *mut u8;
    xor_encrypt_raw(ptr, len, idx);
    let hash = fnv1a_hash_raw(ptr, len);
    *((results as *mut u64).add(idx)) = hash;
}

// ---- 顺序处理 ----

#[no_mangle]
pub unsafe extern "C" fn rust_sequential_process(
    data_ptr: *mut u8,
    data_len: usize,
    result_ptr: *mut u64,
    block_size: usize,
) -> u64 {
    let data_addr = data_ptr as usize;
    let result_addr = result_ptr as usize;
    let block_count = (data_len + block_size - 1) / block_size;
    let start = Instant::now();

    for idx in 0..block_count {
        unsafe { process_block(data_addr, result_addr, idx, block_size, data_len) };
    }

    start.elapsed().as_nanos() as u64
}

// ---- Rayon 并行处理 ----

#[no_mangle]
pub unsafe extern "C" fn rust_parallel_process(
    data_ptr: *mut u8,
    data_len: usize,
    result_ptr: *mut u64,
    block_size: usize,
) -> u64 {
    let data_addr = data_ptr as usize;
    let result_addr = result_ptr as usize;
    let block_count = (data_len + block_size - 1) / block_size;
    let start = Instant::now();

    (0..block_count).into_par_iter().for_each(|idx| {
        unsafe { process_block(data_addr, result_addr, idx, block_size, data_len) };
    });

    start.elapsed().as_nanos() as u64
}
