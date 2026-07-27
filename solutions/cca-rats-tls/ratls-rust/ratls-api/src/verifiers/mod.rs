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

//! Typed verifier adapter and runtime type-erasure definitions.

use crate::verifiers::cca::CcaVerifier;
use crate::RaTlsError;
/// CCA verifier implementation.
use std::fmt;

pub mod cca;

pub struct Evidence {
    pub tag: u64,
    pub name: VerifierPlugin,
    pub raw_evidence: Vec<u8>,
    pub claims_buffer: Vec<u8>,
    /// Peer certificate public key in SubjectPublicKeyInfo DER form.
    pub public_key_der: Vec<u8>,
    /// TLS handshake nonce expected in the claims buffer.
    pub expected_nonce: Vec<u8>,
    /// SHA-256 of the ClientHello key_share bytes.
    pub expected_client_key_share_hash: Vec<u8>,
    /// Plugin-neutral verified evidence exposed to the user callback.
    pub evidence_json: serde_json::Value,
}

/// Trait implemented by evidence verification backends.
pub trait Verifier: Send + 'static {
    /// Return the stable verifier name used by configuration.
    fn name(&self) -> &'static str;
    /// Return the DICE/CBOR tag supported by this verifier.
    fn evidence_tag(&self) -> u64;
    /// Verify certificate-bound evidence and return the public claim view.
    fn verify_evidence(&mut self, evidence: &mut Evidence) -> Result<(), RaTlsError>;
}

pub struct VerifierRegistry;

impl VerifierRegistry {
    /// Load a verifier implementation.
    pub fn load(plugin: VerifierPlugin) -> Result<Box<dyn Verifier>, RaTlsError> {
        match plugin {
            VerifierPlugin::Cca => Ok(Box::new(CcaVerifier)),
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum VerifierPlugin {
    Cca = 1,
}

impl fmt::Display for VerifierPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            VerifierPlugin::Cca => "Cca",
        };
        f.write_str(name)
    }
}

impl TryFrom<u32> for VerifierPlugin {
    type Error = RaTlsError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Cca),
            other => Err(RaTlsError::InvalidArgument(format!(
                "invalid Verifier plugin value: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_c_selector_expose_the_cca_verifier() {
        let verifier = VerifierRegistry::load(VerifierPlugin::Cca).unwrap();
        assert_eq!(verifier.name(), "cca");
        assert_eq!(VerifierPlugin::Cca.to_string(), "Cca");
        assert!(matches!(
            VerifierPlugin::try_from(1).unwrap(),
            VerifierPlugin::Cca
        ));
        assert!(VerifierPlugin::try_from(0).is_err());
    }
}
