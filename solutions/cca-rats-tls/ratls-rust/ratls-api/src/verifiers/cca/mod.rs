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

//! Built-in CCA evidence verifier.
//!
//! The internal layout follows Global Trust Authority's CCA verifier:
//! evidence orchestrates certificate and token verification; token owns the
//! Platform/Realm relationship; Platform and Realm decode their own claims.

use openssl::hash::{hash, MessageDigest};

use crate::core::dice::parse_claims_buffer;
use crate::verifiers::{Evidence, Verifier, VerifierPlugin};
use crate::{RaTlsError, Result};

pub mod constants;
pub mod evidence;
pub mod platform;
pub mod realm;
pub mod token;
pub mod tools;
pub mod trust_anchors;

use self::evidence::CCAEvidence;

/// DICE tag used for CCA evidence.
pub const EVIDENCE_TAG: u64 = 0x1a7507;

#[derive(Debug, Clone, Copy, Default)]
pub struct CcaVerifier;

impl Verifier for CcaVerifier {
    fn name(&self) -> &'static str {
        "cca"
    }

    fn evidence_tag(&self) -> u64 {
        EVIDENCE_TAG
    }

    fn verify_evidence(&mut self, evidence: &mut Evidence) -> Result<()> {
        self.verify_input(evidence)?;

        // The CCA Realm challenge carries the SHA-256 of the complete
        // RATS-TLS claims buffer. This is the nonce passed to the GTA-style
        // cryptographic verifier.
        let evidence_nonce = hash(MessageDigest::sha256(), &evidence.claims_buffer)?;
        let cca_evidence = CCAEvidence::from_raw(&evidence.raw_evidence)?;
        let cca_token = cca_evidence.crypto_verification(Some(evidence_nonce.as_ref()))?;

        // These bindings are RA-TLS-specific and intentionally remain outside
        // the generic CCA token verifier.
        self.verify_ratls_claims(evidence)?;

        evidence.evidence_json = cca_token.to_json_value();
        crate::rtls_info!("CCA certificate chain and token verified");
        Ok(())
    }
}

impl CcaVerifier {
    fn verify_input(&self, evidence: &Evidence) -> Result<()> {
        if !matches!(evidence.name, VerifierPlugin::Cca) {
            return Err(RaTlsError::InvalidArgument(format!(
                "CCA verifier received evidence for '{}'",
                evidence.name
            )));
        }
        if evidence.tag != self.evidence_tag() {
            return Err(RaTlsError::InvalidData(format!(
                "unsupported CCA evidence tag: {:#x}",
                evidence.tag
            )));
        }
        if evidence.claims_buffer.is_empty() {
            return Err(RaTlsError::InvalidData(
                "CCA evidence has an empty claims buffer".into(),
            ));
        }
        Ok(())
    }

    fn verify_ratls_claims(&self, evidence: &Evidence) -> Result<()> {
        let claims = parse_claims_buffer(&evidence.claims_buffer)?;

        if claims.pubkey_hash_algo != 1 {
            return Err(RaTlsError::Unsupported(format!(
                "unsupported public-key hash algorithm: {}",
                claims.pubkey_hash_algo
            )));
        }
        let actual_public_key_hash = hash(MessageDigest::sha256(), &evidence.public_key_der)?;
        if claims.pubkey_hash.as_slice() != actual_public_key_hash.as_ref() {
            return Err(RaTlsError::InvalidData(
                "claims public-key hash does not match the peer certificate".into(),
            ));
        }

        if claims.client_key_share_hash_algo != 1 {
            return Err(RaTlsError::Unsupported(format!(
                "unsupported ClientHello key-share hash algorithm: {}",
                claims.client_key_share_hash_algo
            )));
        }
        if evidence.expected_client_key_share_hash.len() != 32 {
            return Err(RaTlsError::InvalidArgument(format!(
                "expected a 32-byte ClientHello key-share hash, got {}",
                evidence.expected_client_key_share_hash.len()
            )));
        }
        if claims.client_key_share_hash != evidence.expected_client_key_share_hash {
            return Err(RaTlsError::InvalidData(
                "claims key-share hash does not match the TLS ClientHello".into(),
            ));
        }

        if claims.nonce.as_deref() != Some(evidence.expected_nonce.as_slice()) {
            return Err(RaTlsError::InvalidData(
                "claims nonce does not match the TLS handshake nonce".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ciborium::Value;

    use super::*;
    use crate::verifiers::Evidence;

    fn encoded_hash(algorithm: u64, digest: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(
            &Value::Array(vec![
                Value::Integer(algorithm.into()),
                Value::Bytes(digest.to_vec()),
            ]),
            &mut encoded,
        )
        .unwrap();
        encoded
    }

    fn claims(
        public_key_hash_algorithm: u64,
        public_key_hash: &[u8],
        key_share_algorithm: u64,
        key_share_hash: &[u8],
        nonce: &[u8],
    ) -> Vec<u8> {
        let value = Value::Map(vec![
            (
                Value::Text("pubkey-hash".into()),
                Value::Bytes(encoded_hash(public_key_hash_algorithm, public_key_hash)),
            ),
            (
                Value::Text("client-key-share-hash".into()),
                Value::Bytes(encoded_hash(key_share_algorithm, key_share_hash)),
            ),
            (Value::Text("nonce".into()), Value::Bytes(nonce.to_vec())),
        ]);
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&value, &mut encoded).unwrap();
        encoded
    }

    fn evidence() -> Evidence {
        let public_key_der = b"public-key".to_vec();
        let public_key_hash = hash(MessageDigest::sha256(), &public_key_der).unwrap();
        let key_share_hash = vec![2; 32];
        let nonce = vec![3; 32];
        Evidence {
            tag: EVIDENCE_TAG,
            name: VerifierPlugin::Cca,
            raw_evidence: vec![1],
            claims_buffer: claims(1, &public_key_hash, 1, &key_share_hash, &nonce),
            public_key_der,
            expected_nonce: nonce,
            expected_client_key_share_hash: key_share_hash,
            evidence_json: serde_json::Value::Null,
        }
    }

    #[test]
    fn validates_cca_input_and_all_ratls_bindings() {
        let verifier = CcaVerifier;
        let valid = evidence();
        assert_eq!(verifier.name(), "cca");
        assert_eq!(verifier.evidence_tag(), EVIDENCE_TAG);
        assert!(verifier.verify_input(&valid).is_ok());
        assert!(verifier.verify_ratls_claims(&valid).is_ok());

        let mut bad = evidence();
        bad.tag = 1;
        assert!(verifier.verify_input(&bad).is_err());
        let mut bad = evidence();
        bad.claims_buffer.clear();
        assert!(verifier.verify_input(&bad).is_err());
    }

    #[test]
    fn rejects_unsupported_algorithms_and_mismatched_bindings() {
        let verifier = CcaVerifier;
        let mut item = evidence();
        let public_key_hash = hash(MessageDigest::sha256(), &item.public_key_der).unwrap();

        item.claims_buffer = claims(
            2,
            &public_key_hash,
            1,
            &item.expected_client_key_share_hash,
            &item.expected_nonce,
        );
        assert!(verifier.verify_ratls_claims(&item).is_err());

        item = evidence();
        item.public_key_der.push(0);
        assert!(verifier.verify_ratls_claims(&item).is_err());

        item = evidence();
        item.claims_buffer = claims(
            1,
            &public_key_hash,
            2,
            &item.expected_client_key_share_hash,
            &item.expected_nonce,
        );
        assert!(verifier.verify_ratls_claims(&item).is_err());

        item = evidence();
        item.expected_client_key_share_hash.clear();
        assert!(verifier.verify_ratls_claims(&item).is_err());

        item = evidence();
        item.expected_client_key_share_hash[0] ^= 1;
        assert!(verifier.verify_ratls_claims(&item).is_err());

        item = evidence();
        item.expected_nonce[0] ^= 1;
        assert!(verifier.verify_ratls_claims(&item).is_err());
    }
}
