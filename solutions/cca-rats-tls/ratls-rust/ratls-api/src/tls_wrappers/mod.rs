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

pub mod openssl;

use crate::tls_wrappers::openssl::OpenSslTlsWrapper;
use crate::RaTlsError;
use std::fmt;
use std::io::{Read, Write};

pub trait TransportStream: Read + Write + Send {}
impl<T: Read + Write + Send> TransportStream for T {}

#[derive(Debug, Clone)]
pub struct ClientHelloData {
    pub client_random: Vec<u8>,
    pub client_key_share: Vec<u8>,
}

pub struct TlsIdentity {
    /// PEM certificates with the dynamic leaf first, followed by issuer certificates.
    pub certificate_chain_pem: Vec<Vec<u8>>,
    pub private_key_pkcs8: Vec<u8>,
}

/// TLS handshake values bound into a locally generated RA-TLS certificate.
#[derive(Debug, Clone)]
pub struct TlsEvidenceBinding {
    pub nonce: Vec<u8>,
    pub client_key_share: Vec<u8>,
}

pub struct TlsPeerData {
    pub certificate_der: Vec<u8>,
    pub evidence_binding: TlsEvidenceBinding,
}

pub struct TlsNegotiated {
    pub stream: Box<dyn TransportStream>,
    pub peer: Option<TlsPeerData>,
}

pub trait TlsHandshake: Send {
    /// Values needed to generate a local RA-TLS identity.
    ///
    /// A non-mutual client returns `None` because it does not send a certificate.
    fn evidence_binding(&self) -> Result<Option<TlsEvidenceBinding>, RaTlsError>;

    fn install_identity(&mut self, identity: TlsIdentity) -> Result<(), RaTlsError>;

    fn finish(self: Box<Self>) -> Result<TlsNegotiated, RaTlsError>;
}

pub trait TlsWrapper: Send {
    fn name(&self) -> &'static str;

    /// Validate backend-specific peer-verification inputs during handle initialization.
    fn validate_peer_verification(
        &self,
        verify: &crate::core::TlsVerifyConf,
    ) -> Result<(), RaTlsError>;

    fn start_server_handshake(
        &self,
        stream: Box<dyn TransportStream>,
        mutual: bool,
        verify: &crate::core::TlsVerifyConf,
    ) -> Result<Box<dyn TlsHandshake>, RaTlsError>;

    fn start_client_handshake(
        &self,
        stream: Box<dyn TransportStream>,
        mutual: bool,
        verify: &crate::core::TlsVerifyConf,
    ) -> Result<Box<dyn TlsHandshake>, RaTlsError>;
}

pub struct TlsWrapperRegistry;

impl TlsWrapperRegistry {
    pub fn load(wrapper_type: TlsWrapperEnum) -> Result<Box<dyn TlsWrapper>, RaTlsError> {
        match wrapper_type {
            TlsWrapperEnum::OpenSsl => Ok(Box::new(OpenSslTlsWrapper)),
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum TlsWrapperEnum {
    OpenSsl = 1,
}

impl fmt::Display for TlsWrapperEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            TlsWrapperEnum::OpenSsl => "OpenSSL",
        };
        f.write_str(name)
    }
}

impl TryFrom<u32> for TlsWrapperEnum {
    type Error = RaTlsError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::OpenSsl),
            other => Err(RaTlsError::InvalidArgument(format!(
                "invalid Crypto wrapper value: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_selector_and_display_use_openssl() {
        assert_eq!(
            TlsWrapperRegistry::load(TlsWrapperEnum::OpenSsl)
                .unwrap()
                .name(),
            "openssl"
        );
        assert_eq!(TlsWrapperEnum::OpenSsl.to_string(), "OpenSSL");
        assert!(matches!(
            TlsWrapperEnum::try_from(1).unwrap(),
            TlsWrapperEnum::OpenSsl
        ));
        assert!(TlsWrapperEnum::try_from(0).is_err());
    }
}
