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

//! CCA CBOR labels and fixed claim sizes.

pub mod collection {
    pub const CBOR_TAG: u64 = 399;
    pub const PLATFORM_LABEL: i128 = 44234;
    pub const REALM_LABEL: i128 = 44241;
}

pub mod realm_labels {
    pub const CHALLENGE: i128 = 10;
    pub const PROFILE: i128 = 265;
    pub const RPV: i128 = 44235;
    pub const HASH_ALG: i128 = 44236;
    pub const RAK: i128 = 44237;
    pub const RIM: i128 = 44238;
    pub const REM: i128 = 44239;
    pub const RAK_HASH_ALG: i128 = 44240;
}

pub mod platform_labels {
    pub const PROFILE: i128 = 265;
    pub const CHALLENGE: i128 = 10;
    pub const IMPL_ID: i128 = 2396;
    pub const INST_ID: i128 = 256;
    pub const CONFIG: i128 = 2401;
    pub const LIFECYCLE: i128 = 2395;
    pub const SW_COMPONENTS: i128 = 2399;
    pub const VERIFICATION_SERVICE: i128 = 2400;
    pub const HASH_ALG: i128 = 2402;

    pub mod sw_component {
        pub const MTYP: i128 = 1;
        pub const MVAL: i128 = 2;
        pub const VERSION: i128 = 4;
        pub const SIGNER_ID: i128 = 5;
        pub const HASH_ALGO: i128 = 6;
    }
}

pub mod realm_sizes {
    pub const CHALLENGE: usize = 64;
    pub const RPV: usize = 64;
    pub const REM_ARR: usize = 4;
}

pub mod platform_sizes {
    pub const CHALLENGE: usize = 32;
    pub const IMPLEMENTATION: usize = 32;
    pub const INSTANCE: usize = 33;
}
