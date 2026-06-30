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

pub mod mmio_regs;
pub mod mmio_transport;
pub mod device;
pub mod net;
pub mod virtio_pci;
pub mod vsock;

use core::fmt;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const DMA_MODE_HEAP: usize = 0;
const DMA_MODE_SHARED_POOL: usize = 1;

static DMA_MODE: AtomicUsize = AtomicUsize::new(DMA_MODE_HEAP);
static DMA_POOL_BASE: AtomicU64 = AtomicU64::new(0);
static DMA_POOL_OFFSET: AtomicUsize = AtomicUsize::new(0);
static DMA_POOL_SIZE: AtomicUsize = AtomicUsize::new(0);

pub fn dma_pool_used() -> usize {
    DMA_POOL_OFFSET.load(Ordering::Relaxed)
}

pub fn dma_pool_capacity() -> usize {
    DMA_POOL_SIZE.load(Ordering::Relaxed)
}

pub fn init_dma_pool_normal() {
    DMA_MODE.store(DMA_MODE_HEAP, Ordering::Relaxed);
}

pub fn init_dma_pool_shared(pool_va: u64, pool_size: usize) {
    DMA_POOL_BASE.store(pool_va, Ordering::Relaxed);
    DMA_POOL_SIZE.store(pool_size, Ordering::Relaxed);
    DMA_POOL_OFFSET.store(0, Ordering::Relaxed);
    DMA_MODE.store(DMA_MODE_SHARED_POOL, Ordering::Relaxed);
}

pub fn alloc_dma_pages(num_pages: usize) -> Option<u64> {
    const PAGE_SIZE: usize = 0x1000;
    let size = num_pages * PAGE_SIZE;

    if DMA_MODE.load(Ordering::Relaxed) == DMA_MODE_SHARED_POOL {
        let pool_base = DMA_POOL_BASE.load(Ordering::Relaxed);
        let pool_size = DMA_POOL_SIZE.load(Ordering::Relaxed);
        let offset = DMA_POOL_OFFSET.fetch_add(size, Ordering::Relaxed);
        if offset + size > pool_size {
            return None;
        }
        let addr = pool_base + offset as u64;
        unsafe { core::ptr::write_bytes(addr as *mut u8, 0, size); }
        Some(addr)
    } else {
        let layout = alloc::alloc::Layout::from_size_align(size, PAGE_SIZE).ok()?;
        let ptr = unsafe { alloc::alloc::alloc(layout) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as u64)
        }
    }
}

pub fn free_dma_pages(addr: u64, num_pages: usize) {
    if DMA_MODE.load(Ordering::Relaxed) == DMA_MODE_SHARED_POOL {
        return;
    }
    const PAGE_SIZE: usize = 0x1000;
    let size = num_pages * PAGE_SIZE;
    let layout = alloc::alloc::Layout::from_size_align(size, PAGE_SIZE).unwrap();
    unsafe { alloc::alloc::dealloc(addr as *mut u8, layout) }
}

#[derive(Debug)]
pub enum MmioError {
    BadMagic,
    BadVersion,
    DeviceIdMismatch,
    FeatureNegotiationFailed,
    QueueNotAvailable,
    QueueTooSmall,
    DmaAllocation,
    IoError,
}

impl fmt::Display for MmioError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MmioError::BadMagic => write!(f, "BadMagic"),
            MmioError::BadVersion => write!(f, "BadVersion"),
            MmioError::DeviceIdMismatch => write!(f, "DeviceIdMismatch"),
            MmioError::FeatureNegotiationFailed => write!(f, "FeatureNegotiationFailed"),
            MmioError::QueueNotAvailable => write!(f, "QueueNotAvailable"),
            MmioError::QueueTooSmall => write!(f, "QueueTooSmall"),
            MmioError::DmaAllocation => write!(f, "DmaAllocation"),
            MmioError::IoError => write!(f, "IoError"),
        }
    }
}

pub type MmioResult<T = ()> = core::result::Result<T, MmioError>;
