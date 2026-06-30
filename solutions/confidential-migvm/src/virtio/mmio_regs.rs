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

pub const VIRTIO_MMIO_MAGIC_VALUE: u64 = 0x000;
pub const VIRTIO_MMIO_VERSION: u64 = 0x004;
pub const VIRTIO_MMIO_DEVICE_ID: u64 = 0x008;
pub const VIRTIO_MMIO_VENDOR_ID: u64 = 0x00c;
pub const VIRTIO_MMIO_DEVICE_FEATURES: u64 = 0x010;
pub const VIRTIO_MMIO_DEVICE_FEATURES_SEL: u64 = 0x014;
pub const VIRTIO_MMIO_DRIVER_FEATURES: u64 = 0x020;
pub const VIRTIO_MMIO_DRIVER_FEATURES_SEL: u64 = 0x024;
pub const VIRTIO_MMIO_GUEST_PAGE_SIZE: u64 = 0x028;
pub const VIRTIO_MMIO_QUEUE_SEL: u64 = 0x030;
pub const VIRTIO_MMIO_QUEUE_NUM_MAX: u64 = 0x034;
pub const VIRTIO_MMIO_QUEUE_NUM: u64 = 0x038;
pub const VIRTIO_MMIO_QUEUE_ALIGN: u64 = 0x03c;
pub const VIRTIO_MMIO_QUEUE_PFN: u64 = 0x040;
pub const VIRTIO_MMIO_QUEUE_READY: u64 = 0x044;
pub const VIRTIO_MMIO_QUEUE_NOTIFY: u64 = 0x050;
pub const VIRTIO_MMIO_INTERRUPT_STATUS: u64 = 0x060;
pub const VIRTIO_MMIO_INTERRUPT_ACK: u64 = 0x064;
pub const VIRTIO_MMIO_STATUS: u64 = 0x070;
pub const VIRTIO_MMIO_QUEUE_DESC_LOW: u64 = 0x080;
pub const VIRTIO_MMIO_QUEUE_DESC_HIGH: u64 = 0x084;
pub const VIRTIO_MMIO_QUEUE_AVAIL_LOW: u64 = 0x090;
pub const VIRTIO_MMIO_QUEUE_AVAIL_HIGH: u64 = 0x094;
pub const VIRTIO_MMIO_QUEUE_USED_LOW: u64 = 0x0a0;
pub const VIRTIO_MMIO_QUEUE_USED_HIGH: u64 = 0x0a4;
pub const VIRTIO_MMIO_CONFIG_GENERATION: u64 = 0x0fc;
pub const VIRTIO_MMIO_CONFIG: u64 = 0x100;

pub const VIRT_MAGIC: u32 = 0x7472_6976;
pub const VIRT_VERSION_LEGACY: u32 = 1;
pub const VIRT_VERSION_MODERN: u32 = 2;
pub const VIRT_VENDOR: u32 = 0x554D_4551;

pub const VIRTIO_MMIO_INT_VRING: u32 = 1 << 0;
pub const VIRTIO_MMIO_INT_CONFIG: u32 = 1 << 1;

pub const VIRTIO_STATUS_RESET: u8 = 0;
pub const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
pub const VIRTIO_STATUS_DRIVER: u8 = 2;
pub const VIRTIO_STATUS_FEATURES_OK: u8 = 8;
pub const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
pub const VIRTIO_STATUS_FAILED: u8 = 128;

pub const VIRTIO_SUBSYSTEM_NET: u32 = 1;
pub const VIRTIO_SUBSYSTEM_BLOCK: u32 = 2;
pub const VIRTIO_SUBSYSTEM_CONSOLE: u32 = 3;
pub const VIRTIO_SUBSYSTEM_VSOCK: u32 = 19;

pub const VIRTIO_F_VERSION_1: u64 = 1 << 32;
