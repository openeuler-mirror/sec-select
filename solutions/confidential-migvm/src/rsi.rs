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

use core::arch::asm;

const RSI_ABI_VERSION_MAJOR: u64 = 1;
const RSI_ABI_VERSION_MINOR: u64 = 0;
pub const RSI_ABI_VERSION: u64 = (RSI_ABI_VERSION_MAJOR << 16) | RSI_ABI_VERSION_MINOR;

pub const RSI_SUCCESS: u64 = 0;
pub const RSI_ERROR_INPUT: u64 = 1;
pub const RSI_ERROR_STATE: u64 = 2;
pub const RSI_INCOMPLETE: u64 = 3;
pub const RSI_ERROR_UNKNOWN: u64 = 4;

#[repr(u64)]
#[derive(Clone, Copy)]
pub enum Ripas {
    Empty = 0,
    Ram = 1,
    Destroyed = 2,
    Dev = 3,
}

pub const RSI_NO_CHANGE_DESTROYED: u64 = 0;
pub const RSI_CHANGE_DESTROYED: u64 = 1;

pub const RSI_ACCEPT: u64 = 0;
pub const RSI_REJECT: u64 = 1;

const SMC_RSI_FID_BASE: u64 = 0xC400_0000;

const fn smc_rsi_fid(n: u64) -> u64 {
    SMC_RSI_FID_BASE | n
}

pub const SMC_RSI_ABI_VERSION: u64 = smc_rsi_fid(0x190);
pub const SMC_RSI_FEATURES: u64 = smc_rsi_fid(0x191);
pub const SMC_RSI_MEASUREMENT_READ: u64 = smc_rsi_fid(0x192);
pub const SMC_RSI_MEASUREMENT_EXTEND: u64 = smc_rsi_fid(0x193);
pub const SMC_RSI_ATTESTATION_TOKEN_INIT: u64 = smc_rsi_fid(0x194);
pub const SMC_RSI_ATTESTATION_TOKEN_CONTINUE: u64 = smc_rsi_fid(0x195);
pub const SMC_RSI_REALM_CONFIG: u64 = smc_rsi_fid(0x196);
pub const SMC_RSI_IPA_STATE_SET: u64 = smc_rsi_fid(0x197);
pub const SMC_RSI_IPA_STATE_GET: u64 = smc_rsi_fid(0x198);
pub const SMC_RSI_HOST_CALL: u64 = smc_rsi_fid(0x199);

#[repr(C, align(4096))]
pub struct RealmConfig {
    pub ipa_bits: u64,
    pub hash_algo: u64,
    _pad1: [u8; 0x1F0],
    pub rpv: [u8; 64],
    _pad2: [u8; 0xDB8],
}

impl RealmConfig {
    pub fn new() -> Self {
        Self {
            ipa_bits: 0,
            hash_algo: 0,
            _pad1: [0; 0x1F0],
            rpv: [0; 64],
            _pad2: [0; 0xDB8],
        }
    }
}

#[repr(C)]
pub struct SmcResult {
    pub x0: u64,
    pub x1: u64,
    pub x2: u64,
    pub x3: u64,
}

#[inline(always)]
pub fn smc(fid: u64, x1: u64, x2: u64, x3: u64, x4: u64) -> SmcResult {
    let x0: u64;
    let r1: u64;
    let r2: u64;
    let r3: u64;
    unsafe {
        asm!(
            "smc #0",
            in("x0") fid,
            in("x1") x1,
            in("x2") x2,
            in("x3") x3,
            in("x4") x4,
            lateout("x0") x0,
            lateout("x1") r1,
            lateout("x2") r2,
            lateout("x3") r3,
            lateout("x4") _,
            lateout("x5") _,
            lateout("x6") _,
            lateout("x7") _,
            lateout("x8") _,
            lateout("x9") _,
            lateout("x10") _,
            lateout("x11") _,
            lateout("x12") _,
            lateout("x13") _,
            lateout("x14") _,
            lateout("x15") _,
            lateout("x16") _,
            lateout("x17") _,
        );
    }
    SmcResult {
        x0: x0,
        x1: r1,
        x2: r2,
        x3: r3,
    }
}

pub fn rsi_request_version(req: u64) -> (u64, u64, u64) {
    let res = smc(SMC_RSI_ABI_VERSION, req, 0, 0, 0);
    (res.x0, res.x1, res.x2)
}

pub fn rsi_get_realm_config(cfg: &mut RealmConfig) -> u64 {
    let cfg_phys = cfg as *const RealmConfig as u64;
    let res = smc(SMC_RSI_REALM_CONFIG, cfg_phys, 0, 0, 0);
    res.x0
}

pub fn rsi_ipa_state_get(start: u64, end: u64) -> (u64, u64, u64) {
    let res = smc(SMC_RSI_IPA_STATE_GET, start, end, 0, 0);
    (res.x0, res.x1, res.x2)
}

pub fn rsi_ipa_state_set(start: u64, end: u64, state: Ripas, flags: u64) -> (u64, u64, u64) {
    let res = smc(SMC_RSI_IPA_STATE_SET, start, end, state as u64, flags);
    (res.x0, res.x1, res.x2)
}

pub fn rsi_set_addr_range_state(start: u64, end: u64, state: Ripas, flags: u64) -> Result<u64, u64> {
    let mut current = start;
    while current < end {
        let (status, top, accepted) = rsi_ipa_state_set(current, end, state, flags);
        if status != RSI_SUCCESS {
            return Err(status);
        }
        if accepted != RSI_ACCEPT {
            return Err(RSI_ERROR_STATE);
        }
        if top <= current || top > end {
            return Err(RSI_ERROR_INPUT);
        }
        current = top;
    }
    Ok(end)
}

pub fn rsi_set_memory_range_protected(start: u64, end: u64) -> Result<u64, u64> {
    rsi_set_addr_range_state(start, end, Ripas::Ram, RSI_CHANGE_DESTROYED)
}

pub fn rsi_set_memory_range_protected_safe(start: u64, end: u64) -> Result<u64, u64> {
    rsi_set_addr_range_state(start, end, Ripas::Ram, RSI_NO_CHANGE_DESTROYED)
}

pub fn rsi_set_memory_range_shared(start: u64, end: u64) -> Result<u64, u64> {
    rsi_set_addr_range_state(start, end, Ripas::Empty, RSI_CHANGE_DESTROYED)
}

pub fn rsi_set_mmio_range_protected(start: u64, end: u64) -> Result<u64, u64> {
    rsi_set_addr_range_state(start, end, Ripas::Dev, RSI_CHANGE_DESTROYED)
}

pub fn rsi_host_call(ipa: u64) -> u64 {
    let res = smc(SMC_RSI_HOST_CALL, ipa, 0, 0, 0);
    res.x0
}

pub fn rsi_measurement_read(index: u64) -> (u64, [u64; 8]) {
    let res = smc(SMC_RSI_MEASUREMENT_READ, index, 0, 0, 0);
    let res2 = smc(SMC_RSI_MEASUREMENT_READ, index, 0, 0, 0);
    let measurements = [res.x1, res.x2, res.x3, 0, res2.x1, res2.x2, res2.x3, 0];
    (res.x0, measurements)
}

pub fn rsi_attestation_token_init(challenge: &[u8; 64]) -> (u64, u64) {
    let c0 = u64::from_ne_bytes(challenge[0..8].try_into().unwrap());
    let c1 = u64::from_ne_bytes(challenge[8..16].try_into().unwrap());
    let c2 = u64::from_ne_bytes(challenge[16..24].try_into().unwrap());
    let c3 = u64::from_ne_bytes(challenge[24..32].try_into().unwrap());
    let c4 = u64::from_ne_bytes(challenge[32..40].try_into().unwrap());
    let c5 = u64::from_ne_bytes(challenge[40..48].try_into().unwrap());
    let c6 = u64::from_ne_bytes(challenge[48..56].try_into().unwrap());
    let _c7 = u64::from_ne_bytes(challenge[56..64].try_into().unwrap());

    let x0: u64;
    let x1: u64;
    unsafe {
        asm!(
            "smc #0",
            in("x0") SMC_RSI_ATTESTATION_TOKEN_INIT,
            in("x1") c0,
            in("x2") c1,
            in("x3") c2,
            in("x4") c3,
            in("x5") c4,
            in("x6") c5,
            in("x7") c6,
            lateout("x0") x0,
            lateout("x1") x1,
            lateout("x2") _,
            lateout("x3") _,
            lateout("x4") _,
            lateout("x5") _,
            lateout("x6") _,
            lateout("x7") _,
            lateout("x8") _,
            lateout("x9") _,
            lateout("x10") _,
            lateout("x11") _,
            lateout("x12") _,
            lateout("x13") _,
            lateout("x14") _,
            lateout("x15") _,
            lateout("x16") _,
            lateout("x17") _,
        );
    }
    (x0, x1)
}

pub fn rsi_attestation_token_continue(granule: u64, offset: u64, size: u64) -> (u64, u64) {
    let res = smc(SMC_RSI_ATTESTATION_TOKEN_CONTINUE, granule, offset, size, 0);
    (res.x0, res.x1)
}

static mut RSI_PRESENT: bool = false;
static mut PROT_NS_SHARED: u64 = 0;
static mut SMC_TESTING: bool = false;

pub fn is_realm_world() -> bool {
    unsafe { RSI_PRESENT }
}

pub fn set_smc_testing(v: bool) {
    unsafe { SMC_TESTING = v }
}

pub fn is_smc_testing() -> bool {
    unsafe { SMC_TESTING }
}

pub fn prot_ns_shared() -> u64 {
    unsafe { PROT_NS_SHARED }
}

pub fn init_rsi() -> bool {
    let current_el: u64;
    unsafe {
        core::arch::asm!("mrs {}, CurrentEL", out(reg) current_el);
    }
    let el = (current_el >> 2) & 3;
    if el != 1 {
        crate::uart_puts("RSI: Not at EL1, skip RSI init\n");
        return false;
    }

    set_smc_testing(true);
    let (status, lower, _higher) = rsi_request_version(RSI_ABI_VERSION);

    if status == 0xFFFFFFFF {
        crate::uart_puts("RSI: Not supported (SMC not implemented)\n");
        return false;
    }

    if is_smc_testing() {
        set_smc_testing(false);
    }

    if status != RSI_SUCCESS {
        crate::uart_puts("RSI: Version mismatch, status=");
        crate::uart_put_hex(status);
        crate::uart_puts("\n");
        return false;
    }

    let lower_major = lower >> 16;
    let lower_minor = lower & 0xFFFF;
    crate::uart_puts("RSI: Version ");
    crate::uart_put_hex(lower_major);
    crate::uart_puts(".");
    crate::uart_put_hex(lower_minor);
    crate::uart_puts("\n");

    let mut config = RealmConfig::new();
    if rsi_get_realm_config(&mut config) != RSI_SUCCESS {
        crate::uart_puts("RSI: Failed to get realm config\n");
        return false;
    }

    crate::uart_puts("RSI: IPA bits=");
    crate::uart_put_hex(config.ipa_bits);
    crate::uart_puts("\n");

    unsafe {
        PROT_NS_SHARED = 1u64 << (config.ipa_bits - 1);
        RSI_PRESENT = true;
    }

    crate::uart_puts("RSI: PROT_NS_SHARED=");
    crate::uart_put_hex(prot_ns_shared());
    crate::uart_puts("\n");

    true
}

pub fn accept_memory(start: u64, end: u64) -> Result<(), u64> {
    if !is_realm_world() {
        return Ok(());
    }
    rsi_set_memory_range_protected_safe(start, end)?;
    Ok(())
}

pub fn mark_shared(start: u64, end: u64) -> Result<(), u64> {
    if !is_realm_world() {
        return Ok(());
    }
    rsi_set_memory_range_shared(start, end)?;
    Ok(())
}

pub fn mark_mmio_protected(start: u64, end: u64) -> Result<(), u64> {
    if !is_realm_world() {
        return Ok(());
    }
    rsi_set_mmio_range_protected(start, end)?;
    Ok(())
}
