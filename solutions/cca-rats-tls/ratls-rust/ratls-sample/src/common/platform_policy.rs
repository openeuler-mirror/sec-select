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

//! Platform component baseline policy support.
//!
//! This verifier reads the JSON format used by the CCA sample policy path and
//! checks it against the software component claims extracted from verified CCA
//! evidence.

use std::fs;

use serde::Deserialize;

use ratls_api::{hex_decode, RaTlsError, Result};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct PlatformBaselineFile {
    #[serde(default, alias = "measure-values", alias = "measure_values")]
    measure_value: Vec<PlatformBaselineEntry>,
}

#[derive(Debug, Deserialize)]
struct PlatformBaselineEntry {
    #[serde(alias = "firmware_name")]
    firware_name: String,
    measurement: String,
    #[serde(alias = "firmware_version")]
    firware_version: String,
    hash_algorithm: String,
    #[serde(default)]
    signer_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VerifiedPlatformClaims {
    #[serde(default)]
    cca_platform_sw_components: Vec<VerifiedSoftwareComponent>,
}

#[derive(Debug, Deserialize)]
struct VerifiedSoftwareComponent {
    #[serde(default)]
    component_type: Option<String>,
    measurement_value: String,
    #[serde(default)]
    version: Option<String>,
    signer_id: String,
    #[serde(default)]
    hash_algorithm: Option<String>,
}

/// Verify CCA platform software component claims against a JSON policy file.
///
/// The policy can match component name, version, hash algorithm, measurement,
/// and optionally signer ID. Empty strings, `-`, and `*` are treated as
/// wildcards for text fields.
pub fn verify_platform_policy(path: &str, evidence: &Value) -> Result<()> {
    let baseline = load_platform_policy(path)?;
    let claims: VerifiedPlatformClaims = serde_json::from_value(evidence.clone())?;
    for expected in baseline.measure_value {
        let measurement = hex_decode(&expected.measurement)?;
        let signer_id = expected
            .signer_id
            .as_deref()
            .filter(|value| !is_wildcard(value))
            .map(hex_decode)
            .transpose()?;
        let matched = claims.cca_platform_sw_components.iter().any(|actual| {
            let actual_measurement = hex_decode(&actual.measurement_value).ok();
            let actual_signer = hex_decode(&actual.signer_id).ok();
            text_matches(
                &expected.firware_name,
                actual.component_type.as_deref().unwrap_or_default(),
            ) && text_matches(
                &expected.firware_version,
                actual.version.as_deref().unwrap_or_default(),
            ) && hash_alg_matches(
                &expected.hash_algorithm,
                actual.hash_algorithm.as_deref().unwrap_or_default(),
            ) && actual_measurement.as_ref() == Some(&measurement)
                && signer_id
                    .as_ref()
                    .map(|expected| actual_signer.as_ref() == Some(expected))
                    .unwrap_or(true)
        });
        if !matched {
            return Err(RaTlsError::InvalidData(format!(
                "platform baseline component mismatch: {}",
                expected.firware_name
            )));
        }
    }
    Ok(())
}

/// Parse and validate a platform policy before a network connection is opened.
pub fn validate_platform_policy(path: &str) -> Result<()> {
    load_platform_policy(path).map(|_| ())
}

fn load_platform_policy(path: &str) -> Result<PlatformBaselineFile> {
    let text = fs::read_to_string(path)?;
    let baseline: PlatformBaselineFile = serde_json::from_str(&text)?;
    if baseline.measure_value.is_empty() {
        return Err(RaTlsError::InvalidData(
            "platform baseline has no measure_value".into(),
        ));
    }
    for entry in &baseline.measure_value {
        if entry.measurement.is_empty() {
            return Err(RaTlsError::InvalidData(
                "platform baseline measurement is empty".into(),
            ));
        }
        hex_decode(&entry.measurement)?;
        if let Some(signer_id) = entry
            .signer_id
            .as_deref()
            .filter(|value| !is_wildcard(value))
        {
            hex_decode(signer_id)?;
        }
    }
    Ok(baseline)
}

fn text_matches(expected: &str, actual: &str) -> bool {
    is_wildcard(expected) || expected == actual
}

fn is_wildcard(value: &str) -> bool {
    value.is_empty() || value == "-" || value == "*"
}

fn hash_alg_matches(expected: &str, actual: &str) -> bool {
    text_matches(expected, actual)
        || (expected == "sha256" && actual == "sha-256")
        || (expected == "sha384" && actual == "sha-384")
        || (expected == "sha512" && actual == "sha-512")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use ratls_api::hex_encode;

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn temporary_policy(contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ratls-platform-policy-{}-{}.json",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn validates_and_matches_aliases_wildcards_and_hash_names() {
        let measurement = hex_encode(&[0x11; 32]);
        let signer = hex_encode(&[0x22; 32]);
        let policy = temporary_policy(
            &serde_json::json!({
                "measure-values":[{
                    "firmware_name":"bootloader",
                    "measurement":measurement,
                    "firmware_version":"*",
                    "hash_algorithm":"sha256",
                    "signer_id":signer
                }]
            })
            .to_string(),
        );
        let evidence = serde_json::json!({
            "cca_platform_sw_components":[{
                "component_type":"bootloader",
                "measurement_value":measurement,
                "version":"1.0",
                "signer_id":signer,
                "hash_algorithm":"sha-256"
            }]
        });
        assert!(validate_platform_policy(policy.to_str().unwrap()).is_ok());
        assert!(verify_platform_policy(policy.to_str().unwrap(), &evidence).is_ok());
        fs::remove_file(policy).unwrap();
    }

    #[test]
    fn rejects_empty_invalid_and_mismatched_policies() {
        for contents in [
            "{}",
            r#"{"measure_value":[{"firware_name":"x","measurement":"","firware_version":"-","hash_algorithm":"*"}]}"#,
            r#"{"measure_value":[{"firware_name":"x","measurement":"zz","firware_version":"-","hash_algorithm":"*","signer_id":"*"}]}"#,
        ] {
            let path = temporary_policy(contents);
            assert!(validate_platform_policy(path.to_str().unwrap()).is_err());
            fs::remove_file(path).unwrap();
        }

        let policy = temporary_policy(
            r#"{"measure_value":[{
                "firware_name":"expected",
                "measurement":"00",
                "firware_version":"1",
                "hash_algorithm":"sha-256"
            }]}"#,
        );
        let evidence = serde_json::json!({
            "cca_platform_sw_components":[{
                "component_type":"other",
                "measurement_value":"00",
                "version":"1",
                "signer_id":"00",
                "hash_algorithm":"sha-256"
            }]
        });
        assert!(verify_platform_policy(policy.to_str().unwrap(), &evidence).is_err());
        fs::remove_file(policy).unwrap();
    }
}
