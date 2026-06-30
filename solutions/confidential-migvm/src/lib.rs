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

#![no_std]
#![no_main]

extern crate alloc;

pub mod arch;
pub mod fdt;
pub mod mm;
pub mod acpi;
pub mod logger;
pub mod rsi;
pub mod tsi;
pub mod virtio;
pub mod pci;
pub mod network;
pub mod time;
pub mod trng;

use core::arch::global_asm;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    uart_puts("PANIC: ");
    if let Some(loc) = info.location() {
        uart_puts(loc.file());
        uart_puts(":");
        uart_putc(b'0' + (loc.line() / 100) as u8);
        uart_putc(b'0' + ((loc.line() / 10) % 10) as u8);
        uart_putc(b'0' + (loc.line() % 10) as u8);
    }
    uart_puts("\n");
    loop {
        core::hint::spin_loop();
    }
}

global_asm!(include_str!("arch/exception.S"));

const PL011_UART_BASE: usize = 0x0900_0000;

pub fn uart_putc(c: u8) {
    unsafe {
        core::ptr::write_volatile(PL011_UART_BASE as *mut u8, c);
    }
}

pub fn uart_puts(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            uart_putc(b'\r');
        }
        uart_putc(b);
    }
}

pub fn uart_put_hex(v: u64) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    uart_puts("0x");
    for i in (0..16).rev() {
        let nibble = ((v >> (i * 4)) & 0xF) as usize;
        uart_putc(HEX[nibble]);
    }
}

#[no_mangle]
pub extern "C" fn handle_sync_exception(stack: *mut u8) {
    let esr: u64;
    let far: u64;
    unsafe {
        core::arch::asm!("mrs {}, esr_el1", out(reg) esr);
        core::arch::asm!("mrs {}, far_el1", out(reg) far);
    }

    let ec = (esr >> 26) & 0x3F;

    if (ec == 0x17 || ec == 0x00) && rsi::is_smc_testing() {
        rsi::set_smc_testing(false);
        unsafe {
            let sp = stack as *mut u64;
            sp.write_volatile(0xFFFFFFFF);
            let elr_ptr = sp.add(248 / 8);
            let elr = elr_ptr.read_volatile();
            elr_ptr.write_volatile(elr + 4);
        }
        return;
    }

    uart_puts("SYNC exception\n");
    uart_puts("  ESR: ");
    uart_put_hex(esr);
    uart_puts("  FAR: ");
    uart_put_hex(far);
    uart_puts("\n");
    let _ = stack;
    loop {
        core::hint::spin_loop();
    }
}

#[no_mangle]
pub extern "C" fn handle_irq_exception(stack: *mut u8) {
    use arch::apic::{acknowledge_irq, end_of_interrupt, deactivate_irq, is_spurious, InterruptStack};
    use arch::idt::dispatch_interrupt;

    let intid = acknowledge_irq();

    if is_spurious(intid) {
        return;
    }

    end_of_interrupt(intid);

    if intid < 1020 {
        let stack_ref = unsafe { &mut *(stack as *mut InterruptStack) };
        dispatch_interrupt(intid as usize, stack_ref);
    }

    deactivate_irq(intid);
}

#[no_mangle]
pub extern "C" fn handle_fiq_exception(_stack: *mut u8) {
    uart_puts("FIQ\n");
}

#[no_mangle]
pub extern "C" fn handle_serror_exception(_stack: *mut u8) {
    uart_puts("SError\n");
    loop {
        core::hint::spin_loop();
    }
}
