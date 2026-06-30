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

use crate::uart_puts;
use crate::uart_put_hex;
use alloc::vec::Vec;
use core::ptr;

const PCI_VENDOR_ID: u16 = 0x00;
const PCI_DEVICE_ID: u16 = 0x02;
const PCI_HEADER_TYPE: u16 = 0x0E;
const PCI_BAR0: u16 = 0x10;
const PCI_BAR5: u16 = 0x24;

pub const PCI_BAR_IO: u32 = 0x1;
pub const PCI_BAR_MEM32: u32 = 0x0;
pub const PCI_BAR_MEM64: u32 = 0x4;

static mut ECAM_PHYS: u64 = 0;
static mut ECAM_VA: u64 = 0;

const ECAM_VA_BASE: u64 = 0x50000000;

pub fn init_ecam(phys_base: u64) {
    unsafe {
        ECAM_PHYS = phys_base;
        ECAM_VA = ECAM_VA_BASE;
    }
    crate::mm::paging::map_2mb_device(ECAM_VA_BASE, phys_base);

    uart_puts("[PCI] ECAM VA=0x");
    uart_put_hex(ECAM_VA_BASE);
    uart_puts(" raw[0]=0x");
    let raw = unsafe { core::ptr::read_volatile(ECAM_VA_BASE as *const u32) };
    uart_put_hex(raw as u64);
    uart_puts(" raw[0x100000]=0x");
    let raw2 = unsafe { core::ptr::read_volatile((ECAM_VA_BASE + 0x100000) as *const u32) };
    uart_put_hex(raw2 as u64);
    uart_puts(" dev1=0x");
    let dev1 = unsafe { core::ptr::read_volatile((ECAM_VA_BASE + 0x8000) as *const u32) };
    uart_put_hex(dev1 as u64);
    uart_puts("\n");
}

pub fn ecam_addr(bus: u8, dev: u8, func: u8, offset: u16) -> u64 {
    let phys_off = ((bus as u64) << 20) | ((dev as u64) << 15) | ((func as u64) << 12) | offset as u64;
    unsafe { ECAM_VA + (phys_off & 0x1FFFFF) }
}

pub fn ecam_read8(bus: u8, dev: u8, func: u8, offset: u16) -> u8 {
    let aligned = offset & 0xFFFC;
    let shift = ((offset & 3) * 8) as u32;
    let val = ecam_read32(bus, dev, func, aligned);
    ((val >> shift) & 0xFF) as u8
}

pub fn ecam_read16(bus: u8, dev: u8, func: u8, offset: u16) -> u16 {
    let aligned = offset & 0xFFFE;
    if aligned & 2 == 0 {
        ecam_read32_aligned(bus, dev, func, aligned) as u16
    } else {
        let lo = ecam_read8(bus, dev, func, aligned) as u16;
        let hi = ecam_read8(bus, dev, func, aligned + 1) as u16;
        lo | (hi << 8)
    }
}

fn ecam_read32_aligned(bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
    let addr = ecam_addr(bus, dev, func, offset & 0xFFFC);
    unsafe { u32::from_le(ptr::read_volatile(addr as *const u32)) }
}

pub fn ecam_read32(bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
    let addr = ecam_addr(bus, dev, func, offset & 0xFFFC);
    unsafe { u32::from_le(ptr::read_volatile(addr as *const u32)) }
}

pub fn ecam_write32(bus: u8, dev: u8, func: u8, offset: u16, val: u32) {
    let addr = ecam_addr(bus, dev, func, offset & 0xFFFC);
    unsafe { ptr::write_volatile(addr as *mut u32, val.to_le()) }
}

#[derive(Debug, Clone, Copy)]
pub struct PciBar {
    pub addr: u64,
    pub size: u64,
    pub is_io: bool,
    pub is_64bit: bool,
}

#[derive(Debug)]
pub struct PciDeviceInfo {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub bars: [PciBar; 6],
    pub num_bars: u8,
}

pub fn probe_device(bus: u8, dev: u8, func: u8) -> Option<PciDeviceInfo> {
    let vendor = ecam_read16(bus, dev, func, PCI_VENDOR_ID);
    if vendor == 0xFFFF {
        return None;
    }

    let device_id = ecam_read16(bus, dev, func, PCI_DEVICE_ID);
    let header_type = ecam_read8(bus, dev, func, PCI_HEADER_TYPE);

    let mut info = PciDeviceInfo {
        bus, dev, func,
        vendor_id: vendor,
        device_id,
        bars: [
            PciBar { addr: 0, size: 0, is_io: false, is_64bit: false },
            PciBar { addr: 0, size: 0, is_io: false, is_64bit: false },
            PciBar { addr: 0, size: 0, is_io: false, is_64bit: false },
            PciBar { addr: 0, size: 0, is_io: false, is_64bit: false },
            PciBar { addr: 0, size: 0, is_io: false, is_64bit: false },
            PciBar { addr: 0, size: 0, is_io: false, is_64bit: false },
        ],
        num_bars: 0,
    };

    let mut bar_offset = PCI_BAR0;
    let mut bar_idx = 0u8;
    while bar_offset <= PCI_BAR5 && bar_idx < 6 {
        let bar_val = ecam_read32(bus, dev, func, bar_offset);

        ecam_write32(bus, dev, func, bar_offset, 0xFFFF_FFFF);
        let size_raw = ecam_read32(bus, dev, func, bar_offset);
        ecam_write32(bus, dev, func, bar_offset, bar_val);

        if size_raw == 0 || size_raw == 0xFFFF_FFFF {
            bar_offset += 4;
            bar_idx += 1;
            continue;
        }

        if bar_val & PCI_BAR_IO == 1 {
            let addr = (bar_val & !0x3) as u64;
            let size = (!(size_raw & !0x3)).wrapping_add(1) as u64;
            info.bars[bar_idx as usize] = PciBar { addr, size, is_io: true, is_64bit: false };
            info.num_bars = bar_idx + 1;
            bar_offset += 4;
        } else {
            let bar_type = bar_val & 0x6;
            if bar_type == PCI_BAR_MEM64 {
                let bar_val_hi = ecam_read32(bus, dev, func, bar_offset + 4);
                let addr = ((bar_val & !0xF) as u64) | ((bar_val_hi as u64) << 32);

                ecam_write32(bus, dev, func, bar_offset + 4, 0xFFFF_FFFF);
                let size_hi = ecam_read32(bus, dev, func, bar_offset + 4);
                ecam_write32(bus, dev, func, bar_offset + 4, bar_val_hi);

                let size = (!(size_raw & !0xF)).wrapping_add(1) as u64
                    | ((!size_hi).wrapping_add(1) as u64) << 32;

                info.bars[bar_idx as usize] = PciBar { addr, size, is_io: false, is_64bit: true };
                info.num_bars = bar_idx + 1;
                bar_offset += 8;
            } else {
                let addr = (bar_val & !0xF) as u64;
                let size = (!(size_raw & !0xF)).wrapping_add(1) as u64;
                info.bars[bar_idx as usize] = PciBar { addr, size, is_io: false, is_64bit: false };
                info.num_bars = bar_idx + 1;
                bar_offset += 4;
            }
        }
        bar_idx += 1;
    }

    if header_type & 0x80 != 0 && func == 0 {
        for f in 1..8u8 {
            if probe_device(bus, dev, f).is_some() {
                uart_puts("[PCI] sub-function ");
                uart_put_hex(f as u64);
                uart_puts("\n");
            }
        }
    }

    Some(info)
}

pub fn enumerate_bus(bus: u8) -> Vec<PciDeviceInfo> {
    let mut devices = Vec::new();
    for dev in 0..32u8 {
        if let Some(info) = probe_device(bus, dev, 0) {
            devices.push(info);
        }
    }
    devices
}

pub fn print_device(info: &PciDeviceInfo) {
    uart_puts("[PCI] ");
    uart_put_hex(info.bus as u64);
    uart_puts(":");
    uart_put_hex(info.dev as u64);
    uart_puts(".");
    uart_put_hex(info.func as u64);
    uart_puts(" vendor=0x");
    uart_put_hex(info.vendor_id as u64);
    uart_puts(" device=0x");
    uart_put_hex(info.device_id as u64);
    uart_puts(" bars=");
    uart_put_hex(info.num_bars as u64);
    for i in 0..info.num_bars as usize {
        if info.bars[i].size > 0 {
            uart_puts(" [");
            uart_put_hex(i as u64);
            uart_puts("]=");
            uart_put_hex(info.bars[i].addr);
        }
    }
    uart_puts("\n");
}
