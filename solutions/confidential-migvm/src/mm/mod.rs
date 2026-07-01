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

pub mod layout;
pub mod paging;
pub mod shared;
pub mod heap;

pub use paging::init_dma_shared_pool;

use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;

pub const MEMORY_TYPE_RAM: u32 = 1;
pub const MEMORY_TYPE_MMIO: u32 = 2;
pub const MEMORY_TYPE_SHARED: u32 = 3;

#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub addr: u64,
    pub size: u64,
    pub region_type: u32,
}

impl MemoryRegion {
    pub fn new(addr: u64, size: u64, region_type: u32) -> Self {
        Self {
            addr,
            size,
            region_type,
        }
    }

    pub fn end(&self) -> u64 {
        self.addr + self.size
    }
}

lazy_static! {
    pub static ref MEMORY_MAP: Mutex<Vec<MemoryRegion>> = Mutex::new(Vec::new());
}

pub fn init_memory_map() {
    let mut map = MEMORY_MAP.lock();
    map.clear();

    map.push(MemoryRegion::new(0x4000_0000, 0x4000_0000, MEMORY_TYPE_RAM));

    map.push(MemoryRegion::new(0x0800_0000, 0x00C0_0000, MEMORY_TYPE_MMIO));
    map.push(MemoryRegion::new(0x0900_0000, 0x0000_1000, MEMORY_TYPE_MMIO));
}

pub fn init_memory_map_from_fdt(fdt_memory: &[crate::fdt::MemRegion]) {
    let mut map = MEMORY_MAP.lock();
    map.clear();

    for region in fdt_memory {
        map.push(MemoryRegion::new(region.base, region.size, MEMORY_TYPE_RAM));
    }

    map.push(MemoryRegion::new(0x0800_0000, 0x00C0_0000, MEMORY_TYPE_MMIO));
    map.push(MemoryRegion::new(0x0900_0000, 0x0000_1000, MEMORY_TYPE_MMIO));
}

pub fn accept_all_ram() {
    if !crate::rsi::is_realm_world() {
        return;
    }

    let map = MEMORY_MAP.lock();
    for region in map.iter() {
        if region.region_type == MEMORY_TYPE_RAM {
            let aligned_start = (region.addr + 0xFFF) & !0xFFF;
            let aligned_end = region.end() & !0xFFF;
            if aligned_start < aligned_end {
                if crate::rsi::accept_memory(aligned_start, aligned_end).is_err() {
                    crate::uart_puts("[MM] Failed to accept RAM: ");
                    crate::uart_put_hex(aligned_start);
                    crate::uart_puts("-");
                    crate::uart_put_hex(aligned_end);
                    crate::uart_puts("\n");
                }
            }
        }
    }
}
