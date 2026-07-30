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

// Copyright 2023-2025 Contributors to the Veraison project.
// SPDX-License-Identifier: Apache-2.0
//
// Adapted from Global Trust Authority's CCA verifier. The original verifier
// uses secGear for COSE verification; this crate uses coset plus OpenSSL.

use ciborium::Value as CborValue;
use coset::{
    iana, CborSerializable, CoseSign1, RegisteredLabelWithPrivate, TaggedCborSerializable,
};
use openssl::bn::BigNum;
use openssl::ecdsa::EcdsaSig;
use openssl::hash::{hash, MessageDigest};
use openssl::pkey::{PKey, Public};
use openssl::sign::Verifier as OpenSslVerifier;
use openssl::x509::X509;
use serde_json::{json, Map, Value};

use super::constants::collection;
use super::platform::Platform;
use super::realm::Realm;
use super::tools::{cose_key_to_uncompressed_bytes, p521_public_key, Decode};
use crate::{hex_encode, RaTlsError, Result};

#[derive(Debug)]
struct CborCollection {
    platform_token: Vec<u8>,
    realm_token: Vec<u8>,
}

impl CborCollection {
    fn decode(token: &[u8]) -> Result<Self> {
        let value: CborValue = ciborium::de::from_reader(token)
            .map_err(|err| RaTlsError::Cbor(format!("failed to decode CCA token: {err}")))?;
        let CborValue::Tag(tag, value) = value else {
            return Err(RaTlsError::InvalidData(
                "CCA token collection is not tagged".into(),
            ));
        };
        if tag != collection::CBOR_TAG {
            return Err(RaTlsError::InvalidData(format!(
                "CCA token collection tag is {tag}, expected {}",
                collection::CBOR_TAG
            )));
        }
        let CborValue::Map(contents) = *value else {
            return Err(RaTlsError::InvalidData(
                "CCA token collection payload is not a map".into(),
            ));
        };

        let mut platform_token = Vec::new();
        let mut realm_token = Vec::new();
        for (key, value) in contents {
            let Some(label) = key.as_integer().map(i128::from) else {
                continue;
            };
            match label {
                collection::PLATFORM_LABEL => {
                    platform_token = Decode::get_bytes(&value, "platform token", &[])?
                }
                collection::REALM_LABEL => {
                    realm_token = Decode::get_bytes(&value, "realm token", &[])?
                }
                _ => {}
            }
        }
        if platform_token.is_empty() {
            return Err(RaTlsError::InvalidData(
                "CCA collection is missing the Platform token".into(),
            ));
        }
        if realm_token.is_empty() {
            return Err(RaTlsError::InvalidData(
                "CCA collection is missing the Realm token".into(),
            ));
        }
        Ok(Self {
            platform_token,
            realm_token,
        })
    }
}

/// Decoded CCA token collection.
pub struct CCAToken {
    pub platform_claims: Platform,
    pub realm_claims: Realm,
    pub platform: CoseSign1,
    pub realm: CoseSign1,
}

impl Default for CCAToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CCAToken {
    pub fn new() -> Self {
        Self {
            platform_claims: Platform::new(),
            realm_claims: Realm::new(),
            platform: CoseSign1::default(),
            realm: CoseSign1::default(),
        }
    }

    pub fn parse(token: &[u8]) -> Result<Self> {
        let collection = CborCollection::decode(token)?;
        let platform = decode_sign1(&collection.platform_token, "Platform")?;
        let realm = decode_sign1(&collection.realm_token, "Realm")?;
        ensure_es512(&platform, "Platform")?;
        ensure_es512(&realm, "Realm")?;

        let platform_payload = platform.payload.as_deref().ok_or_else(|| {
            RaTlsError::InvalidData("CCA Platform COSE_Sign1 has detached payload".into())
        })?;
        let realm_payload = realm.payload.as_deref().ok_or_else(|| {
            RaTlsError::InvalidData("CCA Realm COSE_Sign1 has detached payload".into())
        })?;
        let platform_claims = Platform::decode(platform_payload)?;
        let realm_claims = Realm::decode(realm_payload)?;

        Ok(Self {
            platform_claims,
            realm_claims,
            platform,
            realm,
        })
    }

    pub fn verify_platform_token(&self, device_cert: &X509) -> Result<()> {
        self.platform_claims.check_profile()?;
        let device_key = device_cert.public_key()?;
        self.platform
            .verify_signature(b"", |signature, to_be_signed| {
                verify_es512_signature(&device_key, signature, to_be_signed)
            })
            .map_err(|err| {
                RaTlsError::InvalidData(format!(
                    "CCA Platform COSE signature verification failed: {err}"
                ))
            })
    }

    pub fn verify_realm_token(&self, nonce: Option<&[u8]>) -> Result<()> {
        self.realm_claims.check_profile()?;
        self.verify_nonce(nonce)?;

        let rak_raw = cose_key_to_uncompressed_bytes(&self.realm_claims.rak_cose_key)?;
        self.verify_rak(&rak_raw)?;
        let rak_key = p521_public_key(&rak_raw)?;
        self.realm
            .verify_signature(b"", |signature, to_be_signed| {
                verify_es512_signature(&rak_key, signature, to_be_signed)
            })
            .map_err(|err| {
                RaTlsError::InvalidData(format!(
                    "CCA Realm COSE signature verification failed: {err}"
                ))
            })
    }

    fn verify_nonce(&self, nonce: Option<&[u8]>) -> Result<()> {
        let Some(nonce) = nonce else {
            return Ok(());
        };
        if nonce.len() > self.realm_claims.challenge.len()
            || self.realm_claims.challenge[..nonce.len()] != *nonce
        {
            return Err(RaTlsError::InvalidData(
                "CCA Realm challenge does not match the evidence nonce".into(),
            ));
        }
        Ok(())
    }

    fn verify_rak(&self, raw_rak: &[u8]) -> Result<()> {
        let digest = match self.realm_claims.rak_hash_alg.as_str() {
            "sha-256" => MessageDigest::sha256(),
            "sha-384" => MessageDigest::sha384(),
            "sha-512" => MessageDigest::sha512(),
            algorithm => {
                return Err(RaTlsError::Unsupported(format!(
                    "unsupported CCA RAK hash algorithm: {algorithm}"
                )))
            }
        };
        let calculated = hash(digest, raw_rak)?;
        if calculated.as_ref() != self.platform_claims.challenge {
            return Err(RaTlsError::InvalidData(
                "CCA Platform challenge does not bind the Realm attestation key".into(),
            ));
        }
        Ok(())
    }

    /// Plugin-neutral claim representation consumed by the public callback.
    pub fn to_json_value(&self) -> Value {
        let platform = &self.platform_claims;
        let realm = &self.realm_claims;
        let mut claims = Map::new();
        claims.insert("cca_platform_profile".into(), json!(platform.profile));
        claims.insert(
            "cca_platform_challenge".into(),
            json!(hex_encode(&platform.challenge)),
        );
        claims.insert(
            "cca_platform_implementation_id".into(),
            json!(hex_encode(&platform.impl_id)),
        );
        claims.insert(
            "cca_platform_instance_id".into(),
            json!(hex_encode(&platform.inst_id)),
        );
        claims.insert(
            "cca_platform_config".into(),
            json!(hex_encode(&platform.config)),
        );
        claims.insert("cca_platform_lifecycle".into(), json!(platform.lifecycle));
        claims.insert("cca_platform_hash_alg".into(), json!(platform.hash_alg));
        if let Some(service) = &platform.verification_service {
            claims.insert("cca_platform_verification_service".into(), json!(service));
        }
        claims.insert(
            "cca_platform_sw_components".into(),
            Value::Array(
                platform
                    .sw_components
                    .iter()
                    .map(|component| {
                        json!({
                            "component_type": component.com_type,
                            "measurement_value": hex_encode(&component.mea_val),
                            "version": component.version,
                            "signer_id": hex_encode(&component.signer_id),
                            "hash_algorithm": component.hash_alg,
                        })
                    })
                    .collect(),
            ),
        );

        claims.insert("cca_realm_profile".into(), json!(realm.profile));
        claims.insert(
            "cca_realm_challenge".into(),
            json!(hex_encode(&realm.challenge)),
        );
        claims.insert("cca_realm_rpv".into(), json!(hex_encode(&realm.rpv)));
        claims.insert("cca_realm_rim".into(), json!(hex_encode(&realm.rim)));
        claims.insert("cca_realm_hash_alg".into(), json!(realm.hash_alg));
        claims.insert("cca_realm_pub_key".into(), json!(hex_encode(&realm.rak)));
        claims.insert(
            "cca_realm_pub_key_hash_alg".into(),
            json!(realm.rak_hash_alg),
        );
        for (index, measurement) in realm.rem.iter().enumerate() {
            claims.insert(
                format!("cca_realm_rem{index}"),
                json!(hex_encode(measurement)),
            );
        }
        Value::Object(claims)
    }
}

fn decode_sign1(token: &[u8], name: &str) -> Result<CoseSign1> {
    CoseSign1::from_tagged_slice(token)
        .or_else(|_| CoseSign1::from_slice(token))
        .map_err(|err| {
            RaTlsError::InvalidData(format!("failed to decode CCA {name} COSE_Sign1: {err:?}"))
        })
}

fn ensure_es512(sign1: &CoseSign1, name: &str) -> Result<()> {
    match &sign1.protected.header.alg {
        Some(RegisteredLabelWithPrivate::Assigned(iana::Algorithm::ES512)) => Ok(()),
        algorithm => Err(RaTlsError::InvalidData(format!(
            "CCA {name} COSE algorithm is {algorithm:?}, expected ES512"
        ))),
    }
}

fn verify_es512_signature(key: &PKey<Public>, signature: &[u8], to_be_signed: &[u8]) -> Result<()> {
    if signature.len() != 132 {
        return Err(RaTlsError::InvalidData(format!(
            "ES512 signature must contain 132 raw bytes, got {}",
            signature.len()
        )));
    }
    let r = BigNum::from_slice(&signature[..66])?;
    let s = BigNum::from_slice(&signature[66..])?;
    let signature_der = EcdsaSig::from_private_components(r, s)?.to_der()?;
    let mut verifier = OpenSslVerifier::new(MessageDigest::sha512(), key)?;
    verifier.update(to_be_signed)?;
    if verifier.verify(&signature_der)? {
        Ok(())
    } else {
        Err(RaTlsError::InvalidData(
            "ES512 signature verification failed".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::platform::SwComponent;
    use super::*;
    use coset::{Header, ProtectedHeader};

    #[test]
    fn collection_rejects_wrong_tag() {
        let value = CborValue::Tag(1, Box::new(CborValue::Map(Vec::new())));
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&value, &mut encoded).unwrap();
        assert!(CborCollection::decode(&encoded).is_err());
    }

    #[test]
    fn collection_requires_both_tokens() {
        let value = CborValue::Tag(collection::CBOR_TAG, Box::new(CborValue::Map(Vec::new())));
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&value, &mut encoded).unwrap();
        assert!(CborCollection::decode(&encoded).is_err());
    }

    #[test]
    fn nonce_must_match_the_realm_challenge_prefix() {
        let mut token = CCAToken::new();
        token.realm_claims.challenge = [1; 64];

        assert!(token.verify_nonce(Some(&[1; 32])).is_ok());
        assert!(token.verify_nonce(Some(&[2; 32])).is_err());
        assert!(token.verify_nonce(Some(&[1; 65])).is_err());
    }

    #[test]
    fn rak_binding_rejects_unknown_hash_algorithm() {
        let mut token = CCAToken::new();
        token.realm_claims.rak_hash_alg = "sha-1024".into();

        assert!(token.verify_rak(&[1; 133]).is_err());
    }

    #[test]
    fn collection_decodes_both_token_labels_and_rejects_wrong_payloads() {
        let value = CborValue::Tag(
            collection::CBOR_TAG,
            Box::new(CborValue::Map(vec![
                (
                    CborValue::Integer((collection::PLATFORM_LABEL as i64).into()),
                    CborValue::Bytes(vec![1]),
                ),
                (
                    CborValue::Integer((collection::REALM_LABEL as i64).into()),
                    CborValue::Bytes(vec![2]),
                ),
            ])),
        );
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&value, &mut encoded).unwrap();
        let collection = CborCollection::decode(&encoded).unwrap();
        assert_eq!(collection.platform_token, [1]);
        assert_eq!(collection.realm_token, [2]);

        let wrong = CborValue::Tag(collection::CBOR_TAG, Box::new(CborValue::Array(vec![])));
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&wrong, &mut encoded).unwrap();
        assert!(CborCollection::decode(&encoded).is_err());
    }

    #[test]
    fn rak_binding_accepts_supported_hashes_and_rejects_mismatch() {
        for algorithm in ["sha-256", "sha-384", "sha-512"] {
            let mut token = CCAToken::new();
            token.realm_claims.rak_hash_alg = algorithm.into();
            let digest = match algorithm {
                "sha-256" => MessageDigest::sha256(),
                "sha-384" => MessageDigest::sha384(),
                _ => MessageDigest::sha512(),
            };
            token.platform_claims.challenge = hash(digest, b"rak").unwrap().to_vec();
            assert!(token.verify_rak(b"rak").is_ok());
            token.platform_claims.challenge[0] ^= 1;
            assert!(token.verify_rak(b"rak").is_err());
        }
    }

    #[test]
    fn public_json_contains_platform_components_and_all_rems() {
        let mut token = CCAToken::new();
        token.platform_claims.profile = "platform-profile".into();
        token.platform_claims.challenge = vec![1, 2];
        token.platform_claims.impl_id = [3; 32];
        token.platform_claims.inst_id = [4; 33];
        token.platform_claims.config = vec![5];
        token.platform_claims.lifecycle = 7;
        token.platform_claims.hash_alg = "sha-256".into();
        token.platform_claims.verification_service = Some("service".into());
        token.platform_claims.sw_components.push(SwComponent {
            com_type: Some("firmware".into()),
            mea_val: vec![6; 32],
            version: Some("1".into()),
            signer_id: vec![7; 32],
            hash_alg: Some("sha-256".into()),
        });
        token.realm_claims.profile = "realm-profile".into();
        token.realm_claims.challenge = [8; 64];
        token.realm_claims.rpv = [9; 64];
        token.realm_claims.rim = vec![10; 32];
        token.realm_claims.hash_alg = "sha-256".into();
        token.realm_claims.rak = vec![11];
        token.realm_claims.rak_hash_alg = "sha-512".into();
        token.realm_claims.rem = std::array::from_fn(|index| vec![index as u8; 32]);

        let value = token.to_json_value();
        assert_eq!(value["cca_platform_profile"], "platform-profile");
        assert_eq!(
            value["cca_platform_sw_components"][0]["component_type"],
            "firmware"
        );
        assert_eq!(value["cca_realm_profile"], "realm-profile");
        assert_eq!(value["cca_realm_rem3"], hex_encode(&[3; 32]));
    }

    #[test]
    fn cose_helpers_require_es512_and_valid_encoding() {
        assert!(decode_sign1(b"not-cose", "test").is_err());
        assert!(ensure_es512(&CoseSign1::default(), "test").is_err());

        let sign1 = CoseSign1 {
            protected: ProtectedHeader {
                header: Header {
                    alg: Some(RegisteredLabelWithPrivate::Assigned(iana::Algorithm::ES512)),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(ensure_es512(&sign1, "test").is_ok());

        let group = openssl::ec::EcGroup::from_curve_name(openssl::nid::Nid::SECP521R1).unwrap();
        let key = openssl::ec::EcKey::generate(&group).unwrap();
        let public = PKey::from_ec_key(
            openssl::ec::EcKey::from_public_key(&group, key.public_key()).unwrap(),
        )
        .unwrap();
        assert!(verify_es512_signature(&public, &[0; 3], b"data").is_err());
    }
}
