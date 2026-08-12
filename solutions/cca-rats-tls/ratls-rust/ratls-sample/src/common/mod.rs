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

//! Shared sample helpers.
//!
//! These modules intentionally sit outside `ratls-api` because they represent
//! application-side policy and transport choices rather than the generic
//! attester/verifier/plugin API.

/// CCEL table parsing, CCA event log parsing, REM replay, and firmware state extraction.
pub mod cli;
pub mod event_log;
/// JSON firmware baseline loading and firmware state verification.
pub mod firmware_policy;
/// Length-prefixed application data framing over an established RA-TLS channel.
pub mod frame;
/// Binary IMA log parsing and digest baseline verification.
pub mod ima_log;
/// JSON CCA platform component policy verification.
pub mod platform_policy;
/// Shared CLI options for certificate issuance and standard TLS verification.
pub mod tls_config;
