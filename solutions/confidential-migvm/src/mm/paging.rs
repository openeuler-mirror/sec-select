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

use crate::rsi;

extern "C" {
    static __page_table_start: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
}

const ENTRY_COUNT: usize = 512;

const PTE_VALID: u64 = 1 << 0;
const PTE_TABLE: u64 = 1 << 1;
const PTE_AF: u64 = 1 << 10;
const PTE_SH_INNER: u64 = 3 << 8;
const PTE_SH_OUTER: u64 = 2 << 8;
const PTE_ATTRINDX_SHIFT: u64 = 2;

const MT_NORMAL: u64 = 0;
const MT_NORMAL_NC: u64 = 1;
const MT_DEVICE_NGNRE: u64 = 2;

const PTE_ATTR_NORMAL: u64 = MT_NORMAL << PTE_ATTRINDX_SHIFT;
const PTE_ATTR_NORMAL_NC: u64 = MT_NORMAL_NC << PTE_ATTRINDX_SHIFT;
const PTE_ATTR_DEVICE: u64 = MT_DEVICE_NGNRE << PTE_ATTRINDX_SHIFT;

const PTE_PXN: u64 = 1 << 53;
const PTE_UXN: u64 = 1 << 54;
const PTE_DBM: u64 = 1 << 51;
const PTE_AP_RO: u64 = 1 << 7;
const PTE_AP_RW_EL1: u64 = (1 << 7) | (1 << 6);

#[cfg(feature = "virtcca")]
const CVM_PTE_NS: u64 = 1 << 5;

const PTE_PAGE: u64 = PTE_VALID | PTE_TABLE;

const PROT_NORMAL: u64 = PTE_AF | PTE_SH_INNER | PTE_ATTR_NORMAL | PTE_PXN | PTE_UXN | PTE_DBM | PTE_AP_RW_EL1;
const PROT_NORMAL_NC: u64 = PTE_AF | PTE_SH_OUTER | PTE_ATTR_NORMAL_NC | PTE_PXN | PTE_UXN | PTE_DBM | PTE_AP_RW_EL1;
const PROT_DEVICE: u64 = PTE_AF | PTE_ATTR_DEVICE | PTE_PXN | PTE_UXN | PTE_DBM | PTE_AP_RW_EL1;

const PROT_KERNEL_ROX: u64 = PTE_AF | PTE_SH_INNER | PTE_ATTR_NORMAL | PTE_UXN | PTE_AP_RO;
const PROT_KERNEL_RO:  u64 = PTE_AF | PTE_SH_INNER | PTE_ATTR_NORMAL | PTE_PXN | PTE_UXN | PTE_AP_RO;
const PROT_KERNEL_RW:  u64 = PROT_NORMAL;

fn page_table_base() -> *mut u64 {
    unsafe { &__page_table_start as *const u8 as *mut u64 }
}

fn l0_table() -> *mut u64 {
    page_table_base()
}

fn l1_table() -> *mut u64 {
    unsafe { page_table_base().add(ENTRY_COUNT) }
}

fn l2_table_for(vaddr: u64) -> *mut u64 {
    let l1_idx = ((vaddr >> 30) & 0x1FF) as usize;
    unsafe { page_table_base().add(2 * ENTRY_COUNT + l1_idx * ENTRY_COUNT) }
}

pub fn init_page_tables(pt_size: usize) {
    let pt_base = page_table_base();
    let pt_entries = pt_size / 8;

    unsafe {
        for i in 0..pt_entries {
            core::ptr::write_volatile(pt_base.add(i), 0);
        }
    }

    let l0 = l0_table();
    let l1 = l1_table();
    unsafe {
        core::ptr::write_volatile(l0, (l1 as u64) | PTE_VALID | PTE_TABLE);
    }

    let text_start  = unsafe { &__text_start as *const u8 as u64 };
    let text_end    = unsafe { &__text_end   as *const u8 as u64 };
    let ro_start    = unsafe { &__rodata_start as *const u8 as u64 };
    let ro_end      = unsafe { &__rodata_end   as *const u8 as u64 };
    let data_start  = unsafe { &__data_start as *const u8 as u64 };
    let data_end    = unsafe { &__data_end   as *const u8 as u64 };

    let ram_base:   u64 = 0x4000_0000;
    let ram_size:   u64 = 0x4000_0000;
    let first_2mb:  u64 = ram_base + 0x20_0000;

    map_2mb_blocks(first_2mb, first_2mb, ram_size - 0x20_0000, PROT_NORMAL);

    let l3 = alloc_l3_table();
    let l2 = l2_table_for(ram_base);
    let l3_pa = l3 as u64;
    unsafe {
        core::ptr::write_volatile(l2.add(0), l3_pa | PTE_VALID | PTE_TABLE);
    }
    for i in 0..ENTRY_COUNT {
        let va = ram_base + (i as u64) * 0x1000;
        let prot = if va >= text_start && va < text_end {
            PROT_KERNEL_ROX
        } else if va >= ro_start && va < ro_end {
            PROT_KERNEL_RO
        } else if va >= data_start && va < data_end {
            PROT_KERNEL_RW
        } else {
            PROT_KERNEL_RW
        };
        unsafe {
            core::ptr::write_volatile(l3.add(i), va | PTE_VALID | prot);
        }
    }

    map_2mb_blocks(0x0000_0000, 0x0000_0000, 0x4000_0000, PROT_DEVICE);

    unsafe {
        asm!("dsb ishst", "tlbi vmalle1", "dsb ish", "isb");
    }
}

fn map_2mb_blocks(vaddr: u64, paddr: u64, size: u64, prot: u64) {
    let mut v = vaddr;
    let mut p = paddr;
    let end = vaddr + size;
    while v < end {
        let l2 = l2_table_for(v);
        let l2_idx = ((v >> 21) & 0x1FF) as usize;
        unsafe {
            core::ptr::write_volatile(l2.add(l2_idx), p | PTE_VALID | prot);
        }
        v += 0x20_0000;
        p += 0x20_0000;
    }
}

pub fn map_range(vaddr: u64, paddr: u64, size: u64, prot: u64) {
    let mut v = vaddr;
    let mut p = paddr;
    let end = vaddr + size;

    while v < end {
        let l2 = l2_table_for(v);
        let l2_idx = ((v >> 21) & 0x1FF) as usize;

        let entry = p | PTE_VALID | prot;

        unsafe {
            core::ptr::write_volatile(l2.add(l2_idx), entry);
        }

        v += 0x20_0000;
        p += 0x20_0000;
    }

    unsafe {
        asm!("dsb ish");
        asm!("isb");
    }
}

pub fn map_device_region(_phys: u64, _size: u64) {}

pub fn map_2mb_device(vaddr: u64, paddr: u64) {
    let l1 = l1_table();
    let l1_idx = ((vaddr - 0x40000000) >> 21) as usize;
    if l1_idx >= ENTRY_COUNT {
        return;
    }
    let entry = (paddr & 0xFFFF_FFFF_FFE0_0000u64) | (0x701u64);
    unsafe {
        core::ptr::write_volatile(l1.add(l1_idx), entry);
        asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb");
    }
}

pub fn map_bar_space(vaddr: u64, paddr: u64) {
    let l1 = l1_table();
    let l1_idx = ((vaddr - 0x40000000) >> 21) as usize;
    if l1_idx >= ENTRY_COUNT {
        return;
    }
    let entry = (paddr & 0xFFFF_FFFF_FFE0_0000u64) | (0x609u64);
    unsafe {
        core::ptr::write_volatile(l1.add(l1_idx), entry);
        asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb");
    }
}

pub fn map_page(vaddr: u64, paddr: u64, prot: u64) {
    let l2 = l2_table_for(vaddr);
    let l2_idx = ((vaddr >> 21) & 0x1FF) as usize;

    let l3 = unsafe {
        let entry = core::ptr::read_volatile(l2.add(l2_idx));
        if entry & PTE_TABLE != 0 {
            (entry & !0x1FF) as *mut u64
        } else {
            let l3_base = alloc_l3_table();
            let old = core::ptr::read_volatile(l2.add(l2_idx));
            if old & PTE_VALID != 0 {
                core::ptr::write_volatile(l2.add(l2_idx), old | PTE_TABLE);
                (old & !0x1FF) as *mut u64
            } else {
                let entry = l3_base as u64 | PTE_VALID | PTE_TABLE;
                core::ptr::write_volatile(l2.add(l2_idx), entry);
                l3_base
            }
        }
    };

    let l3_idx = ((vaddr >> 12) & 0x1FF) as usize;
    let entry = paddr | PTE_PAGE | prot;

    unsafe {
        core::ptr::write_volatile(l3.add(l3_idx), entry);
        asm!("dsb ish");
        asm!("isb");
    }
}

fn alloc_l3_table() -> *mut u64 {
    static mut L3_NEXT: usize = 0;
    unsafe {
        let base = page_table_base();
        let l3_offset = 4 * ENTRY_COUNT + L3_NEXT * ENTRY_COUNT;
        L3_NEXT += 1;
        let table = base.add(l3_offset);
        for i in 0..ENTRY_COUNT {
            core::ptr::write_volatile(table.add(i), 0);
        }
        table
    }
}

pub fn unmap_page(vaddr: u64) {
    let l2 = l2_table_for(vaddr);
    let l2_idx = ((vaddr >> 21) & 0x1FF) as usize;

    let l2_entry = unsafe { core::ptr::read_volatile(l2.add(l2_idx)) };
    if l2_entry & PTE_TABLE == 0 {
        return;
    }

    let l3 = (l2_entry & !0x1FF) as *mut u64;
    let l3_idx = ((vaddr >> 12) & 0x1FF) as usize;

    unsafe {
        core::ptr::write_volatile(l3.add(l3_idx), 0);
        asm!("dsb ish");
        asm!("isb");
    }
}

pub fn virt_to_phys(vaddr: u64) -> Option<u64> {
    let l2 = l2_table_for(vaddr);
    let l2_idx = ((vaddr >> 21) & 0x1FF) as usize;

    let l2_entry = unsafe { core::ptr::read_volatile(l2.add(l2_idx)) };
    if l2_entry & PTE_VALID == 0 {
        return None;
    }

    if l2_entry & PTE_TABLE != 0 {
        let l3 = (l2_entry & !0x1FF) as *mut u64;
        let l3_idx = ((vaddr >> 12) & 0x1FF) as usize;
        let l3_entry = unsafe { core::ptr::read_volatile(l3.add(l3_idx)) };
        if l3_entry & PTE_VALID == 0 {
            return None;
        }
        let paddr = l3_entry & !0x1FF;
        let offset = vaddr & 0xFFF;
        Some(paddr | offset)
    } else {
        let paddr = l2_entry & !0x1FF;
        let offset = vaddr & 0x1FFFFF;
        Some(paddr | offset)
    }
}

pub fn pgprot_encrypted(prot: u64) -> u64 {
    prot & !rsi::prot_ns_shared()
}

pub fn pgprot_decrypted(prot: u64) -> u64 {
    prot | rsi::prot_ns_shared()
}

pub fn map_shared_page(vaddr: u64, paddr: u64) {
    let prot = pgprot_decrypted(PROT_NORMAL_NC);
    map_page(vaddr, paddr, prot);
}

pub fn map_protected_page(vaddr: u64, paddr: u64) {
    let prot = pgprot_encrypted(PROT_NORMAL);
    map_page(vaddr, paddr, prot);
}

pub fn map_shared_range(vaddr: u64, paddr: u64, size: u64) {
    let prot = pgprot_decrypted(PROT_NORMAL_NC);
    map_range(vaddr, paddr, size, prot);
}

pub fn init_dma_shared_pool(mem_base: u64, mem_size: u64) -> Option<(u64, u64, u64)> {
    const DMA_POOL_SIZE: u64 = 0x200000;
    const DMA_POOL_VA: u64 = 0x70000000;

    let pool_pa = mem_base + mem_size - DMA_POOL_SIZE;

    if pool_pa < mem_base + (8 * 0x100000) {
        return None;
    }

    if crate::rsi::is_realm_world() {
        if crate::rsi::mark_shared(pool_pa, pool_pa + DMA_POOL_SIZE).is_err() {
            return None;
        }
    }

    map_shared_range(DMA_POOL_VA, pool_pa, DMA_POOL_SIZE);

    crate::virtio::init_dma_pool_shared(DMA_POOL_VA, DMA_POOL_SIZE as usize);

    crate::uart_puts("[DMA] Shared pool: PA=0x");
    crate::uart_put_hex(pool_pa);
    crate::uart_puts(" VA=0x");
    crate::uart_put_hex(DMA_POOL_VA);
    crate::uart_puts(" size=0x");
    crate::uart_put_hex(DMA_POOL_SIZE);
    crate::uart_puts("\n");

    Some((pool_pa, DMA_POOL_VA, DMA_POOL_SIZE))
}

#[cfg(feature = "virtcca")]
pub fn mark_range_shared_virtcca(vaddr: u64, size: u64) {
    let l1 = l1_table();
    let mut v = vaddr;
    let end = vaddr + size;

    while v < end {
        let l1_idx = ((v - 0x40000000) >> 21) as usize;
        if l1_idx < ENTRY_COUNT {
            unsafe {
                let entry = core::ptr::read_volatile(l1.add(l1_idx));
                if entry & PTE_VALID != 0 {
                    core::ptr::write_volatile(l1.add(l1_idx), entry | CVM_PTE_NS);
                }
            }
        }
        v += 0x20_0000;
    }

    unsafe {
        asm!("dsb ishst");
        asm!("tlbi vmalle1is");
        asm!("dsb ish");
        asm!("isb");
    }
}
