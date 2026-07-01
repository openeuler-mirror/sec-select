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

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::rsi;
use crate::mm::paging;

const PAGE_SIZE: usize = 0x1000;

pub struct SharedMemory {
    buf: Vec<u8>,
    vaddr: usize,
    pages: usize,
    is_shared: bool,
}

impl SharedMemory {
    pub fn new(pages: usize) -> Option<Self> {
        if pages == 0 {
            return None;
        }
        let size = pages.checked_mul(PAGE_SIZE)?;
        let buf = Vec::from_iter(core::iter::repeat(0u8).take(size));
        let vaddr = buf.as_ptr() as usize;
        Some(Self {
            buf,
            vaddr,
            pages,
            is_shared: false,
        })
    }

    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    pub fn copy_to_private_shadow(&mut self) -> Option<&[u8]> {
        Some(&self.buf)
    }

    pub fn mark_shared(&mut self) -> bool {
        if self.is_shared {
            return true;
        }

        if rsi::is_realm_world() {
            let start = (self.vaddr as u64) & !(PAGE_SIZE as u64 - 1);
            let end = start + (self.pages as u64) * (PAGE_SIZE as u64);

            if rsi::mark_shared(start, end).is_err() {
                return false;
            }

            for i in 0..self.pages {
                let v = start + (i as u64) * (PAGE_SIZE as u64);
                let p = v;
                paging::map_shared_page(v, p);
            }
        }

        self.is_shared = true;
        true
    }

    pub fn mark_private(&mut self) -> bool {
        if !self.is_shared {
            return true;
        }

        if rsi::is_realm_world() {
            let start = (self.vaddr as u64) & !(PAGE_SIZE as u64 - 1);
            let end = start + (self.pages as u64) * (PAGE_SIZE as u64);

            if rsi::accept_memory(start, end).is_err() {
                return false;
            }

            for i in 0..self.pages {
                let v = start + (i as u64) * (PAGE_SIZE as u64);
                let p = v;
                paging::map_protected_page(v, p);
            }
        }

        self.is_shared = false;
        true
    }
}

pub unsafe fn alloc_shared_pages(num: usize) -> Option<usize> {
    let size = PAGE_SIZE.checked_mul(num)?;
    let buf = Vec::from_iter(core::iter::repeat(0u8).take(size)).into_boxed_slice();
    let ptr = Box::into_raw(buf) as *mut u8;
    let addr = ptr as usize;

    if rsi::is_realm_world() {
        let start = (addr as u64) & !(PAGE_SIZE as u64 - 1);
        let end = start + (num as u64) * (PAGE_SIZE as u64);
        if rsi::mark_shared(start, end).is_err() {
            return None;
        }
    }

    Some(addr)
}

pub unsafe fn alloc_shared_page() -> Option<usize> {
    alloc_shared_pages(1)
}

pub unsafe fn free_shared_pages(addr: usize, num: usize) {
    if rsi::is_realm_world() {
        let start = (addr as u64) & !(PAGE_SIZE as u64 - 1);
        let end = start + (num as u64) * (PAGE_SIZE as u64);
        let _ = rsi::accept_memory(start, end);
    }

    let size = PAGE_SIZE.checked_mul(num).expect("Invalid page num");
    let ptr = addr as *mut u8;
    let _ = Box::from_raw(core::slice::from_raw_parts_mut(ptr, size));
}

pub unsafe fn free_shared_page(addr: usize) {
    free_shared_pages(addr, 1)
}
