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

use crate::fdt::VirtioMmioDevice;
use crate::uart_puts;
use crate::uart_put_hex;
use crate::virtio::mmio_regs::*;
use crate::virtio::mmio_transport::VirtioMmioTransport;

pub struct VirtioDeviceInfo {
    pub device_type: u32,
    pub transport: VirtioMmioTransport,
}

pub fn probe_devices(devices: &[VirtioMmioDevice]) -> Vec<VirtioDeviceInfo> {
    let mut found = Vec::new();

    uart_puts("[VirtIO] Probing ");
    uart_put_hex(devices.len() as u64);
    uart_puts(" MMIO devices...\n");

    for dev in devices {
        let mut transport = VirtioMmioTransport::new(dev);

        match transport.probe() {
            Ok(()) => {
                let device_type = transport.device_id();
                if device_type == 0 {
                    continue;
                }

                let type_name = match device_type {
                    VIRTIO_SUBSYSTEM_NET => "net",
                    VIRTIO_SUBSYSTEM_BLOCK => "block",
                    VIRTIO_SUBSYSTEM_CONSOLE => "console",
                    VIRTIO_SUBSYSTEM_VSOCK => "vsock",
                    _ => "unknown",
                };

                uart_puts("[VirtIO] Found ");
                uart_puts(type_name);
                uart_puts(" device at 0x");
                uart_put_hex(dev.base);
                uart_puts("\n");

                found.push(VirtioDeviceInfo {
                    device_type,
                    transport,
                });
            }
            Err(e) => {
                uart_puts("[VirtIO] Probe failed at 0x");
                uart_put_hex(dev.base);
                uart_puts(": ");
                match e {
                    crate::virtio::MmioError::BadMagic => uart_puts("bad magic"),
                    crate::virtio::MmioError::BadVersion => uart_puts("bad version"),
                    _ => uart_puts("unknown error"),
                }
                uart_puts("\n");
            }
        }
    }

    uart_puts("[VirtIO] Found ");
    uart_put_hex(found.len() as u64);
    uart_puts(" valid devices\n");

    found
}

pub fn test_init_devices(devices: &mut [VirtioDeviceInfo]) {
    for info in devices.iter_mut() {
        let type_name = match info.device_type {
            VIRTIO_SUBSYSTEM_NET => "net",
            VIRTIO_SUBSYSTEM_BLOCK => "block",
            VIRTIO_SUBSYSTEM_CONSOLE => "console",
            VIRTIO_SUBSYSTEM_VSOCK => "vsock",
            _ => "unknown",
        };

        uart_puts("\n[VirtIO] === Testing ");
        uart_puts(type_name);
        uart_puts(" init ===\n");

        match info.transport.init_device(info.device_type) {
            Ok(()) => {
                uart_puts("[VirtIO] init_device OK\n");

                let features = info.transport.get_features();
                uart_puts("[VirtIO] Device features: 0x");
                uart_put_hex(features);
                uart_puts("\n");

                for q in 0..4u16 {
                    let max = info.transport.get_queue_max_size(q);
                    if max > 0 {
                        uart_puts("[VirtIO] Queue[");
                        uart_put_hex(q as u64);
                        uart_puts("] max_size=");
                        uart_put_hex(max as u64);
                        uart_puts("\n");
                    }
                }
            }
            Err(e) => {
                uart_puts("[VirtIO] init_device FAILED: ");
                match e {
                    crate::virtio::MmioError::DeviceIdMismatch => uart_puts("DeviceIdMismatch"),
                    _ => uart_puts("io error"),
                }
                uart_puts("\n");
            }
        }
    }

    uart_puts("[VirtIO] Device init test complete\n");
}

use alloc::vec::Vec;
