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

use std::fs;
use std::path::{Path, PathBuf};

use crate::attesters::Attester;
use crate::core::evidence::AttestationEvidence;
use crate::verifiers::cca::evidence::CcaEvidence;
use crate::verifiers::cca::EVIDENCE_TAG;
use crate::RaTlsError;

const DEFAULT_TSM_REPORT_DIR: &str = "/sys/kernel/config/tsm/report/report0";

/// CCA attester backed by the Linux TSM report sysfs interface.
#[derive(Debug, Clone)]
pub struct CcaAttester {
    /// Directory containing `inblob`, `outblob`, and `auxblob`.
    pub report_dir: PathBuf,
}

impl Default for CcaAttester {
    fn default() -> Self {
        Self {
            report_dir: PathBuf::from(DEFAULT_TSM_REPORT_DIR),
        }
    }
}

impl Attester for CcaAttester {
    fn name(&self) -> &'static str {
        "cca"
    }

    fn evidence_tag(&self) -> u64 {
        EVIDENCE_TAG
    }

    fn collect_evidence(&self, challenge: &[u8]) -> Result<AttestationEvidence, RaTlsError> {
        if !self.report_dir.exists() {
            crate::rtls_err!(
                "CCA TSM report directory does not exist: {}",
                self.report_dir.display()
            );
            return Err(RaTlsError::InvalidData(format!(
                "CCA TSM report directory does not exist: {}",
                self.report_dir.display()
            )));
        }
        crate::rtls_debug!(
            "CCA TSM report directory found: {}",
            self.report_dir.display()
        );
        crate::rtls_debug!(
            "collecting CCA evidence with {} bytes report data",
            challenge.len()
        );
        let normalized_challenge: [u8; 32] = match challenge.try_into() {
            Ok(challenge) => challenge,
            Err(_) => openssl::sha::sha256(challenge),
        };
        write_file(self.report_dir.join("inblob"), &normalized_challenge)?;
        let token = read_file(self.report_dir.join("outblob"))?;
        let dev_cert = read_file(self.report_dir.join("auxblob"))?;
        if token.is_empty() {
            return Err(RaTlsError::InvalidData("CCA TSM outblob is empty".into()));
        }
        if dev_cert.is_empty() {
            return Err(RaTlsError::InvalidData(
                "CCA TSM auxblob/dev_cert is empty".into(),
            ));
        }
        crate::rtls_debug!(
            "collected CCA evidence: token={} bytes, dev_cert={} bytes",
            token.len(),
            dev_cert.len()
        );
        Ok(AttestationEvidence {
            raw: CcaEvidence { token, dev_cert }.encode_raw_native(),
        })
    }
}

fn write_file(path: impl AsRef<Path>, data: &[u8]) -> Result<(), RaTlsError> {
    let path = path.as_ref();
    fs::write(path, data).map_err(|err| {
        RaTlsError::Io(std::io::Error::new(
            err.kind(),
            format!("failed to write {}: {err}", path.display()),
        ))
    })
}

fn read_file(path: impl AsRef<Path>) -> Result<Vec<u8>, RaTlsError> {
    let path = path.as_ref();
    fs::read(path).map_err(|err| {
        RaTlsError::Io(std::io::Error::new(
            err.kind(),
            format!("failed to read {}: {err}", path.display()),
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn report_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ratls-cca-attester-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn collects_native_evidence_from_a_mock_tsm_directory() {
        let report_dir = report_directory();
        fs::write(report_dir.join("outblob"), [1, 2, 3]).unwrap();
        fs::write(report_dir.join("auxblob"), [4, 5]).unwrap();
        let attester = CcaAttester {
            report_dir: report_dir.clone(),
        };

        assert_eq!(attester.name(), "cca");
        assert_eq!(attester.evidence_tag(), EVIDENCE_TAG);
        let evidence = attester.collect_evidence(&[9; 32]).unwrap();
        let decoded = CcaEvidence::decode_raw_native(&evidence.raw).unwrap();
        assert_eq!(decoded.token, [1, 2, 3]);
        assert_eq!(decoded.dev_cert, [4, 5]);
        assert_eq!(fs::read(report_dir.join("inblob")).unwrap(), [9; 32]);

        attester.collect_evidence(b"short challenge").unwrap();
        assert_eq!(
            fs::read(report_dir.join("inblob")).unwrap(),
            openssl::sha::sha256(b"short challenge")
        );
        fs::remove_dir_all(report_dir).unwrap();
    }

    #[test]
    fn rejects_missing_and_empty_tsm_outputs() {
        let missing = CcaAttester {
            report_dir: std::env::temp_dir().join("ratls-definitely-missing-tsm-report"),
        };
        assert!(missing.collect_evidence(&[0; 32]).is_err());

        let report_dir = report_directory();
        fs::write(report_dir.join("outblob"), []).unwrap();
        fs::write(report_dir.join("auxblob"), [1]).unwrap();
        let attester = CcaAttester {
            report_dir: report_dir.clone(),
        };
        assert!(attester.collect_evidence(&[0; 32]).is_err());

        fs::write(report_dir.join("outblob"), [1]).unwrap();
        fs::write(report_dir.join("auxblob"), []).unwrap();
        assert!(attester.collect_evidence(&[0; 32]).is_err());
        fs::remove_dir_all(report_dir).unwrap();
    }
}
