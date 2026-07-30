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

//! Protocol-independent RA-TLS data structures and encoders.
//!
//! This module contains the evidence layout embedded into RA-TLS certificates,
//! claims buffer encoding, and certificate generation orchestration.

use crate::attesters::{Attester, AttesterPlugin};
use crate::core::dice::CustomClaim;
use crate::crypto_wrappers::{CryptoWrapper, CryptoWrapperEnum};
use crate::tls_wrappers::{TlsWrapper, TlsWrapperEnum};
use crate::verifiers::{Evidence, Verifier, VerifierPlugin};
use crate::RaTlsError;

mod claims;
pub mod dice;
pub(crate) mod engine;
pub mod evidence;

pub struct RaTlsConf {
    pub attester: Option<AttesterPlugin>,
    pub verifier: Option<VerifierPlugin>,
    pub tls_type: TlsWrapperEnum,
    pub crypto_type: CryptoWrapperEnum,
    pub cert_algo: crate::crypto_wrappers::CertAlgorithm,
    pub custom_claims: Vec<CustomClaim>,
    pub mutual: bool,
    pub server: bool,
    pub certificate: CertificateConf,
    pub tls_verify: TlsVerifyConf,
}

/// Configuration used to sign the dynamically generated RA-TLS certificate.
#[derive(Debug, Clone, Default)]
pub struct CertificateConf {
    /// Optional unencrypted PEM CA private key.
    ///
    /// When absent together with `issuer_certificate_chain`, the generated
    /// RA-TLS leaf certificate is self-signed.
    pub issuer_private_key: Vec<u8>,
    /// Optional PEM CA certificate bundle corresponding to `issuer_private_key`.
    pub issuer_certificate_chain: Vec<u8>,
    /// Optional SAN entries such as `DNS:server.example.com` or `IP:192.0.2.10`.
    pub subject_alt_names: Vec<String>,
}

/// Standard TLS peer-certificate verification configuration.
#[derive(Debug, Clone, Default)]
pub struct TlsVerifyConf {
    /// Enable OpenSSL trust-anchor and optional peer-name verification.
    ///
    /// Certificate signature, validity, constraints, key usage, and role EKU
    /// checks remain enabled when this setting is `false`.
    pub verify_peer_certificate: bool,
    /// Add OpenSSL's default system trust paths to the verification store.
    pub use_system_ca: bool,
    /// Optional PEM CA bundle added to the verification store.
    pub trusted_ca_chain: Vec<u8>,
    /// Optional DNS name or IP address checked by OpenSSL.
    pub expected_peer_name: Option<String>,
}

impl RaTlsConf {
    fn new(attester: AttesterPlugin, verifier: VerifierPlugin) -> Self {
        Self {
            attester: Some(attester),
            verifier: Some(verifier),
            tls_type: TlsWrapperEnum::OpenSsl,
            crypto_type: CryptoWrapperEnum::OpenSsl,
            cert_algo: crate::crypto_wrappers::CertAlgorithm::default(),
            custom_claims: Vec::new(),
            mutual: false,
            server: false,
            certificate: CertificateConf::default(),
            tls_verify: TlsVerifyConf::default(),
        }
    }
}

impl Default for RaTlsConf {
    fn default() -> Self {
        Self::new(AttesterPlugin::Cca, VerifierPlugin::Cca)
    }
}

pub type VerificationCallback = Box<dyn FnMut(&Evidence) -> Result<(), RaTlsError> + Send>;

pub struct RaTlsHandle {
    pub conf: RaTlsConf,
    pub attester: Option<Box<dyn Attester>>,
    pub verifier: Option<Box<dyn Verifier>>,
    pub crypto: Box<dyn CryptoWrapper>,
    pub tls: Box<dyn TlsWrapper>,
    pub(crate) certificate_issuer: Option<crate::crypto_wrappers::RatlsCertificateIssuer>,
    pub(crate) user_callback: Option<VerificationCallback>,
}

const CERT_SUBJECT_ORGANIZATION: &str = "RATS-TLS Attestation";
const CERT_SUBJECT_COMMON_NAME: &str = "RA-TLS";
pub const DICE_TAGGED_EVIDENCE_OID: &str = "2.23.133.5.4.9";
