/*
 * Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
 * Global Trust Authority is licensed under the Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL v2.
 * You may obtain a copy of Mulan PSL v2 at:
 *     http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY KIND, EITHER EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR
 * PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use core::arch::asm;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

// RNG 模式：0=未初始化，1=RNDR 硬件真随机，2=软件 PRNG 回退
static RNG_MODE: AtomicU8 = AtomicU8::new(0);

// 回退 PRNG 状态（仅 RNDR 不可用时使用，例如 TCG 仿真测试环境）
static FALLBACK_STATE: AtomicU64 = AtomicU64::new(0);

/// 读取虚拟计数器 cntvct_el0 作为回退熵源
fn cntvct_raw() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, cntvct_el0", out(reg) v, options(nostack)); }
    v
}

/// 检测 RNDR 指令是否实现（ID_AA64ISAR0_EL1.RNDR 字段，bits[63:60]）。
/// 未实现时执行 mrs RNDR 会触发 UNDEFINED 异常，因此必须先检测。
fn rndr_implemented() -> bool {
    let id: u64;
    unsafe { asm!("mrs {}, ID_AA64ISAR0_EL1", out(reg) id, options(nostack)); }
    ((id >> 60) & 0xf) != 0
}

/// 读取硬件真随机数。在 CCA Realm / VirtCCA 内 RNDR 返回真随机数。
/// RNDR 失败时（随机源未就绪）置 V 条件标志并返回 None。
/// 额外过滤 0 值：真随机返回 0 的概率为 2^-64，可视为失败。
fn rndr_read() -> Option<u64> {
    let v: u64;
    let ok: u64;
    unsafe {
        asm!(
            "mrs {v}, S3_3_C2_C4_0",   // RNDR (Random Number)
            "cset {ok}, vc",            // RNDR 成功时 V=0，vc(V clear)为真
            v = out(reg) v,
            ok = out(reg) ok,
            options(nostack),
        );
    }
    if ok != 0 && v != 0 { Some(v) } else { None }
}

/// 读取硬件真随机数（重播种版本）。RNDRRS 强制从真随机源重新采样，
/// 熵质量高于 RNDR 但更慢。与 RNDR 共用 ID_AA64ISAR0_EL1.RNDR 字段，
/// 字段非 0 即表示两者都实现。
/// 失败时返回 None。
///
/// 注意：部分 VirtCCA TMM 实现对 RNDRRS 的 V 条件标志处理有误——
/// 随机源未就绪时应置 V=1（失败），却误置 V=0（成功）并返回 0。
/// 因此这里把 V=0 但返回值为 0 的结果也视为失败。
fn rndrrs_read() -> Option<u64> {
    let v: u64;
    let ok: u64;
    unsafe {
        asm!(
            "mrs {v}, S3_3_C2_C4_1",   // RNDRRS (Random Number, Reseeded)
            "cset {ok}, vc",
            v = out(reg) v,
            ok = out(reg) ok,
            options(nostack),
        );
    }
    if ok != 0 && v != 0 { Some(v) } else { None }
}

/// 初始化随机数发生器。应在启动早期（logger 之后）调用一次。
/// 返回 true 表示 RNDR 硬件真随机可用。
pub fn init() -> bool {
    if rndr_implemented() {
        // 重试几次，避免单次瞬时 0 / 未就绪导致误判
        for _ in 0..8 {
            if rndr_read().is_some() {
                RNG_MODE.store(1, Ordering::Relaxed);
                return true;
            }
        }
    }
    // 回退：用 cntvct 播种 splitmix64
    let seed = cntvct_raw() | 1; // |1 避免 0 状态
    FALLBACK_STATE.store(seed, Ordering::Relaxed);
    RNG_MODE.store(2, Ordering::Relaxed);
    false
}

/// 查询 RNDR 硬件真随机是否启用
pub fn rndr_enabled() -> bool {
    RNG_MODE.load(Ordering::Relaxed) == 1
}

/// 探测 RNDRRS 是否可用。仅用于启动期诊断，不影响主路径。
/// 返回 Some(v) 表示 RNDRRS 读取成功（v 为样本值），None 表示不可用或失败。
pub fn rndrrs_probe() -> Option<u64> {
    if !rndr_implemented() {
        return None;
    }
    rndrrs_read()
}

/// 探测 RNDR 单次读取，仅用于诊断。
pub fn rndr_probe() -> Option<u64> {
    if !rndr_implemented() {
        return None;
    }
    rndr_read()
}

/// 软件回退 PRNG（splitmix64），仅用于无 RNDR 的测试环境。
fn fallback_rand() -> u64 {
    let mut x = FALLBACK_STATE.load(Ordering::Relaxed);
    if x == 0 {
        // init 未调用时的 lazy 初始化
        x = cntvct_raw() | 1;
        if x == 0 { x = 0xDEADBEEFCAFEBABE; }
        FALLBACK_STATE.store(x, Ordering::Relaxed);
    }
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    FALLBACK_STATE.store(x, Ordering::Relaxed);
    // splitmix64 finalizer
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    z
}

/// 生成一个 64-bit 随机数。
/// 优先级：RNDRRS（重播种真随机）→ RNDR（真随机）→ 软件 PRNG 回退。
/// RNDRRS 随机源间歇未就绪时最多重试 8 次，拿到一个有效值即用；
/// 8 次全失败则降级到 RNDR；RNDR 也失败则用软件 PRNG。
pub fn rand_u64() -> u64 {
    if RNG_MODE.load(Ordering::Relaxed) == 1 {
        // 1) 首选 RNDRRS，重试 8 次
        for _ in 0..8 {
            if let Some(v) = rndrrs_read() {
                return v;
            }
        }
        // 2) RNDRRS 连续失败，降级到 RNDR
        if let Some(v) = rndr_read() {
            return v;
        }
        // 3) RNDR 也瞬时失败，本次回退软件 PRNG
    }
    fallback_rand()
}

pub fn fill_random(buf: &mut [u8]) {
    let mut i = 0;
    while i + 8 <= buf.len() {
        let val = rand_u64();
        buf[i..i + 8].copy_from_slice(&val.to_le_bytes());
        i += 8;
    }
    if i < buf.len() {
        let val = rand_u64();
        let rem = buf.len() - i;
        buf[i..].copy_from_slice(&val.to_le_bytes()[..rem]);
    }
}

pub fn u64_seed() -> u64 {
    rand_u64()
}
