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

use ciborium::Value;
use coset::{CborSerializable, CoseKey};

use super::constants::{realm_labels, realm_sizes};
use super::tools::Decode;
use crate::{RaTlsError, Result};

/// Decoded CCA Realm claims-set.
#[derive(Debug)]
pub struct Realm {
    pub challenge: [u8; realm_sizes::CHALLENGE],
    pub profile: String,
    pub rpv: [u8; realm_sizes::RPV],
    pub rim: Vec<u8>,
    pub rem: [Vec<u8>; realm_sizes::REM_ARR],
    pub hash_alg: String,
    pub rak: Vec<u8>,
    pub rak_hash_alg: String,
    pub rak_cose_key: CoseKey,
}

impl Default for Realm {
    fn default() -> Self {
        Self {
            challenge: [0; realm_sizes::CHALLENGE],
            profile: String::new(),
            rpv: [0; realm_sizes::RPV],
            rim: Vec::new(),
            rem: Default::default(),
            hash_alg: String::new(),
            rak: Vec::new(),
            rak_hash_alg: String::new(),
            rak_cose_key: CoseKey::default(),
        }
    }
}

impl Realm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check_profile(&self) -> Result<()> {
        if self.profile.is_empty() {
            return Err(RaTlsError::InvalidData("CCA Realm profile is empty".into()));
        }
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        let value: Value = ciborium::de::from_reader(buf)
            .map_err(|err| RaTlsError::Cbor(format!("failed to decode CCA Realm: {err}")))?;
        let Value::Map(contents) = value else {
            return Err(RaTlsError::InvalidData(
                "CCA Realm claims-set must be a map".into(),
            ));
        };

        let mut realm = Self::new();
        for (key, value) in contents {
            let Some(label) = key.as_integer().map(i128::from) else {
                continue;
            };
            match label {
                realm_labels::CHALLENGE => {
                    let bytes = Decode::get_bytes(&value, "challenge", &[realm_sizes::CHALLENGE])?;
                    realm.challenge.copy_from_slice(&bytes);
                }
                realm_labels::PROFILE => realm.profile = Decode::get_string(&value, "profile")?,
                realm_labels::RPV => {
                    let bytes =
                        Decode::get_bytes(&value, "personalization-value", &[realm_sizes::RPV])?;
                    realm.rpv.copy_from_slice(&bytes);
                }
                realm_labels::RIM => {
                    realm.rim = Decode::get_bytes(&value, "initial-measurement", &[])?
                }
                realm_labels::REM => {
                    let measurements = Decode::get_array(
                        &value,
                        "extensible-measurements",
                        Some(realm_sizes::REM_ARR),
                    )?;
                    for (index, measurement) in measurements.iter().enumerate() {
                        realm.rem[index] = Decode::get_bytes(
                            measurement,
                            &format!("extensible-measurement[{index}]"),
                            &[],
                        )?;
                    }
                }
                realm_labels::HASH_ALG => {
                    realm.hash_alg = Decode::get_string(&value, "hash-algo-id")?
                }
                realm_labels::RAK => {
                    realm.rak = Decode::get_bytes(&value, "public-key", &[])?;
                    realm.rak_cose_key = CoseKey::from_slice(&realm.rak).map_err(|err| {
                        RaTlsError::InvalidData(format!(
                            "failed to parse CCA Realm attestation COSE key: {err}"
                        ))
                    })?;
                }
                realm_labels::RAK_HASH_ALG => {
                    realm.rak_hash_alg = Decode::get_string(&value, "public-key-hash-algo-id")?
                }
                _ => {}
            }
        }
        Ok(realm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::{iana, CoseKey, KeyType, Label};

    fn label(value: i128) -> Value {
        Value::Integer((value as i64).into())
    }

    fn sample_cose_key() -> Vec<u8> {
        CoseKey {
            kty: KeyType::Assigned(iana::KeyType::EC2),
            params: vec![
                (Label::Int(-1), Value::Integer(3.into())),
                (Label::Int(-2), Value::Bytes(vec![1; 66])),
                (Label::Int(-3), Value::Bytes(vec![2; 66])),
            ],
            ..Default::default()
        }
        .to_vec()
        .unwrap()
    }

    #[test]
    fn decodes_realm_claims() {
        let claims = Value::Map(vec![
            (label(realm_labels::CHALLENGE), Value::Bytes(vec![1; 64])),
            (label(realm_labels::PROFILE), Value::Text("profile".into())),
            (label(realm_labels::RPV), Value::Bytes(vec![2; 64])),
            (label(realm_labels::RIM), Value::Bytes(vec![3; 32])),
            (
                label(realm_labels::REM),
                Value::Array((0..4).map(|_| Value::Bytes(vec![4; 32])).collect()),
            ),
            (label(realm_labels::HASH_ALG), Value::Text("sha-256".into())),
            (label(realm_labels::RAK), Value::Bytes(sample_cose_key())),
            (
                label(realm_labels::RAK_HASH_ALG),
                Value::Text("sha-256".into()),
            ),
        ]);
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&claims, &mut encoded).unwrap();

        let realm = Realm::decode(&encoded).unwrap();
        assert_eq!(realm.profile, "profile");
        assert_eq!(realm.rem[3], vec![4; 32]);
    }
}
