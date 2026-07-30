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

//! Crypto wrapper abstraction.

/// OpenSSL-backed crypto wrapper.
pub mod openssl;

use crate::RaTlsError;
use std::fmt;

/// Generated private key material used while building an RA-TLS certificate.
pub struct RatlsPrivateKey {
    pub cert_algo: CertAlgorithm,
    pub private_key: Vec<u8>,
    pub public_key: Vec<u8>,
    pub cert: Option<Vec<u8>>,
}

/// Prepared CA material used to sign a dynamic RA-TLS leaf certificate.
#[derive(Debug, Clone)]
pub struct RatlsCertificateIssuer {
    /// Normalized unencrypted PKCS#8 issuer private key.
    pub private_key_pkcs8: Vec<u8>,
    /// PEM certificate whose public key matches `private_key_pkcs8`.
    pub issuer_certificate_pem: Vec<u8>,
    /// Ordered PEM issuer chain sent after the leaf, excluding a self-signed root.
    pub certificate_chain_pem: Vec<Vec<u8>>,
}

/// TLS endpoint role used to generate role-specific certificate extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateRole {
    Server,
    Client,
}

/// Inputs used to generate a dynamic RA-TLS certificate.
pub struct RatlsCertificateInfo<'a> {
    /// X509 subject organization.
    pub organization: &'a str,
    /// X509 subject common name.
    pub common_name: &'a str,
    /// Optional encoded DICE evidence extension payload.
    pub evidence_buffer: Option<&'a [u8]>,
    /// Optional CA issuer. `None` generates a self-signed certificate.
    pub issuer: Option<&'a RatlsCertificateIssuer>,
    /// TLS role used for extended-key-usage generation.
    pub role: CertificateRole,
    /// Optional subject alternative names.
    pub subject_alt_names: &'a [String],
}

/// Trait implemented by cryptographic backends.
pub trait CryptoWrapper: Send {
    fn name(&self) -> &'static str;
    fn generate_private_key(&self, algorithm: CertAlgorithm)
        -> Result<RatlsPrivateKey, RaTlsError>;
    fn prepare_certificate_issuer(
        &self,
        private_key: &[u8],
        certificate_chain: &[u8],
    ) -> Result<RatlsCertificateIssuer, RaTlsError>;
    fn generate_ra_certificate(
        &self,
        conf: &mut RatlsPrivateKey,
        cert_info: RatlsCertificateInfo,
    ) -> Result<(), RaTlsError>;
}

/// Registry for loading crypto wrappers by name.
pub struct CryptoWrapperRegistry;

impl CryptoWrapperRegistry {
    /// Load a crypto wrapper implementation by name.
    pub fn load(wrapper_type: CryptoWrapperEnum) -> Result<Box<dyn CryptoWrapper>, RaTlsError> {
        match wrapper_type {
            CryptoWrapperEnum::OpenSsl => Ok(Box::new(openssl::OpenSslCrypto)),
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum CryptoWrapperEnum {
    OpenSsl = 1,
}

impl fmt::Display for CryptoWrapperEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            CryptoWrapperEnum::OpenSsl => "OpenSSL",
        };
        f.write_str(name)
    }
}

impl TryFrom<u32> for CryptoWrapperEnum {
    type Error = RaTlsError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(CryptoWrapperEnum::OpenSsl),
            other => Err(RaTlsError::InvalidArgument(format!(
                "invalid TLS wrapper value: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CertAlgorithm {
    Rsa3072Sha256,
    #[default]
    Ecc256Sha256,
    RsaSha256,
    EcdsaP256Sha256,
    EcdsaP384Sha384,
    EcdsaP521Sha512,
    Ed25519,
    Ed448,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_selector_and_display_use_openssl() {
        assert_eq!(
            CryptoWrapperRegistry::load(CryptoWrapperEnum::OpenSsl)
                .unwrap()
                .name(),
            "openssl"
        );
        assert_eq!(CryptoWrapperEnum::OpenSsl.to_string(), "OpenSSL");
        assert!(matches!(
            CryptoWrapperEnum::try_from(1).unwrap(),
            CryptoWrapperEnum::OpenSsl
        ));
        assert!(CryptoWrapperEnum::try_from(0).is_err());
    }
}
