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

use crate::arch::apic::InterruptStack;
use lazy_static::lazy_static;
use spin::Mutex;

lazy_static! {
    static ref CALLBACKS: Mutex<[Option<fn(&mut InterruptStack)>; 256]> =
        Mutex::new([None; 256]);
}

#[derive(Copy, Clone)]
pub struct InterruptCallback(fn(&mut InterruptStack));

impl InterruptCallback {
    pub fn new(cb: fn(&mut InterruptStack)) -> Self {
        Self(cb)
    }

    pub fn call(&self, stack: &mut InterruptStack) {
        (self.0)(stack)
    }
}

pub fn register_interrupt_callback(
    vector: usize,
    cb: InterruptCallback,
) -> Result<(), ()> {
    if vector >= 256 {
        return Err(());
    }
    CALLBACKS.lock()[vector] = Some(cb.0);
    Ok(())
}

pub fn dispatch_interrupt(vector: usize, stack: &mut InterruptStack) {
    if vector < 256 {
        if let Some(cb) = CALLBACKS.lock()[vector] {
            cb(stack);
        }
    }
}
