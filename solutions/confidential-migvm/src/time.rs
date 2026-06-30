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
use crate::arch::timer;

static SYS_TICK_MS: AtomicU64 = AtomicU64::new(0);
static TIMER_FREQ_HZ: AtomicU64 = AtomicU64::new(0);
const TICK_INTERVAL_US: u64 = 1000;

const CTL_ENABLE: u64 = 1 << 0;
const CTL_IMASK: u64 = 1 << 1;
const CTL_ISTAT: u64 = 1 << 2;

pub fn init_sys_tick() {
    let freq: u64;
    let cntvct: u64;
    unsafe {
        asm!("mrs {}, cntfrq_el0", out(reg) freq);
        asm!("mrs {}, cntvct_el0", out(reg) cntvct);
    }
    TIMER_FREQ_HZ.store(freq, Ordering::Relaxed);

    crate::uart_puts("[Time] CNTFRQ=");
    crate::uart_put_hex(freq);
    crate::uart_puts(" CNTVCT=");
    crate::uart_put_hex(cntvct);

    let ticks = (TICK_INTERVAL_US * freq) / 1_000_000;
    unsafe { asm!("msr cntv_tval_el0, {}", in(reg) ticks); }
    unsafe { asm!("msr cntv_ctl_el0, {}", in(reg) CTL_ENABLE); }
    unsafe { asm!("isb"); }

    timer::unmask_irq();
    crate::uart_puts(" [Time] Poll-based tick started (1ms)\n");
}

fn poll_and_update() -> bool {
    let ctl: u64;
    unsafe { asm!("mrs {}, cntv_ctl_el0", out(reg) ctl); }
    if ctl & CTL_ISTAT == 0 {
        return false;
    }
    let freq = TIMER_FREQ_HZ.load(Ordering::Relaxed);
    let ticks = (TICK_INTERVAL_US * freq) / 1_000_000;
    unsafe {
        asm!("msr cntv_ctl_el0, {}", in(reg) CTL_IMASK);
        asm!("isb");
        asm!("msr cntv_tval_el0, {}", in(reg) ticks);
        asm!("msr cntv_ctl_el0, {}", in(reg) CTL_ENABLE);
        asm!("isb");
    }
    SYS_TICK_MS.fetch_add(1, Ordering::SeqCst);
    true
}

pub fn now_ms() -> u64 {
    poll_and_update();
    SYS_TICK_MS.load(Ordering::SeqCst)
}

pub fn quiesce_before_smc() {
    unsafe {
        asm!("msr cntv_ctl_el0, {}", in(reg) CTL_IMASK);
        asm!("isb");
        asm!("msr cntv_ctl_el0, xzr");
        asm!("isb");
    }
}

pub fn resume_after_smc() {
    let freq = TIMER_FREQ_HZ.load(Ordering::Relaxed);
    let ticks = (TICK_INTERVAL_US * freq) / 1_000_000;
    unsafe {
        asm!("msr cntv_tval_el0, {}", in(reg) ticks);
        asm!("msr cntv_ctl_el0, {}", in(reg) CTL_ENABLE);
        asm!("isb");
    }
}

pub fn wait_ms(ms: u64) {
    let target = now_ms().saturating_add(ms);
    while now_ms() < target {
        core::hint::spin_loop();
    }
}
