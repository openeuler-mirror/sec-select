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

use crate::mm;
use crate::mm::layout::RuntimeLayout;
#[cfg(feature = "cca")]
use crate::rsi;
use core::arch::asm;

extern "C" {
    static mut __bss_start: u8;
    static mut __bss_end: u8;
    static __stack_end: u8;
}

static mut MAIN_FN: Option<fn()> = None;

pub fn pre_init(_fdt_ptr: u64, layout: &RuntimeLayout, accept_memory: bool) {
    #[cfg(not(feature = "cca"))]
    let _ = accept_memory;
    unsafe {
        let bss_start = core::ptr::addr_of!(__bss_start) as *const u8 as usize;
        let bss_end = core::ptr::addr_of!(__bss_end) as *const u8 as usize;
        core::ptr::write_bytes(bss_start as *mut u8, 0, bss_end - bss_start);
    }

    mm::paging::init_page_tables(layout.page_table_size);

    mm::heap::init_heap(layout.heap_size);

    unsafe {
        let sp = core::ptr::addr_of!(__stack_end) as *const u8 as u64;
        asm!("mov sp, {}", in(reg) sp);
    }

    #[cfg(feature = "cca")]
    if rsi::init_rsi() {
        crate::uart_puts("[RSI] Realm detected, CCA guest mode\n");

        if accept_memory {
            if rsi::accept_memory(0x4000_0000, 0x8000_0000).is_ok() {
                crate::uart_puts("[RSI] RAM memory accepted\n");
            } else {
                crate::uart_puts("[RSI] WARNING: Memory accept failed (non-CCA mode?)\n");
            }
        }

        let _ = rsi::mark_mmio_protected(0x0000_0000, 0x0A00_0000);
        let _ = rsi::mark_mmio_protected(0x0900_0000, 0x0900_1000);
    }
    #[cfg(feature = "cca")]
    {
        if !rsi::is_realm_world() {
            crate::uart_puts("[RSI] Not in Realm, running in normal VM mode\n");
        }
    }

    log::info!("AArch64 pre_init done");
}

pub fn init(layout: &RuntimeLayout, main: fn()) -> ! {
    unsafe {
        MAIN_FN = Some(main);
    }

    let _ = layout;

    log::info!("AArch64 init done, jumping to main");

    unsafe {
        if let Some(main_fn) = MAIN_FN {
            main_fn();
        }
    }

    loop {
        unsafe { asm!("wfi") }
    }
}
