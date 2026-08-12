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

//! Binary IMA log parsing and digest baseline verification.
//!
//! This sample does not replay IMA measurements into a CCA REM because the
//! current CCA evidence path does not expose a comparable IMA REM value. It
//! instead validates each parsed IMA file identity against a digest baseline.

use std::collections::HashSet;
use std::fs;

use openssl::hash::{hash, MessageDigest};

use ratls_api::{hex_decode, RaTlsError, Result};

/// Maximum number of IMA log entries accepted by the sample.
pub const MAX_IMA_ENTRIES: usize = 100_000;
/// Maximum number of baseline entries accepted by the sample.
pub const MAX_IMA_BASELINES: usize = 100_000;
const MAX_TEMPLATE_NAME: usize = 255;
const MAX_TEMPLATE_DATA: usize = 16 * 1024 * 1024;
const MAX_IMA_PATH: usize = 4096;

/// Stable identity of one measured file in an IMA log.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImaIdentity {
    /// Digest algorithm used for the file measurement.
    pub algorithm: ImaAlgorithm,
    /// File digest bytes.
    pub digest: Vec<u8>,
    /// File path recorded by the IMA template.
    pub path: String,
}

/// IMA digest algorithms supported by the sample parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImaAlgorithm {
    /// SHA-1 file digest.
    Sha1,
    /// SHA-256 file digest.
    Sha256,
}

/// Parsed IMA log entry.
#[derive(Debug, Clone)]
pub struct ImaEntry {
    /// PCR index reported by the IMA log record.
    pub pcr: u32,
    /// IMA template name, currently `ima-ng`, `ima-sig`, or `ima-buf`.
    pub template_name: String,
    /// File identity extracted from the template payload.
    pub identity: ImaIdentity,
}

/// Set-based digest baseline used for IMA entry checking.
#[derive(Debug, Clone)]
pub struct ImaDigestBaseline {
    /// Allowed measured file identities.
    pub entries: HashSet<ImaIdentity>,
}

impl ImaDigestBaseline {
    /// Load a whitespace-separated IMA digest baseline file.
    ///
    /// Each non-empty, non-comment line must contain:
    ///
    /// ```text
    /// <algorithm> <hex_digest> <path>
    /// ```
    ///
    /// Supported algorithms are `sha1` and `sha256`.
    pub fn load(path: &str) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut entries = HashSet::new();
        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if entries.len() >= MAX_IMA_BASELINES {
                return Err(RaTlsError::InvalidData("too many IMA baselines".into()));
            }
            let mut parts = line
                .splitn(3, char::is_whitespace)
                .filter(|s| !s.is_empty());
            let alg = parse_algorithm(parts.next().ok_or_else(|| {
                RaTlsError::InvalidData(format!("bad IMA baseline line {}", line_no + 1))
            })?)?;
            let digest_text = parts.next().ok_or_else(|| {
                RaTlsError::InvalidData(format!("bad IMA baseline line {}", line_no + 1))
            })?;
            let path = parts.next().ok_or_else(|| {
                RaTlsError::InvalidData(format!("bad IMA baseline line {}", line_no + 1))
            })?;
            let digest = hex_decode(digest_text)?;
            if digest.len() != alg.digest_size() || path.is_empty() || path.len() > MAX_IMA_PATH {
                return Err(RaTlsError::InvalidData(format!(
                    "invalid IMA baseline at line {}",
                    line_no + 1
                )));
            }
            entries.insert(ImaIdentity {
                algorithm: alg,
                digest,
                path: path.to_string(),
            });
        }
        if entries.is_empty() {
            return Err(RaTlsError::InvalidData("IMA baseline is empty".into()));
        }
        Ok(Self { entries })
    }

    /// Verify that every parsed IMA entry is present in the baseline.
    ///
    /// The `boot_aggregate` pseudo-entry is skipped, matching the sample policy
    /// behavior used for CCA application-side verification.
    pub fn verify_entries(&self, entries: &[ImaEntry]) -> Result<()> {
        for (idx, entry) in entries.iter().enumerate() {
            if entry.identity.path == "boot_aggregate" {
                continue;
            }
            if !self.entries.contains(&entry.identity) {
                return Err(RaTlsError::InvalidData(format!(
                    "IMA baseline mismatch at entry {idx}: {}",
                    entry.identity.path
                )));
            }
        }
        Ok(())
    }
}

/// Parse a binary IMA log using the kernel template format.
///
/// Supported templates are `ima-ng`, `ima-sig`, and `ima-buf`. The template
/// payload hash is checked against the entry header hash before extracting the
/// measured file identity.
pub fn parse_binary_ima_log(data: &[u8]) -> Result<Vec<ImaEntry>> {
    let mut cursor = Cursor::new(data);
    let mut entries = Vec::new();
    while cursor.remaining() > 0 {
        if entries.len() >= MAX_IMA_ENTRIES {
            return Err(RaTlsError::InvalidData(
                "IMA log exceeds 100000 entries".into(),
            ));
        }
        let entry_offset = cursor.offset;
        let pcr = cursor.u32()?;
        let header_digest = cursor.bytes(20)?;
        let name_size = cursor.u32()? as usize;
        if name_size == 0 || name_size > MAX_TEMPLATE_NAME {
            return Err(RaTlsError::InvalidData(format!(
                "invalid IMA template name size at offset {entry_offset}"
            )));
        }
        let name = cursor.bytes(name_size)?;
        let template_size = cursor.u32()? as usize;
        if template_size > MAX_TEMPLATE_DATA {
            return Err(RaTlsError::InvalidData(format!(
                "invalid IMA template data size at offset {entry_offset}"
            )));
        }
        let template_data = cursor.bytes(template_size)?;
        let template_name = trim_nul_string(name)?;
        if !matches!(template_name.as_str(), "ima-ng" | "ima-sig" | "ima-buf") {
            return Err(RaTlsError::InvalidData(format!(
                "unsupported IMA template {template_name}"
            )));
        }
        let calculated = hash(MessageDigest::sha1(), template_data)?.to_vec();
        if calculated != header_digest {
            return Err(RaTlsError::InvalidData(format!(
                "IMA template hash mismatch at entry {}",
                entries.len()
            )));
        }
        let identity = parse_ima_identity(template_data)?;
        entries.push(ImaEntry {
            pcr,
            template_name,
            identity,
        });
    }
    if entries.is_empty() {
        return Err(RaTlsError::InvalidData("IMA log is empty".into()));
    }
    Ok(entries)
}

fn parse_ima_identity(template_data: &[u8]) -> Result<ImaIdentity> {
    let mut cursor = Cursor::new(template_data);
    let digest_field_size = cursor.u32()? as usize;
    let digest_field = cursor.bytes(digest_field_size)?;
    let Some(separator) = digest_field.iter().position(|b| *b == b':') else {
        return Err(RaTlsError::InvalidData(
            "IMA digest field has no algorithm separator".into(),
        ));
    };
    let algorithm = std::str::from_utf8(&digest_field[..separator])
        .map_err(|_| RaTlsError::InvalidData("IMA digest algorithm is not UTF-8".into()))?;
    let algorithm = parse_algorithm(algorithm)?;
    let mut digest = &digest_field[separator + 1..];
    if digest.first() == Some(&0) {
        digest = &digest[1..];
    }
    if digest.len() != algorithm.digest_size() {
        return Err(RaTlsError::InvalidData(
            "IMA file digest length mismatch".into(),
        ));
    }

    let path_size = cursor.u32()? as usize;
    if path_size == 0 || path_size > MAX_IMA_PATH + 1 {
        return Err(RaTlsError::InvalidData("IMA path size is invalid".into()));
    }
    let path = cursor.bytes(path_size)?;
    let path = trim_nul_string(path)?;
    if path.is_empty() || path.len() > MAX_IMA_PATH {
        return Err(RaTlsError::InvalidData("IMA path is invalid".into()));
    }
    Ok(ImaIdentity {
        algorithm,
        digest: digest.to_vec(),
        path,
    })
}

fn parse_algorithm(text: &str) -> Result<ImaAlgorithm> {
    match text.to_ascii_lowercase().as_str() {
        "sha1" => Ok(ImaAlgorithm::Sha1),
        "sha256" => Ok(ImaAlgorithm::Sha256),
        _ => Err(RaTlsError::InvalidData(format!(
            "unsupported IMA algorithm {text}"
        ))),
    }
}

impl ImaAlgorithm {
    fn digest_size(self) -> usize {
        match self {
            ImaAlgorithm::Sha1 => 20,
            ImaAlgorithm::Sha256 => 32,
        }
    }
}

fn trim_nul_string(data: &[u8]) -> Result<String> {
    let end = data
        .iter()
        .rposition(|b| *b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    std::str::from_utf8(&data[..end])
        .map(str::to_string)
        .map_err(|_| RaTlsError::InvalidData("IMA string is not UTF-8".into()))
}

struct Cursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| RaTlsError::InvalidData("IMA cursor overflow".into()))?;
        if end > self.data.len() {
            return Err(RaTlsError::InvalidData("truncated IMA log".into()));
        }
        let out = &self.data[self.offset..end];
        self.offset = end;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use ratls_api::hex_encode;

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn temporary_baseline(contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ratls-ima-baseline-{}-{}.txt",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, contents).unwrap();
        path
    }

    fn ima_entry(template_name: &[u8], algorithm: &str, digest: &[u8], path: &str) -> Vec<u8> {
        let mut digest_field = format!("{algorithm}:").into_bytes();
        digest_field.push(0);
        digest_field.extend_from_slice(digest);
        let mut template = Vec::new();
        template.extend_from_slice(&(digest_field.len() as u32).to_le_bytes());
        template.extend_from_slice(&digest_field);
        template.extend_from_slice(&((path.len() + 1) as u32).to_le_bytes());
        template.extend_from_slice(path.as_bytes());
        template.push(0);

        let mut entry = 10u32.to_le_bytes().to_vec();
        entry.extend_from_slice(hash(MessageDigest::sha1(), &template).unwrap().as_ref());
        entry.extend_from_slice(&(template_name.len() as u32).to_le_bytes());
        entry.extend_from_slice(template_name);
        entry.extend_from_slice(&(template.len() as u32).to_le_bytes());
        entry.extend_from_slice(&template);
        entry
    }

    #[test]
    fn parses_supported_templates_and_verifies_a_baseline() {
        let digest = [0x5a; 32];
        let data = ima_entry(b"ima-ng\0", "sha256", &digest, "/usr/bin/test");
        let entries = parse_binary_ima_log(&data).unwrap();
        assert_eq!(entries[0].pcr, 10);
        assert_eq!(entries[0].template_name, "ima-ng");
        assert_eq!(entries[0].identity.algorithm, ImaAlgorithm::Sha256);

        let path = temporary_baseline(&format!(
            "# known good\nsha256 {} /usr/bin/test\n",
            hex_encode(&digest)
        ));
        let baseline = ImaDigestBaseline::load(path.to_str().unwrap()).unwrap();
        assert!(baseline.verify_entries(&entries).is_ok());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn baseline_rejects_invalid_input_and_unknown_measurements() {
        for contents in [
            "",
            "sha512 00 /bad",
            "sha256 00 /bad",
            "sha1 0000000000000000000000000000000000000000",
        ] {
            let path = temporary_baseline(contents);
            assert!(ImaDigestBaseline::load(path.to_str().unwrap()).is_err());
            fs::remove_file(path).unwrap();
        }

        let path = temporary_baseline(&format!("sha256 {} /allowed\n", hex_encode(&[1; 32])));
        let baseline = ImaDigestBaseline::load(path.to_str().unwrap()).unwrap();
        let entries = vec![
            ImaEntry {
                pcr: 10,
                template_name: "ima-ng".into(),
                identity: ImaIdentity {
                    algorithm: ImaAlgorithm::Sha256,
                    digest: vec![2; 32],
                    path: "boot_aggregate".into(),
                },
            },
            ImaEntry {
                pcr: 10,
                template_name: "ima-ng".into(),
                identity: ImaIdentity {
                    algorithm: ImaAlgorithm::Sha256,
                    digest: vec![2; 32],
                    path: "/unknown".into(),
                },
            },
        ];
        assert!(baseline.verify_entries(&entries).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn parser_rejects_empty_truncated_tampered_and_unsupported_logs() {
        assert!(parse_binary_ima_log(&[]).is_err());
        assert!(parse_binary_ima_log(&[0; 3]).is_err());

        let mut tampered = ima_entry(b"ima-ng\0", "sha1", &[3; 20], "/test");
        tampered[4] ^= 1;
        assert!(parse_binary_ima_log(&tampered).is_err());

        let unsupported = ima_entry(b"ima\0", "sha1", &[3; 20], "/test");
        assert!(parse_binary_ima_log(&unsupported).is_err());

        let bad_algorithm = ima_entry(b"ima-ng\0", "sha512", &[3; 64], "/test");
        assert!(parse_binary_ima_log(&bad_algorithm).is_err());
    }
}
