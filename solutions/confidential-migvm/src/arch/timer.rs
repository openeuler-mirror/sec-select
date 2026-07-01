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

use super::apic::{enable_irq, readback_isenabler0, InterruptStack};
use super::idt::{register_interrupt_callback, InterruptCallback};

const TIMER_PPI: u32 = 27;

const CTL_ENABLE: u64 = 1 << 0;
const CTL_IMASK: u64 = 1 << 1;
const CTL_ISTAT: u64 = 1 << 2;

static TIMER_FREQ_HZ: AtomicU64 = AtomicU64::new(0);
static TIMEOUT_CALLBACK: spin::Once<fn()> = spin::Once::new();

fn timer_irq_handler(_stack: &mut InterruptStack) {
    let ctl: u64;
    unsafe { asm!("mrs {}, cntv_ctl_el0", out(reg) ctl); }

    if ctl & CTL_ISTAT == 0 {
        return;
    }

    unsafe { asm!("msr cntv_ctl_el0, {}", in(reg) CTL_IMASK); }
    unsafe { asm!("isb"); }

    if let Some(cb) = TIMEOUT_CALLBACK.get() {
        cb();
    }
}

pub fn init_timer() -> bool {
    let freq: u64;
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) freq); }
    TIMER_FREQ_HZ.store(freq, Ordering::Relaxed);

    crate::uart_puts("[Timer] CNTFRQ_EL0=");
    crate::uart_put_hex(freq);
    crate::uart_puts(" Hz\n");

    unsafe { asm!("msr cntv_ctl_el0, xzr"); }
    unsafe { asm!("isb"); }

    if register_interrupt_callback(
        TIMER_PPI as usize,
        InterruptCallback::new(timer_irq_handler),
    )
    .is_err()
    {
        crate::uart_puts("[Timer] ERROR: Failed to register PPI 27 callback\n");
    }

    let isen_before = readback_isenabler0() & (1u32 << TIMER_PPI);
    enable_irq(TIMER_PPI);
    let isen_after = readback_isenabler0() & (1u32 << TIMER_PPI);

    crate::uart_puts("[Timer] PPI 27 ISENABLER before=0x");
    crate::uart_put_hex(isen_before as u64);
    crate::uart_puts(" after=0x");
    crate::uart_put_hex(isen_after as u64);

    if isen_after != 0 {
        crate::uart_puts(" OK\n");
        true
    } else {
        crate::uart_puts(" STUCK\n");
        false
    }
}

pub fn schedule_timeout_us(us: u64) {
    let freq = TIMER_FREQ_HZ.load(Ordering::Relaxed);
    let ticks = (us * freq) / 1_000_000;

    if ticks > 0x7FFFFFFF {
        let cnt: u64;
        unsafe { asm!("isb", "mrs {}, cntvct_el0", out(reg) cnt); }
        unsafe { asm!("msr cntv_cval_el0, {}", in(reg) cnt.saturating_add(ticks)); }
    } else {
        unsafe { asm!("msr cntv_tval_el0, {}", in(reg) ticks); }
    }

    unsafe { asm!("msr cntv_ctl_el0, {}", in(reg) CTL_ENABLE); }
    unsafe { asm!("isb"); }
}

pub fn set_timer_callback(cb: fn()) {
    TIMEOUT_CALLBACK.call_once(|| cb);
}

pub fn cancel_timer() {
    unsafe { asm!("msr cntv_ctl_el0, {}", in(reg) CTL_IMASK); }
    unsafe { asm!("isb"); }
}

pub fn unmask_irq() {
    unsafe { asm!("msr DAIFClr, #2"); }
}
