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

use super::constants::{platform_labels, platform_labels::sw_component, platform_sizes};
use super::tools::Decode;
use crate::{RaTlsError, Result};

#[derive(Debug, Default, PartialEq)]
pub struct SwComponent {
    pub com_type: Option<String>,
    pub mea_val: Vec<u8>,
    pub version: Option<String>,
    pub signer_id: Vec<u8>,
    pub hash_alg: Option<String>,
}

impl SwComponent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode(value: &Value) -> Result<Self> {
        let Value::Map(contents) = value else {
            return Err(RaTlsError::InvalidData(
                "CCA software component must be a map".into(),
            ));
        };
        let mut component = Self::new();
        for (key, value) in contents {
            let Some(label) = key.as_integer().map(i128::from) else {
                continue;
            };
            match label {
                sw_component::MTYP => {
                    component.com_type = Some(Decode::get_string(value, "measurement-type")?)
                }
                sw_component::MVAL => {
                    component.mea_val = Decode::get_bytes(value, "measurement-value", &[])?
                }
                sw_component::VERSION => {
                    component.version = Some(Decode::get_string(value, "version")?)
                }
                sw_component::SIGNER_ID => {
                    component.signer_id = Decode::get_bytes(value, "signer-id", &[])?
                }
                sw_component::HASH_ALGO => {
                    component.hash_alg = Some(Decode::get_string(value, "hash-algo-id")?)
                }
                _ => {}
            }
        }

        let hash_alg = component.hash_alg.as_deref().ok_or_else(|| {
            RaTlsError::InvalidData("CCA software component is missing hash-algo-id".into())
        })?;
        let expected_size = match hash_alg {
            "sha-256" => 32,
            "sha-384" => 48,
            "sha-512" => 64,
            algorithm => {
                return Err(RaTlsError::Unsupported(format!(
                    "unsupported CCA software component hash algorithm: {algorithm}"
                )))
            }
        };
        if component.mea_val.len() != expected_size {
            return Err(RaTlsError::InvalidData(format!(
                "measurement-value for {hash_alg} must contain {expected_size} bytes, got {}",
                component.mea_val.len()
            )));
        }

        Ok(component)
    }
}

/// Decoded CCA Platform claims-set.
#[derive(Debug)]
pub struct Platform {
    pub profile: String,
    pub challenge: Vec<u8>,
    pub impl_id: [u8; platform_sizes::IMPLEMENTATION],
    pub inst_id: [u8; platform_sizes::INSTANCE],
    pub config: Vec<u8>,
    pub lifecycle: u16,
    pub sw_components: Vec<SwComponent>,
    pub verification_service: Option<String>,
    pub hash_alg: String,
}

impl Default for Platform {
    fn default() -> Self {
        Self {
            profile: String::new(),
            challenge: Vec::new(),
            impl_id: [0; platform_sizes::IMPLEMENTATION],
            inst_id: [0; platform_sizes::INSTANCE],
            config: Vec::new(),
            lifecycle: 0,
            sw_components: Vec::new(),
            verification_service: None,
            hash_alg: String::new(),
        }
    }
}

impl Platform {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check_profile(&self) -> Result<()> {
        if self.profile.is_empty() {
            return Err(RaTlsError::InvalidData(
                "CCA Platform profile is empty".into(),
            ));
        }
        Ok(())
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        let value: Value = ciborium::de::from_reader(buf)
            .map_err(|err| RaTlsError::Cbor(format!("failed to decode CCA Platform: {err}")))?;
        let Value::Map(contents) = value else {
            return Err(RaTlsError::InvalidData(
                "CCA Platform claims-set must be a map".into(),
            ));
        };

        let mut platform = Self::new();
        for (key, value) in contents {
            let Some(label) = key.as_integer().map(i128::from) else {
                continue;
            };
            match label {
                platform_labels::PROFILE => {
                    platform.profile = Decode::get_string(&value, "profile")?
                }
                platform_labels::CHALLENGE => {
                    platform.challenge =
                        Decode::get_bytes(&value, "challenge", &[platform_sizes::CHALLENGE])?
                }
                platform_labels::IMPL_ID => {
                    let bytes = Decode::get_bytes(
                        &value,
                        "implementation-id",
                        &[platform_sizes::IMPLEMENTATION],
                    )?;
                    platform.impl_id.copy_from_slice(&bytes);
                }
                platform_labels::INST_ID => {
                    let bytes =
                        Decode::get_bytes(&value, "instance-id", &[platform_sizes::INSTANCE])?;
                    platform.inst_id.copy_from_slice(&bytes);
                }
                platform_labels::CONFIG => {
                    platform.config = Decode::get_bytes(&value, "config", &[])?
                }
                platform_labels::LIFECYCLE => {
                    let lifecycle = Decode::get_num(&value, "lifecycle")?;
                    platform.lifecycle = u16::try_from(lifecycle).map_err(|_| {
                        RaTlsError::InvalidData(format!(
                            "CCA Platform lifecycle is outside u16: {lifecycle}"
                        ))
                    })?;
                }
                platform_labels::SW_COMPONENTS => {
                    for component in Decode::get_array(&value, "sw-components", None)? {
                        platform
                            .sw_components
                            .push(SwComponent::decode(&component)?);
                    }
                }
                platform_labels::VERIFICATION_SERVICE => {
                    platform.verification_service =
                        Some(Decode::get_string(&value, "verification-service")?)
                }
                platform_labels::HASH_ALG => {
                    platform.hash_alg = Decode::get_string(&value, "hash-algo-id")?
                }
                _ => {}
            }
        }
        Ok(platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(value: i128) -> Value {
        Value::Integer((value as i64).into())
    }

    fn software_component(hash_alg: Option<&str>, measurement_size: usize) -> Value {
        let mut claims = vec![(
            label(sw_component::MVAL),
            Value::Bytes(vec![0; measurement_size]),
        )];
        if let Some(hash_alg) = hash_alg {
            claims.push((label(sw_component::HASH_ALGO), Value::Text(hash_alg.into())));
        }
        Value::Map(claims)
    }

    #[test]
    fn validates_software_measurement_size_from_hash_algorithm() {
        for (hash_alg, measurement_size) in [("sha-256", 32), ("sha-384", 48), ("sha-512", 64)] {
            let component =
                SwComponent::decode(&software_component(Some(hash_alg), measurement_size)).unwrap();
            assert_eq!(component.hash_alg.as_deref(), Some(hash_alg));
            assert_eq!(component.mea_val.len(), measurement_size);
        }
    }

    #[test]
    fn rejects_software_measurement_size_mismatch() {
        let error = SwComponent::decode(&software_component(Some("sha-512"), 32)).unwrap_err();
        assert!(error
            .to_string()
            .contains("measurement-value for sha-512 must contain 64 bytes, got 32"));
    }

    #[test]
    fn rejects_unsupported_software_measurement_hash_algorithm() {
        let error = SwComponent::decode(&software_component(Some("sha3-256"), 32)).unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported CCA software component hash algorithm: sha3-256"));
    }

    #[test]
    fn rejects_missing_software_measurement_hash_algorithm() {
        let error = SwComponent::decode(&software_component(None, 32)).unwrap_err();
        assert!(error
            .to_string()
            .contains("CCA software component is missing hash-algo-id"));
    }

    #[test]
    fn decodes_platform_claims() {
        let claims = Value::Map(vec![
            (
                label(platform_labels::PROFILE),
                Value::Text("profile".into()),
            ),
            (label(platform_labels::CHALLENGE), Value::Bytes(vec![1; 32])),
            (label(platform_labels::IMPL_ID), Value::Bytes(vec![2; 32])),
            (label(platform_labels::INST_ID), Value::Bytes(vec![3; 33])),
            (label(platform_labels::LIFECYCLE), Value::Integer(1.into())),
        ]);
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&claims, &mut encoded).unwrap();

        let platform = Platform::decode(&encoded).unwrap();
        assert_eq!(platform.profile, "profile");
        assert_eq!(platform.challenge, vec![1; 32]);
        assert_eq!(platform.lifecycle, 1);
    }
}
