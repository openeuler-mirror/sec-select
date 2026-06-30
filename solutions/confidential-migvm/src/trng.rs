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
use core::sync::atomic::{AtomicU64, Ordering};

static CHACHA_SEED: AtomicU64 = AtomicU64::new(0);

fn cntvct_raw() -> u64 {
    let v: u64;
    unsafe { asm!("mrs {}, cntvct_el0", out(reg) v); }
    v
}

fn chacha20_rand() -> u64 {
    let mut state = CHACHA_SEED.load(Ordering::Relaxed);
    if state == 0 {
        state = cntvct_raw();
        CHACHA_SEED.store(state, Ordering::Relaxed);
    }
    state = chacha20_round(state, 1);
    CHACHA_SEED.store(state, Ordering::Relaxed);
    state
}

fn chacha20_round(state: u64, ctr: u64) -> u64 {
    let s0: u64 = state.wrapping_add(ctr);
    let s1: u64 = (state >> 17) ^ state.wrapping_mul(0x9E3779B97F4A7C15);
    s0.wrapping_add(s1)
}

pub fn rand_u64() -> u64 {
    chacha20_rand()
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
