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

const TSI_ABI_VERSION_MAJOR: u64 = 1;
const TSI_ABI_VERSION_MINOR: u64 = 0;
pub const TSI_ABI_VERSION: u64 = (TSI_ABI_VERSION_MAJOR << 16) | TSI_ABI_VERSION_MINOR;

pub const TSI_SUCCESS: u64 = 0;
pub const TSI_ERROR_INPUT: u64 = 1;
pub const TSI_ERROR_STATE: u64 = 2;
pub const TSI_INCOMPLETE: u64 = 3;

pub const GRANULE_SIZE: usize = 0x1000;
pub const CHALLENGE_SIZE: usize = 64;
pub const MAX_MEASUREMENT_SIZE: usize = 64;
pub const MAX_TOKEN_GRANULE_COUNT: usize = 2;
pub const MAX_DEV_CERT_SIZE: usize = 4096;

pub const MEASUREMENT_SLOT_NR: u64 = 5;
pub const RIM_MEASUREMENT_SLOT: u64 = 0;

const SMC_TSI_FID_BASE: u64 = 0xC400_0000;

const fn smc_tsi_fid(n: u64) -> u64 {
    SMC_TSI_FID_BASE | n
}

pub const SMC_TSI_ABI_VERSION: u64 = smc_tsi_fid(0x190);
pub const SMC_TSI_MEASUREMENT_READ: u64 = smc_tsi_fid(0x192);
pub const SMC_TSI_MEASUREMENT_EXTEND: u64 = smc_tsi_fid(0x193);
pub const SMC_TSI_ATTESTATION_TOKEN_INIT: u64 = smc_tsi_fid(0x194);
pub const SMC_TSI_ATTESTATION_TOKEN_CONTINUE: u64 = smc_tsi_fid(0x195);
pub const SMC_TSI_CVM_CONFIG: u64 = smc_tsi_fid(0x196);
pub const SMC_TSI_DEVICE_CERT: u64 = smc_tsi_fid(0x19A);
pub const SMC_TSI_SEC_MEM_UNMAP: u64 = smc_tsi_fid(0x19C);
pub const SMC_TSI_MIGVM_GET_ATTR: u64 = smc_tsi_fid(0x19D);
pub const SMC_TSI_MIGVM_SET_SLOT: u64 = smc_tsi_fid(0x19E);
pub const SMC_TSI_MIGVM_PEEK_BINDINGRD: u64 = smc_tsi_fid(0x19F);
pub const SMC_TSI_MIG_INTEGRITY_CHECKSUM_INIT: u64 = smc_tsi_fid(0x1A0);
pub const SMC_TSI_MIG_INTEGRITY_CHECKSUM_LOOP: u64 = smc_tsi_fid(0x1A1);

#[repr(C, align(4096))]
pub struct CvmConfig {
    pub ipa_bits: u64,
    pub algorithm: u64,
}

impl CvmConfig {
    pub fn new() -> Self {
        Self { ipa_bits: 0, algorithm: 0 }
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
pub fn smc(fid: u64, x1: u64, x2: u64, x3: u64) -> SmcResult {
    smc_full(fid, x1, x2, x3, 0, 0, 0, 0)
}

#[inline(always)]
pub fn smc_full(fid: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64, a7: u64) -> SmcResult {
    let x0: u64;
    let r1: u64;
    let r2: u64;
    let r3: u64;
    unsafe {
        asm!(
            "smc #0",
            in("x0") fid,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            in("x5") a5,
            in("x6") a6,
            in("x7") a7,
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
    SmcResult { x0, x1: r1, x2: r2, x3: r3 }
}

static mut TSI_PRESENT: bool = false;
static mut TSI_PROBED: bool = false;

pub fn is_tsi_present() -> bool {
    unsafe {
        if !TSI_PROBED {
            TSI_PROBED = true;
            crate::rsi::set_smc_testing(true);
            let res = smc(SMC_TSI_ABI_VERSION, TSI_ABI_VERSION, 0, 0);
            crate::rsi::set_smc_testing(false);
            TSI_PRESENT = res.x0 != 0xFFFFFFFF && res.x0 == TSI_ABI_VERSION;
        }
        TSI_PRESENT
    }
}

pub fn is_virtcca_world() -> bool {
    is_tsi_present()
}

pub fn tsi_version() -> u64 {
    smc(SMC_TSI_ABI_VERSION, TSI_ABI_VERSION, 0, 0).x0
}

pub fn tsi_get_realm_config() -> Option<CvmConfig> {
    let res = smc(SMC_TSI_CVM_CONFIG, 0, 0, 0);
    if res.x0 != TSI_SUCCESS {
        return None;
    }
    Some(CvmConfig {
        ipa_bits: res.x1,
        algorithm: res.x2,
    })
}

pub fn tsi_measurement_read(index: u64, buf: &mut [u8; MAX_MEASUREMENT_SIZE]) -> u64 {
    let buf_pa = buf.as_ptr() as u64;
    let mut r = SmcResult { x0: 0, x1: 0, x2: 0, x3: 0 };
    let res = smc_full(SMC_TSI_MEASUREMENT_READ, index, buf_pa, &mut r as *mut SmcResult as u64, 0, 0, 0, 0);
    res.x0
}

pub fn tsi_measurement_extend(index: u64, value: &[u8; MAX_MEASUREMENT_SIZE]) -> u64 {
    let value_pa = value.as_ptr() as u64;
    let res = smc(SMC_TSI_MEASUREMENT_EXTEND, index, value.len() as u64, value_pa);
    res.x0
}

#[repr(C)]
pub struct TokenGranule {
    pub head: u64,
    pub ipa: u64,
    pub count: u64,
    pub offset: u64,
    pub size: u64,
    pub num_wr_bytes: u64,
}

pub fn tsi_attestation_token_init(challenge: &[u8; CHALLENGE_SIZE]) -> (u64, u64) {
    let chal_pa = challenge.as_ptr() as u64;
    let res = smc(SMC_TSI_ATTESTATION_TOKEN_INIT, chal_pa, 0, 0);
    (res.x0, res.x1)
}

pub fn tsi_attestation_token_init_raw(challenge_ipa: u64) -> (u64, u64) {
    // SMCCC 1.1: x1=challenge_ipa, x2=&smccc_res (32-byte result buffer)
    // x2 must be a valid IPA where TMM writes {a0,a1,a2,a3}
    let mut smccc_res = SmcResult { x0: 0, x1: 0, x2: 0, x3: 0 };
    let res_pa = &mut smccc_res as *mut SmcResult as u64;
    let res = smc_full(SMC_TSI_ATTESTATION_TOKEN_INIT, challenge_ipa, res_pa, 0, 0, 0, 0, 0);
    (res.x0, res.x1)
}

pub fn tsi_attestation_token_continue(granule: &mut TokenGranule) -> u64 {
    // SMCCC 1.1: x1=ipa, x2=offset, x3=size, x4=&smccc_res
    // x4 must be a valid IPA (NOT the struct PA — TMM writes {a0,a1,a2,a3} there)
    let mut smccc_res = SmcResult { x0: 0, x1: 0, x2: 0, x3: 0 };
    let res = smc_full(
        SMC_TSI_ATTESTATION_TOKEN_CONTINUE,
        granule.ipa,
        granule.offset,
        granule.size,
        &mut smccc_res as *mut SmcResult as u64, 0, 0, 0,
    );
    granule.num_wr_bytes = res.x1;
    res.x0
}

pub fn tsi_device_cert(buf: &mut [u8; MAX_DEV_CERT_SIZE]) -> (u64, u64) {
    let buf_pa = buf.as_ptr() as u64;
    let res = smc(SMC_TSI_DEVICE_CERT, buf_pa, MAX_DEV_CERT_SIZE as u64, 0);
    (res.x0, res.x1)
}

pub fn tsi_device_cert_raw(ipa: u64, size: u64) -> SmcResult {
    smc(SMC_TSI_DEVICE_CERT, ipa, size, 0)
}

pub fn tsi_sec_mem_unmap(paddr: u64, size: u64) -> u64 {
    smc(SMC_TSI_SEC_MEM_UNMAP, paddr, size, 0).x0
}

pub fn tsi_migvm_get_attr(guest_rd: u64, buf_pa: u64) -> u64 {
    smc(SMC_TSI_MIGVM_GET_ATTR, guest_rd, buf_pa, 0).x0
}

pub fn tsi_migvm_set_slot(guest_rd: u64, buf_pa: u64) -> u64 {
    smc(SMC_TSI_MIGVM_SET_SLOT, guest_rd, buf_pa, 0).x0
}

pub fn tsi_peek_binding_list(buf_pa: u64) -> u64 {
    smc(SMC_TSI_MIGVM_PEEK_BINDINGRD, buf_pa, 0, 0).x0
}

pub fn tsi_mig_integrity_checksum_init(dst_rd: u64, queue_pa: u64) -> u64 {
    smc(SMC_TSI_MIG_INTEGRITY_CHECKSUM_INIT, dst_rd, queue_pa, 0).x0
}

pub fn tsi_mig_integrity_checksum_loop(guest_rd: u64, thread_id: u64) -> u64 {
    smc(SMC_TSI_MIG_INTEGRITY_CHECKSUM_LOOP, guest_rd, thread_id, 0).x0
}

pub fn prot_ns_shared() -> u64 {
    if !is_virtcca_world() {
        return 0;
    }
    0
}

pub fn accept_memory(_start: u64, _end: u64) -> Result<u64, u64> {
    Ok(_end)
}

pub fn mark_shared(start: u64, end: u64) -> Result<u64, u64> {
    let size = end - start;
    if tsi_sec_mem_unmap(start, size) != TSI_SUCCESS {
        return Err(TSI_ERROR_STATE);
    }
    Ok(end)
}

pub fn mark_private(_start: u64, _end: u64) -> Result<u64, u64> {
    Ok(_end)
}
