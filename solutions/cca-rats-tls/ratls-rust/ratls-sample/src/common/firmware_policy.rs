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

//! Firmware baseline policy support.
//!
//! The firmware baseline is a JSON file checked against firmware measurements
//! extracted from the CCEL event log. It covers GRUB, `grub.cfg`, and one of
//! the allowed kernel/initramfs pairs.

use std::fs;

use serde::Deserialize;

use crate::common::event_log::FirmwareState;
use ratls_api::{hex_decode, RaTlsError, Result};

/// JSON firmware baseline used by the sample.
#[derive(Debug, Deserialize)]
pub struct FirmwareBaseline {
    /// Hash algorithm name. The current sample accepts `sha-256`.
    pub hash_alg: String,
    /// Expected GRUB EFI image SHA-256 hex digest.
    pub grub: String,
    /// Expected `grub.cfg` SHA-256 hex digest.
    #[serde(rename = "grub.cfg")]
    pub grub_cfg: String,
    /// Allowed kernel/initramfs combinations.
    pub kernels: Vec<KernelBaseline>,
}

/// One allowed kernel/initramfs baseline entry.
#[derive(Debug, Deserialize)]
pub struct KernelBaseline {
    /// Optional human-readable kernel version.
    pub version: Option<String>,
    /// Optional kernel image SHA-256 hex digest.
    pub kernel: Option<String>,
    /// Optional initramfs SHA-256 hex digest.
    pub initramfs: Option<String>,
}

/// Load and validate a firmware baseline JSON file.
pub fn load_firmware_baseline(path: &str) -> Result<FirmwareBaseline> {
    let text = fs::read_to_string(path)?;
    let baseline: FirmwareBaseline = serde_json::from_str(&text).map_err(|error| {
        RaTlsError::InvalidData(format!("invalid firmware baseline JSON `{path}`: {error}"))
    })?;
    if baseline.hash_alg != "sha-256" {
        return Err(RaTlsError::InvalidData(format!(
            "firmware baseline `{path}` field `hash_alg` must be `sha-256`; got `{}`",
            baseline.hash_alg
        )));
    }
    validate_sha256_hex(path, "grub", &baseline.grub)?;
    validate_sha256_hex(path, "grub.cfg", &baseline.grub_cfg)?;
    for (index, kernel) in baseline.kernels.iter().enumerate() {
        if let Some(value) = &kernel.kernel {
            validate_sha256_hex(path, &format!("kernels[{index}].kernel"), value)?;
        }
        if let Some(value) = &kernel.initramfs {
            validate_sha256_hex(path, &format!("kernels[{index}].initramfs"), value)?;
        }
    }
    Ok(baseline)
}

/// Verify extracted firmware state against a firmware baseline file.
///
/// GRUB and `grub.cfg` must match exactly. Kernel and initramfs are accepted if
/// they match any entry in the `kernels` array.
pub fn verify_firmware_baseline(path: &str, state: &FirmwareState) -> Result<()> {
    let baseline = load_firmware_baseline(path)?;
    let grub = hex_decode(&baseline.grub)?;
    if !state.efi_images.contains(&grub) {
        return Err(RaTlsError::InvalidData(
            "CCA firmware baseline mismatch: grub".into(),
        ));
    }
    let grub_cfg = hex_decode(&baseline.grub_cfg)?;
    if state.grub_config.as_ref() != Some(&grub_cfg) {
        return Err(RaTlsError::InvalidData(
            "CCA firmware baseline mismatch: grub.cfg".into(),
        ));
    }

    let mut matched = false;
    for kernel in &baseline.kernels {
        let kernel_ok = match (&state.kernel, &kernel.kernel) {
            (Some(actual), Some(expected)) => actual == &hex_decode(expected)?,
            (None, _) => true,
            (_, None) => true,
        };
        let initrd_ok = match (&state.initramfs, &kernel.initramfs) {
            (Some(actual), Some(expected)) => actual == &hex_decode(expected)?,
            (None, _) => true,
            (_, None) => true,
        };
        if kernel_ok && initrd_ok {
            matched = true;
            break;
        }
    }
    if !matched {
        return Err(RaTlsError::InvalidData(
            "CCA firmware baseline mismatch: kernel/initramfs".into(),
        ));
    }
    Ok(())
}

fn validate_sha256_hex(file: &str, field: &str, value: &str) -> Result<()> {
    let digest = value.trim();
    let character_count = digest.chars().count();
    if character_count != 64 {
        return Err(RaTlsError::InvalidData(format!(
            "firmware baseline `{file}` field `{field}` must be a 64-character hexadecimal \
             SHA-256 digest; got {character_count} characters"
        )));
    }
    if let Some((position, character)) = digest
        .chars()
        .enumerate()
        .find(|(_, character)| !character.is_ascii_hexdigit())
    {
        return Err(RaTlsError::InvalidData(format!(
            "firmware baseline `{file}` field `{field}` contains invalid hexadecimal character \
             '{character}' at position {}",
            position + 1
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use ratls_api::hex_encode;

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn temporary_json(contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ratls-firmware-policy-{}-{}.json",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, contents).unwrap();
        path
    }

    fn baseline(grub: u8, grub_cfg: u8, kernel: u8, initramfs: u8) -> String {
        format!(
            r#"{{
                "hash_alg":"sha-256",
                "grub":"{}",
                "grub.cfg":"{}",
                "kernels":[{{
                    "version":"test",
                    "kernel":"{}",
                    "initramfs":"{}"
                }}]
            }}"#,
            hex_encode(&[grub; 32]),
            hex_encode(&[grub_cfg; 32]),
            hex_encode(&[kernel; 32]),
            hex_encode(&[initramfs; 32])
        )
    }

    #[test]
    fn loads_and_matches_a_complete_firmware_baseline() {
        let path = temporary_json(&baseline(1, 2, 3, 4));
        let loaded = load_firmware_baseline(path.to_str().unwrap()).unwrap();
        assert_eq!(loaded.kernels[0].version.as_deref(), Some("test"));

        let state = FirmwareState {
            efi_images: vec![vec![1; 32]],
            grub_config: Some(vec![2; 32]),
            kernel: Some(vec![3; 32]),
            initramfs: Some(vec![4; 32]),
        };
        assert!(verify_firmware_baseline(path.to_str().unwrap(), &state).is_ok());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_bad_algorithms_digests_and_component_mismatches() {
        let unsupported =
            temporary_json(&baseline(1, 2, 3, 4).replace(r#""sha-256""#, r#""sha-384""#));
        assert!(load_firmware_baseline(unsupported.to_str().unwrap()).is_err());
        fs::remove_file(unsupported).unwrap();

        let short = temporary_json(&baseline(1, 2, 3, 4).replace(&hex_encode(&[1; 32]), "00"));
        assert!(load_firmware_baseline(short.to_str().unwrap()).is_err());
        fs::remove_file(short).unwrap();

        let path = temporary_json(&baseline(1, 2, 3, 4));
        let base = FirmwareState {
            efi_images: vec![vec![1; 32]],
            grub_config: Some(vec![2; 32]),
            kernel: Some(vec![3; 32]),
            initramfs: Some(vec![4; 32]),
        };
        let mut bad = base.clone();
        bad.efi_images.clear();
        assert!(verify_firmware_baseline(path.to_str().unwrap(), &bad).is_err());
        let mut bad = base.clone();
        bad.grub_config = Some(vec![9; 32]);
        assert!(verify_firmware_baseline(path.to_str().unwrap(), &bad).is_err());
        let mut bad = base;
        bad.kernel = Some(vec![9; 32]);
        assert!(verify_firmware_baseline(path.to_str().unwrap(), &bad).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_the_file_and_json_location_for_malformed_input() {
        let path = temporary_json(
            r#"{
                "hash_alg":"sha-256",
                "grub":"00"
                "grub.cfg":"00",
                "kernels":[]
            }"#,
        );

        let error = load_firmware_baseline(path.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid firmware baseline JSON"), "{error}");
        assert!(error.contains(path.to_str().unwrap()), "{error}");
        assert!(error.contains("line 4 column"), "{error}");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_the_full_field_path_for_an_invalid_kernel_digest() {
        let invalid_digest = format!("{}g", "0".repeat(63));
        let contents = baseline(1, 2, 3, 4).replace(&hex_encode(&[3; 32]), &invalid_digest);
        let path = temporary_json(&contents);

        let error = load_firmware_baseline(path.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains(path.to_str().unwrap()), "{error}");
        assert!(error.contains("`kernels[0].kernel`"), "{error}");
        assert!(
            error.contains("invalid hexadecimal character 'g'"),
            "{error}"
        );
        fs::remove_file(path).unwrap();
    }
}
