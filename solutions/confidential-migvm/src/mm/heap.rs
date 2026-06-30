// Copyright (c) 2022-2025 Intel Corporation
// SPDX-License-Identifier: BSD-2-Clause-Patent

extern "C" {
    static __heap_start: u8;
    static __heap_end: u8;
}

#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

pub fn init_heap(heap_size: usize) {
    unsafe {
        let heap_start = core::ptr::addr_of!(__heap_start) as *const u8 as usize;
        let heap_end = core::ptr::addr_of!(__heap_end) as *const u8 as usize;
        let actual_size = heap_end - heap_start;

        let size = if actual_size >= heap_size {
            heap_size
        } else {
            actual_size
        };

        if size == 0 {
            return;
        }

        ALLOCATOR.lock().init(heap_start as *mut u8, size);
    }
}
