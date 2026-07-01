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
use crate::arch::apic;
use crate::arch::idt;
use crate::fdt::VirtioMmioDevice;
use crate::virtio::mmio_regs::*;
use crate::virtio::MmioError;
use crate::virtio::MmioResult;
use core::ptr;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MmioVersion {
    Legacy = 1,
    Modern = 2,
}

#[derive(Clone, Copy)]
pub struct VirtioMmioTransport {
    base: u64,
    irq: u32,
    device_id: u32,
    version: MmioVersion,
}

impl VirtioMmioTransport {
    pub fn new(dev: &VirtioMmioDevice) -> Self {
        VirtioMmioTransport {
            base: dev.base,
            irq: dev.irq,
            device_id: 0,
            version: MmioVersion::Modern,
        }
    }

    pub fn device_id(&self) -> u32 {
        self.device_id
    }

    pub fn irq(&self) -> u32 {
        self.irq
    }

    pub fn version(&self) -> MmioVersion {
        self.version
    }

    pub fn is_legacy(&self) -> bool {
        self.version == MmioVersion::Legacy
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn read_config8(&self, offset: u64) -> u8 {
        let aligned = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        let val = self.read32(VIRTIO_MMIO_CONFIG + aligned);
        ((val >> shift) & 0xFF) as u8
    }

    fn read32(&self, offset: u64) -> u32 {
        unsafe { ptr::read_volatile((self.base + offset) as *const u32) }
    }

    fn write32(&self, offset: u64, val: u32) {
        unsafe { ptr::write_volatile((self.base + offset) as *mut u32, val) }
    }

    #[allow(dead_code)]
    fn read8(&self, offset: u64) -> u8 {
        unsafe { ptr::read_volatile((self.base + offset) as *const u8) }
    }

    #[allow(dead_code)]
    fn write8(&self, offset: u64, val: u8) {
        unsafe { ptr::write_volatile((self.base + offset) as *mut u8, val) }
    }

    pub fn probe(&mut self) -> MmioResult<()> {
        let magic = self.read32(VIRTIO_MMIO_MAGIC_VALUE);
        if magic != VIRT_MAGIC {
            return Err(MmioError::BadMagic);
        }

        let version = self.read32(VIRTIO_MMIO_VERSION);
        match version {
            VIRT_VERSION_LEGACY => {
                self.version = MmioVersion::Legacy;
            }
            VIRT_VERSION_MODERN => {
                self.version = MmioVersion::Modern;
            }
            _ => {
                uart_puts("[VirtIO] Unsupported version: ");
                uart_put_hex(version as u64);
                uart_puts("\n");
                return Err(MmioError::BadVersion);
            }
        }

        self.device_id = self.read32(VIRTIO_MMIO_DEVICE_ID);
        let vendor_id = self.read32(VIRTIO_MMIO_VENDOR_ID);

        let ver_str = if self.is_legacy() { "legacy" } else { "modern" };
        uart_puts("[VirtIO] MMIO ");
        uart_puts(ver_str);
        uart_puts(" device at 0x");
        uart_put_hex(self.base);
        uart_puts(" irq=");
        uart_put_hex(self.irq as u64);
        uart_puts(" device_id=");
        uart_put_hex(self.device_id as u64);
        uart_puts(" vendor_id=");
        uart_put_hex(vendor_id as u64);
        uart_puts("\n");

        Ok(())
    }

    pub fn init_device(&mut self, device_type: u32) -> MmioResult<()> {
        if self.device_id != device_type {
            uart_puts("[VirtIO] Device ID mismatch: expected ");
            uart_put_hex(device_type as u64);
            uart_puts(" got ");
            uart_put_hex(self.device_id as u64);
            uart_puts("\n");
            return Err(MmioError::DeviceIdMismatch);
        }

        self.write32(VIRTIO_MMIO_STATUS, VIRTIO_STATUS_RESET as u32);
        while self.read32(VIRTIO_MMIO_STATUS) != VIRTIO_STATUS_RESET as u32 {}

        self.write32(VIRTIO_MMIO_STATUS, VIRTIO_STATUS_ACKNOWLEDGE as u32);
        self.write32(VIRTIO_MMIO_STATUS, (self.read32(VIRTIO_MMIO_STATUS) as u8 | VIRTIO_STATUS_DRIVER) as u32);

        Ok(())
    }

    pub fn negotiate_features(&self, driver_features: u64) -> MmioResult<u64> {
        let device_features = self.get_features();

        let negotiated = device_features & driver_features;

        if self.is_legacy() {
            self.write32(VIRTIO_MMIO_DRIVER_FEATURES_SEL, 0);
            self.write32(VIRTIO_MMIO_DRIVER_FEATURES, negotiated as u32);
        } else {
            self.write32(VIRTIO_MMIO_DRIVER_FEATURES_SEL, 0);
            self.write32(VIRTIO_MMIO_DRIVER_FEATURES, negotiated as u32);
            self.write32(VIRTIO_MMIO_DRIVER_FEATURES_SEL, 1);
            self.write32(VIRTIO_MMIO_DRIVER_FEATURES, (negotiated >> 32) as u32);
        }

        if !self.is_legacy() {
            let old_status = self.read32(VIRTIO_MMIO_STATUS) as u8;
            let new_status = old_status | VIRTIO_STATUS_FEATURES_OK;
            self.write32(VIRTIO_MMIO_STATUS, new_status as u32);

            let status = self.read32(VIRTIO_MMIO_STATUS) as u8;
            uart_puts("[VirtIO] status after FEATURES_OK: 0x");
            uart_put_hex(status as u64);
            uart_puts("\n");
            if status & VIRTIO_STATUS_FEATURES_OK == 0 {
                uart_puts("[VirtIO] Feature negotiation failed\n");
                self.write32(VIRTIO_MMIO_STATUS, VIRTIO_STATUS_FAILED as u32);
                return Err(MmioError::FeatureNegotiationFailed);
            }
        }

        Ok(negotiated)
    }

    pub fn get_features(&self) -> u64 {
        if self.is_legacy() {
            self.write32(VIRTIO_MMIO_DEVICE_FEATURES_SEL, 0);
            self.read32(VIRTIO_MMIO_DEVICE_FEATURES) as u64
        } else {
            self.write32(VIRTIO_MMIO_DEVICE_FEATURES_SEL, 0);
            let lo = self.read32(VIRTIO_MMIO_DEVICE_FEATURES) as u64;
            self.write32(VIRTIO_MMIO_DEVICE_FEATURES_SEL, 1);
            let hi = self.read32(VIRTIO_MMIO_DEVICE_FEATURES) as u64;
            lo | (hi << 32)
        }
    }

    pub fn setup_queue(&self, idx: u16, desc_addr: u64, avail_addr: u64, used_addr: u64, size: u16) -> MmioResult<()> {
        self.write32(VIRTIO_MMIO_QUEUE_SEL, idx as u32);

        let max_size = self.read32(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16;
        if max_size == 0 {
            return Err(MmioError::QueueNotAvailable);
        }
        if size > max_size {
            return Err(MmioError::QueueTooSmall);
        }

        self.write32(VIRTIO_MMIO_QUEUE_NUM, size as u32);

        if self.is_legacy() {
            self.write32(VIRTIO_MMIO_GUEST_PAGE_SIZE, 0x1000);
            self.write32(VIRTIO_MMIO_QUEUE_ALIGN, 0x1000);
            let pfn = (desc_addr >> 12) as u32;
            self.write32(VIRTIO_MMIO_QUEUE_PFN, pfn);
        } else {
            self.write32(VIRTIO_MMIO_QUEUE_DESC_LOW, desc_addr as u32);
            self.write32(VIRTIO_MMIO_QUEUE_DESC_HIGH, (desc_addr >> 32) as u32);

            self.write32(VIRTIO_MMIO_QUEUE_AVAIL_LOW, avail_addr as u32);
            self.write32(VIRTIO_MMIO_QUEUE_AVAIL_HIGH, (avail_addr >> 32) as u32);

            self.write32(VIRTIO_MMIO_QUEUE_USED_LOW, used_addr as u32);
            self.write32(VIRTIO_MMIO_QUEUE_USED_HIGH, (used_addr >> 32) as u32);

            self.write32(VIRTIO_MMIO_QUEUE_READY, 1);
        }

        Ok(())
    }

    pub fn get_queue_max_size(&self, idx: u16) -> u16 {
        self.write32(VIRTIO_MMIO_QUEUE_SEL, idx as u32);
        self.read32(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16
    }

    pub fn notify_queue(&self, queue: u16) {
        self.write32(VIRTIO_MMIO_QUEUE_NOTIFY, queue as u32);
    }

    pub fn driver_ok(&self) {
        self.write32(VIRTIO_MMIO_STATUS, (self.read32(VIRTIO_MMIO_STATUS) as u8 | VIRTIO_STATUS_DRIVER_OK) as u32);
    }

    pub fn read_interrupt_status(&self) -> u32 {
        self.read32(VIRTIO_MMIO_INTERRUPT_STATUS)
    }

    pub fn ack_interrupt(&self, status: u32) {
        self.write32(VIRTIO_MMIO_INTERRUPT_ACK, status);
    }

    pub fn read_device_config32(&self, offset: u64) -> u32 {
        self.read32(VIRTIO_MMIO_CONFIG + offset)
    }

    pub fn enable_interrupt(&self) {
        apic::enable_irq(self.irq);
    }

    pub fn register_irq_handler(&self, callback: fn(&mut apic::InterruptStack)) {
        apic::enable_irq(self.irq);
        idt::register_interrupt_callback(self.irq as usize, idt::InterruptCallback::new(callback))
            .ok();
    }
}
