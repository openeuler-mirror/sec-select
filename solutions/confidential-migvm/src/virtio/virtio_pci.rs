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
use crate::pci::{ecam_read8, ecam_read16, ecam_read32, ecam_write32, PciDeviceInfo};
use core::ptr;

const PCI_BAR0: u16 = 0x10;
const PCI_COMMAND: u16 = 0x04;
const PCI_CMD_MEM: u32 = 0x02;
const PCI_CMD_BUSMASTER: u32 = 0x04;

const VIRTIO_STATUS_RESET: u32 = 0;
const VIRTIO_STATUS_ACKNOWLEDGE: u32 = 1;
const VIRTIO_STATUS_DRIVER: u32 = 2;
const VIRTIO_STATUS_FEATURES_OK: u32 = 8;
const VIRTIO_STATUS_DRIVER_OK: u32 = 4;

#[derive(Clone, Copy)]
pub struct VirtioPciTransport {
    base: u64,
    notify_base: u64,
    notify_mult: u32,
    device_base: u64,
    isr_base: u64,
    pub device_id: u32,
}

impl VirtioPciTransport {
    pub fn init(info: &PciDeviceInfo, mmio_pa: u64, bar_va: u64) -> Option<Self> {
        let bus = info.bus;
        let dev = info.dev;
        let func = info.func;

        ecam_write32(bus, dev, func, PCI_COMMAND, PCI_CMD_MEM | PCI_CMD_BUSMASTER);

        let mut common_off: u32 = 0;
        let mut notify_off_cap: u32 = 0;
        let mut notify_mult: u32 = 0;
        let mut device_off: u32 = 0;
        let mut isr_off: u32 = 0;
        let mut bar_idx: u8 = 0;

        let mut cap_next = ecam_read8(bus, dev, func, 0x34);
        let mut visited = [0u8; 16];
        let mut vc = 0usize;

        while cap_next > 0 && cap_next < 0xFC && vc < 16 {
            if visited[..vc].contains(&cap_next) { break; }
            visited[vc] = cap_next;
            vc += 1;

            let cap_id = ecam_read8(bus, dev, func, cap_next as u16);
            let next = ecam_read8(bus, dev, func, cap_next as u16 + 1);

            if cap_id == 0x09 {
                let cfg_type = ecam_read8(bus, dev, func, cap_next as u16 + 3);
                let cb_bar = ecam_read8(bus, dev, func, cap_next as u16 + 4);
                let offset = ecam_read32(bus, dev, func, (cap_next as u16 + 8) & 0xFFFC);
                let length = ecam_read32(bus, dev, func, (cap_next as u16 + 12) & 0xFFFC);

                uart_puts("[VirtIO PCI] cap: type=");
                uart_put_hex(cfg_type as u64);
                uart_puts(" bar=");
                uart_put_hex(cb_bar as u64);
                uart_puts(" off=0x");
                uart_put_hex(offset as u64);
                uart_puts(" len=0x");
                uart_put_hex(length as u64);
                uart_puts("\n");

                match cfg_type {
                    1 => { common_off = offset; bar_idx = cb_bar; }
                    2 => {
                        notify_off_cap = offset;
                        notify_mult = ecam_read32(bus, dev, func, (cap_next as u16 + 16) & 0xFFFC);
                    }
                    3 => isr_off = offset,
                    4 => device_off = offset,
                    _ => {}
                }
            }
            cap_next = next;
        }

        let mut bar_pa = if (bar_idx as usize) < info.bars.len() {
            info.bars[bar_idx as usize].addr
        } else {
            0
        };

        uart_puts("[VirtIO PCI] bar_idx=");
        uart_put_hex(bar_idx as u64);
        uart_puts(" bar_orig=0x");
        uart_put_hex(bar_pa);
        uart_puts(" bar_size=0x");
        if (bar_idx as usize) < info.bars.len() {
            uart_put_hex(info.bars[bar_idx as usize].size);
        } else {
            uart_puts("0");
        }
        uart_puts(" is_64bit=");
        if (bar_idx as usize) < info.bars.len() && info.bars[bar_idx as usize].is_64bit {
            uart_puts("yes");
        } else {
            uart_puts("no");
        }
        uart_puts(" mmio_pa=0x");
        uart_put_hex(mmio_pa);
        uart_puts("\n");

        if bar_pa == 0 {
            bar_pa = mmio_pa;

            let bar_offset = PCI_BAR0 + (bar_idx as u16) * 4;
            ecam_write32(bus, dev, func, bar_offset, (bar_pa & 0xFFFF_FFFF) as u32);
            if (bar_idx as usize) < info.bars.len() && info.bars[bar_idx as usize].is_64bit {
                ecam_write32(bus, dev, func, bar_offset + 4, (bar_pa >> 32) as u32);
                uart_puts("[VirtIO PCI] Programmed 64-bit BAR");
                uart_put_hex(bar_idx as u64);
                uart_puts(" = 0x");
                uart_put_hex(bar_pa);
                uart_puts("\n");
            } else {
                uart_puts("[VirtIO PCI] Programmed 32-bit BAR");
                uart_put_hex(bar_idx as u64);
                uart_puts(" = 0x");
                uart_put_hex(bar_pa);
                uart_puts("\n");
            }

            let verify = ecam_read32(bus, dev, func, bar_offset);
            uart_puts("[VirtIO PCI] BAR readback: 0x");
            uart_put_hex(verify as u64);
            uart_puts("\n");
        }

        ecam_write32(bus, dev, func, PCI_COMMAND, PCI_CMD_MEM | PCI_CMD_BUSMASTER);

        crate::mm::paging::map_bar_space(bar_va, bar_pa);

        let probe = unsafe { core::ptr::read_volatile(bar_va as *const u32) };
        uart_puts("[VirtIO PCI] probe va=0x");
        uart_put_hex(bar_va);
        uart_puts(" → 0x");
        uart_put_hex(probe as u64);
        if probe == 0xFFFFFFFF {
            uart_puts(" BAD (all-1s)\n");
            return None;
        }
        uart_puts(" (device_feature_select=");
        uart_put_hex(probe as u64);
        uart_puts(")\n");

        let device_id_reg = ecam_read16(bus, dev, func, 0x02) as u32;

        let transport = VirtioPciTransport {
            base: bar_va + common_off as u64,
            notify_base: bar_va + notify_off_cap as u64,
            notify_mult,
            device_base: bar_va + device_off as u64,
            isr_base: bar_va + isr_off as u64,
            device_id: device_id_reg,
        };

        transport.write_status(VIRTIO_STATUS_RESET);
        while transport.read_status() != VIRTIO_STATUS_RESET as u8 {}

        transport.write_status(VIRTIO_STATUS_ACKNOWLEDGE);
        transport.write_status(VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

        Some(transport)
    }

    #[allow(dead_code)]
    fn read_reg0(&self) -> u32 {
        unsafe { u32::from_le(ptr::read_volatile(self.base as *const u32)) }
    }
    fn write_reg0(&self, v: u32) {
        unsafe { ptr::write_volatile(self.base as *mut u32, v.to_le()) }
    }
    fn read_reg1(&self) -> u32 {
        unsafe { u32::from_le(ptr::read_volatile((self.base + 4) as *const u32)) }
    }
    fn write_reg1(&self, v: u32) {
        unsafe { ptr::write_volatile((self.base + 4) as *mut u32, v.to_le()) }
    }
    #[allow(dead_code)]
    fn read_u32(&self, off: u64) -> u32 {
        unsafe { u32::from_le(ptr::read_volatile((self.base + off) as *const u32)) }
    }
    fn write_u32(&self, off: u64, v: u32) {
        unsafe { ptr::write_volatile((self.base + off) as *mut u32, v.to_le()) }
    }

    fn read_status(&self) -> u8 {
        unsafe { ptr::read_volatile((self.base + 0x14) as *const u8) }
    }
    fn write_status(&self, val: u32) {
        unsafe {
            ptr::write_volatile((self.base + 0x14) as *mut u8, val as u8);
            core::arch::asm!("dmb sy", options(nomem, nostack));
        }
    }
    fn write_queue_select(&self, idx: u16) {
        unsafe {
            ptr::write_volatile((self.base + 0x16) as *mut u16, idx.to_le());
            core::arch::asm!("dmb sy", options(nomem, nostack));
        }
    }
    fn read_queue_size(&self) -> u16 {
        unsafe { u16::from_le(ptr::read_volatile((self.base + 0x18) as *const u16)) }
    }
    fn write_queue_size(&self, size: u16) {
        unsafe {
            ptr::write_volatile((self.base + 0x18) as *mut u16, size.to_le());
            core::arch::asm!("dmb sy", options(nomem, nostack));
        }
    }
    fn write_queue_enable(&self) {
        unsafe {
            ptr::write_volatile((self.base + 0x1c) as *mut u16, 1u16.to_le());
            core::arch::asm!("dmb sy", options(nomem, nostack));
        }
    }
    fn read_notify_off(&self) -> u16 {
        unsafe { u16::from_le(ptr::read_volatile((self.base + 0x1e) as *const u16)) }
    }

    pub fn device_id(&self) -> u32 { self.device_id }

    pub fn get_features(&self) -> u64 {
        self.write_reg0(0);
        let lo = self.read_reg1() as u64;
        self.write_reg0(1);
        let hi = self.read_reg1() as u64;
        lo | (hi << 32)
    }

    pub fn negotiate_features(&self, driver_features: u64) -> Option<u64> {
        let device_features = self.get_features();
        let negotiated = device_features & driver_features;

        self.write_reg0(0);
        self.write_reg1(negotiated as u32);
        self.write_reg0(1);
        self.write_reg1((negotiated >> 32) as u32);

        let st = self.read_status() as u32;
        self.write_status(st | VIRTIO_STATUS_FEATURES_OK);

        let new_st = self.read_status();
        uart_puts("[VirtIO PCI] status after FEATURES_OK: 0x");
        uart_put_hex(new_st as u64);
        uart_puts("\n");

        if new_st & VIRTIO_STATUS_FEATURES_OK as u8 == 0 {
            return None;
        }
        Some(negotiated)
    }

    pub fn get_queue_max_size(&self, idx: u16) -> u16 {
        self.write_queue_select(idx);
        self.read_queue_size()
    }

    pub fn setup_queue(&self, idx: u16, desc: u64, avail: u64, used: u64, size: u16) -> bool {
        self.write_queue_select(idx);
        let ms = self.read_queue_size();
        if ms == 0 || size > ms { return false; }
        self.write_queue_size(size);
        self.write_u32(0x20, desc as u32);
        self.write_u32(0x24, (desc >> 32) as u32);
        self.write_u32(0x28, avail as u32);
        self.write_u32(0x2c, (avail >> 32) as u32);
        self.write_u32(0x30, used as u32);
        self.write_u32(0x34, (used >> 32) as u32);
        self.write_queue_enable();
        true
    }

    pub fn driver_ok(&self) {
        self.write_status(self.read_status() as u32 | VIRTIO_STATUS_DRIVER_OK);
    }

    pub fn notify_queue(&self, idx: u16) {
        self.write_queue_select(idx);
        unsafe { core::arch::asm!("dmb sy", options(nomem, nostack)); }
        let off = self.read_notify_off();
        let addr = self.notify_base + (off as u64) * (self.notify_mult as u64);

        uart_puts("[VirtIO PCI] notify: base=0x");
        uart_put_hex(self.notify_base);
        uart_puts(" off=0x");
        uart_put_hex(off as u64);
        uart_puts(" mult=0x");
        uart_put_hex(self.notify_mult as u64);
        uart_puts(" addr=0x");
        uart_put_hex(addr);
        uart_puts("\n");

        unsafe {
            ptr::write_volatile(addr as *mut u8, idx as u8);
            core::arch::asm!("dmb sy", options(nomem, nostack));
        }
    }

    pub fn read_config8(&self, offset: u64) -> u8 {
        let aligned = offset & !3u64;
        let shift = ((offset & 3) * 8) as u32;
        let val = unsafe { u32::from_le(ptr::read_volatile((self.device_base + aligned) as *const u32)) };
        ((val >> shift) & 0xFF) as u8
    }

    pub fn read_config64(&self, offset: u64) -> u64 {
        let lo = unsafe { u32::from_le(ptr::read_volatile((self.device_base + offset) as *const u32)) };
        let hi = unsafe { u32::from_le(ptr::read_volatile((self.device_base + offset + 4) as *const u32)) };
        lo as u64 | ((hi as u64) << 32)
    }

    pub fn read_interrupt_status(&self) -> u32 {
        if self.isr_base != 0 {
            unsafe { ptr::read_volatile(self.isr_base as *const u8) as u32 }
        } else {
            0
        }
    }
}
