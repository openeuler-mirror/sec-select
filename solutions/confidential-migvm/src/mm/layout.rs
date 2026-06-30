// Copyright (c) 2022-2025 Intel Corporation
// SPDX-License-Identifier: BSD-2-Clause-Patent

#[derive(Debug, Clone, Copy)]
pub struct RuntimeLayout {
    pub heap_size: usize,
    pub stack_size: usize,
    pub page_table_size: usize,
    pub shared_memory_size: usize,
}
