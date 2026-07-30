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

//! Shared command-line options for local certificate issuance and peer TLS verification.

use std::path::PathBuf;

use ratls_api::api::{
    MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE, MAX_ISSUER_PRIVATE_KEY_PEM_SIZE,
    MAX_TRUSTED_CA_CHAIN_PEM_SIZE,
};
use ratls_api::core::{CertificateConf, TlsVerifyConf};
use ratls_api::{RaTlsError, Result};

use crate::common::cli::read_input_file;

/// Options controlling how this endpoint's dynamic RA-TLS certificate is signed.
#[derive(clap::Args, Debug, Default)]
pub struct CertificateOptions {
    /// Unencrypted PEM CA private key used to sign the dynamic leaf certificate.
    #[arg(long, value_name = "PEM_FILE", requires = "issuer_certificate_chain")]
    issuer_private_key: Option<PathBuf>,

    /// PEM issuer certificate chain matching --issuer-private-key.
    #[arg(long, value_name = "PEM_FILE", requires = "issuer_private_key")]
    issuer_certificate_chain: Option<PathBuf>,

    /// SAN added to the dynamic leaf certificate, for example DNS:server.example.com.
    #[arg(long = "subject-alt-name", alias = "san", value_name = "TYPE:VALUE")]
    subject_alt_names: Vec<String>,
}

impl CertificateOptions {
    /// Load PEM files and convert these CLI options to the API configuration.
    pub fn load(&self) -> Result<CertificateConf> {
        Ok(CertificateConf {
            issuer_private_key: read_optional(
                &self.issuer_private_key,
                "issuer private key",
                MAX_ISSUER_PRIVATE_KEY_PEM_SIZE,
            )?,
            issuer_certificate_chain: read_optional(
                &self.issuer_certificate_chain,
                "issuer certificate chain",
                MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE,
            )?,
            subject_alt_names: self.subject_alt_names.clone(),
        })
    }
}

/// Options controlling standard OpenSSL verification of the peer certificate.
#[derive(clap::Args, Debug, Default)]
pub struct TlsVerifyOptions {
    /// Enable standard TLS CA-chain verification in addition to RA verification.
    #[arg(long)]
    verify_peer_certificate: bool,

    /// Add OpenSSL's default system CA paths to the trust store.
    #[arg(long)]
    use_system_ca: bool,

    /// PEM CA bundle trusted for peer certificate verification.
    #[arg(long, value_name = "PEM_FILE")]
    trusted_ca_chain: Option<PathBuf>,

    /// Optional peer DNS name or IP address checked against the certificate.
    #[arg(long, value_name = "DNS_OR_IP")]
    expected_peer_name: Option<String>,
}

impl TlsVerifyOptions {
    /// Validate relationships between the standard TLS verification flags.
    pub fn validate_cli(&self, server: bool, mutual: bool) -> Result<()> {
        if !self.verify_peer_certificate {
            if self.use_system_ca
                || self.trusted_ca_chain.is_some()
                || self.expected_peer_name.is_some()
            {
                return Err(RaTlsError::InvalidArgument(
                    "--use-system-ca, --trusted-ca-chain, and --expected-peer-name require --verify-peer-certificate"
                        .into(),
                ));
            }
            return Ok(());
        }
        if server && !mutual {
            return Err(RaTlsError::InvalidArgument(
                "server --verify-peer-certificate requires --mutual".into(),
            ));
        }
        if !self.use_system_ca && self.trusted_ca_chain.is_none() {
            return Err(RaTlsError::InvalidArgument(
                "--verify-peer-certificate requires --use-system-ca or --trusted-ca-chain".into(),
            ));
        }
        if self.expected_peer_name.as_deref() == Some("") {
            return Err(RaTlsError::InvalidArgument(
                "--expected-peer-name must not be empty".into(),
            ));
        }
        Ok(())
    }

    /// Load the trust bundle and convert these CLI options to the API configuration.
    pub fn load(&self) -> Result<TlsVerifyConf> {
        if !self.verify_peer_certificate {
            return Ok(TlsVerifyConf::default());
        }
        Ok(TlsVerifyConf {
            verify_peer_certificate: true,
            use_system_ca: self.use_system_ca,
            trusted_ca_chain: read_optional(
                &self.trusted_ca_chain,
                "trusted CA chain",
                MAX_TRUSTED_CA_CHAIN_PEM_SIZE,
            )?,
            expected_peer_name: self.expected_peer_name.clone(),
        })
    }
}

fn read_optional(path: &Option<PathBuf>, name: &str, maximum: usize) -> Result<Vec<u8>> {
    path.as_deref()
        .map(|path| read_input_file(path, name, maximum))
        .transpose()
        .map(Option::unwrap_or_default)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn pem_file(name: &str, contents: &[u8]) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "ratls-tls-config-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn loads_certificate_and_trust_files_into_api_configuration() {
        let key = pem_file("issuer.key", b"private");
        let chain = pem_file("issuer.pem", b"certificate");
        let options = CertificateOptions {
            issuer_private_key: Some(key.clone()),
            issuer_certificate_chain: Some(chain.clone()),
            subject_alt_names: vec!["DNS:test.example".into()],
        };
        let loaded = options.load().unwrap();
        assert_eq!(loaded.issuer_private_key, b"private");
        assert_eq!(loaded.issuer_certificate_chain, b"certificate");
        assert_eq!(loaded.subject_alt_names, ["DNS:test.example"]);

        let verify = TlsVerifyOptions {
            verify_peer_certificate: true,
            use_system_ca: false,
            trusted_ca_chain: Some(chain.clone()),
            expected_peer_name: Some("test.example".into()),
        };
        assert!(verify.validate_cli(false, false).is_ok());
        let loaded = verify.load().unwrap();
        assert!(loaded.verify_peer_certificate);
        assert_eq!(loaded.trusted_ca_chain, b"certificate");
        assert_eq!(loaded.expected_peer_name.as_deref(), Some("test.example"));

        fs::remove_dir_all(key.parent().unwrap()).unwrap();
        fs::remove_dir_all(chain.parent().unwrap()).unwrap();
    }

    #[test]
    fn validates_tls_flag_relationships_and_disabled_defaults() {
        let disabled = TlsVerifyOptions::default();
        assert!(disabled.validate_cli(false, false).is_ok());
        assert!(!disabled.load().unwrap().verify_peer_certificate);

        let stray = TlsVerifyOptions {
            use_system_ca: true,
            ..Default::default()
        };
        assert!(stray.validate_cli(false, false).is_err());

        let no_trust = TlsVerifyOptions {
            verify_peer_certificate: true,
            ..Default::default()
        };
        assert!(no_trust.validate_cli(false, false).is_err());

        let server = TlsVerifyOptions {
            verify_peer_certificate: true,
            use_system_ca: true,
            ..Default::default()
        };
        assert!(server.validate_cli(true, false).is_err());
        assert!(server.validate_cli(true, true).is_ok());

        let empty_name = TlsVerifyOptions {
            expected_peer_name: Some(String::new()),
            ..server
        };
        assert!(empty_name.validate_cli(false, false).is_err());
    }
}
