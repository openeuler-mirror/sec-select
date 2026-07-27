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

use crate::attesters::AttesterRegistry;
use crate::core::{RaTlsConf, RaTlsHandle, VerificationCallback};
use crate::crypto_wrappers::CryptoWrapperRegistry;
use crate::tls_wrappers::{TlsWrapperRegistry, TransportStream};
use crate::verifiers::VerifierRegistry;
use crate::RaTlsError;
use std::collections::HashSet;
use std::net::IpAddr;

/// Maximum accepted unencrypted issuer private-key PEM size.
pub const MAX_ISSUER_PRIVATE_KEY_PEM_SIZE: usize = 64 * 1024;
/// Maximum accepted issuer certificate-chain PEM bundle size.
pub const MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE: usize = 1024 * 1024;
/// Maximum accepted custom trusted-CA PEM bundle size.
pub const MAX_TRUSTED_CA_CHAIN_PEM_SIZE: usize = 4 * 1024 * 1024;
/// Maximum number of SAN entries generated into one dynamic leaf certificate.
pub const MAX_SUBJECT_ALT_NAMES: usize = 64;
/// Maximum UTF-8 byte length of one prefixed SAN entry.
pub const MAX_SUBJECT_ALT_NAME_LENGTH: usize = 2048;
/// Maximum byte length of a peer DNS name or textual IP address.
pub const MAX_EXPECTED_PEER_NAME_LENGTH: usize = 253;
/// Maximum number of application-defined custom claims.
pub const MAX_CUSTOM_CLAIMS: usize = 64;
/// Maximum UTF-8 byte length of one custom claim name.
pub const MAX_CUSTOM_CLAIM_NAME_LENGTH: usize = 128;
/// Maximum byte length of one custom claim value.
pub const MAX_CUSTOM_CLAIM_VALUE_LENGTH: usize = 64 * 1024;
/// Maximum combined byte length of all custom claim values.
pub const MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH: usize = 256 * 1024;

pub fn rats_tls_init(conf: RaTlsConf) -> Result<RaTlsHandle, RaTlsError> {
    crate::rtls_debug!(
        "initializing RATS-TLS: tls='{}' crypto='{}' attester={:?} verifier={:?} server={} mutual={}",
        &conf.tls_type,
        &conf.crypto_type,
        &conf.attester,
        &conf.verifier,
        &conf.server,
        &conf.mutual
    );
    validate_configuration(&conf)?;
    let crypto = CryptoWrapperRegistry::load(conf.crypto_type)?;
    let certificate_issuer = match (
        conf.certificate.issuer_private_key.is_empty(),
        conf.certificate.issuer_certificate_chain.is_empty(),
    ) {
        (true, true) => None,
        (false, false) => Some(crypto.prepare_certificate_issuer(
            &conf.certificate.issuer_private_key,
            &conf.certificate.issuer_certificate_chain,
        )?),
        _ => {
            return Err(RaTlsError::InvalidArgument(
                "issuer_private_key and issuer_certificate_chain must either both be configured or both be empty"
                    .into(),
            ))
        }
    };
    let tls = TlsWrapperRegistry::load(conf.tls_type)?;
    tls.validate_peer_verification(&conf.tls_verify)?;
    let verifier = conf.verifier.map(VerifierRegistry::load).transpose()?;
    let attester = conf.attester.map(AttesterRegistry::load).transpose()?;
    Ok(RaTlsHandle {
        conf,
        attester,
        verifier,
        crypto,
        tls,
        certificate_issuer,
        user_callback: None,
    })
}

fn validate_configuration(conf: &RaTlsConf) -> Result<(), RaTlsError> {
    validate_leaf_algorithm(conf)?;
    validate_custom_claims(conf)?;
    validate_certificate_configuration(conf)?;
    validate_tls_verification_configuration(conf)?;

    if conf.server && conf.attester.is_none() {
        return Err(RaTlsError::InvalidArgument(
            "server configuration requires an attester".into(),
        ));
    }
    if !conf.server && conf.verifier.is_none() {
        return Err(RaTlsError::InvalidArgument(
            "client configuration requires a verifier".into(),
        ));
    }
    if conf.mutual && conf.attester.is_none() {
        return Err(RaTlsError::InvalidArgument(
            "mutual TLS configuration requires an attester".into(),
        ));
    }
    if conf.mutual && conf.verifier.is_none() {
        return Err(RaTlsError::InvalidArgument(
            "mutual TLS configuration requires a verifier".into(),
        ));
    }
    if conf.server && conf.tls_verify.verify_peer_certificate && !conf.mutual {
        return Err(RaTlsError::InvalidArgument(
            "a server can verify a peer TLS certificate only when mutual TLS is enabled".into(),
        ));
    }
    Ok(())
}

fn validate_leaf_algorithm(conf: &RaTlsConf) -> Result<(), RaTlsError> {
    if matches!(
        conf.cert_algo,
        crate::crypto_wrappers::CertAlgorithm::Rsa3072Sha256
            | crate::crypto_wrappers::CertAlgorithm::Ecc256Sha256
    ) {
        return Ok(());
    }
    Err(RaTlsError::InvalidArgument(format!(
        "dynamic leaf certificate algorithm {:?} is not supported",
        conf.cert_algo
    )))
}

fn validate_custom_claims(conf: &RaTlsConf) -> Result<(), RaTlsError> {
    if conf.custom_claims.len() > MAX_CUSTOM_CLAIMS {
        return Err(RaTlsError::InvalidArgument(format!(
            "too many custom claims: maximum is {MAX_CUSTOM_CLAIMS}"
        )));
    }

    let mut names = HashSet::with_capacity(conf.custom_claims.len());
    let mut total_value_length = 0usize;
    for claim in &conf.custom_claims {
        if claim.name.is_empty() {
            return Err(RaTlsError::InvalidArgument(
                "custom claim name must not be empty".into(),
            ));
        }
        if claim.name.len() > MAX_CUSTOM_CLAIM_NAME_LENGTH {
            return Err(RaTlsError::InvalidArgument(format!(
                "custom claim name exceeds {MAX_CUSTOM_CLAIM_NAME_LENGTH} bytes"
            )));
        }
        if claim.name.chars().any(char::is_control) {
            return Err(RaTlsError::InvalidArgument(format!(
                "custom claim name {:?} contains a control character",
                claim.name
            )));
        }
        if matches!(
            claim.name.as_str(),
            "pubkey-hash" | "client-key-share-hash" | "nonce"
        ) {
            return Err(RaTlsError::InvalidArgument(format!(
                "reserved custom claim name '{}'",
                claim.name
            )));
        }
        if !names.insert(claim.name.as_str()) {
            return Err(RaTlsError::InvalidArgument(format!(
                "duplicate custom claim name '{}'",
                claim.name
            )));
        }
        if claim.value.len() > MAX_CUSTOM_CLAIM_VALUE_LENGTH {
            return Err(RaTlsError::InvalidArgument(format!(
                "custom claim '{}' value exceeds {MAX_CUSTOM_CLAIM_VALUE_LENGTH} bytes",
                claim.name
            )));
        }
        total_value_length = total_value_length
            .checked_add(claim.value.len())
            .ok_or_else(|| {
                RaTlsError::InvalidArgument("total custom claim data length overflows".into())
            })?;
        if total_value_length > MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH {
            return Err(RaTlsError::InvalidArgument(format!(
                "total custom claim data exceeds {MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH} bytes"
            )));
        }
    }
    Ok(())
}

fn validate_certificate_configuration(conf: &RaTlsConf) -> Result<(), RaTlsError> {
    let certificate = &conf.certificate;
    match (
        certificate.issuer_private_key.is_empty(),
        certificate.issuer_certificate_chain.is_empty(),
    ) {
        (true, true) | (false, false) => {}
        _ => {
            return Err(RaTlsError::InvalidArgument(
                "issuer_private_key and issuer_certificate_chain must either both be configured or both be empty"
                    .into(),
            ))
        }
    }
    check_size(
        "issuer private key",
        certificate.issuer_private_key.len(),
        MAX_ISSUER_PRIVATE_KEY_PEM_SIZE,
    )?;
    check_size(
        "issuer certificate chain",
        certificate.issuer_certificate_chain.len(),
        MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE,
    )?;
    if certificate.subject_alt_names.len() > MAX_SUBJECT_ALT_NAMES {
        return Err(RaTlsError::InvalidArgument(format!(
            "too many subject alternative names: maximum is {MAX_SUBJECT_ALT_NAMES}"
        )));
    }
    for entry in &certificate.subject_alt_names {
        validate_subject_alt_name(entry)?;
    }
    Ok(())
}

fn validate_tls_verification_configuration(conf: &RaTlsConf) -> Result<(), RaTlsError> {
    let verify = &conf.tls_verify;
    if !verify.verify_peer_certificate {
        return Ok(());
    }
    check_size(
        "trusted CA chain",
        verify.trusted_ca_chain.len(),
        MAX_TRUSTED_CA_CHAIN_PEM_SIZE,
    )?;
    if !verify.use_system_ca && verify.trusted_ca_chain.is_empty() {
        return Err(RaTlsError::InvalidArgument(
            "TLS peer verification requires the system CA store or a trusted CA chain".into(),
        ));
    }
    if let Some(name) = verify
        .expected_peer_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        if name.len() > MAX_EXPECTED_PEER_NAME_LENGTH {
            return Err(RaTlsError::InvalidArgument(format!(
                "expected peer name exceeds {MAX_EXPECTED_PEER_NAME_LENGTH} bytes"
            )));
        }
        if name.parse::<IpAddr>().is_err() {
            validate_dns_name(name, false, "expected peer DNS name")?;
        }
    }
    Ok(())
}

fn validate_subject_alt_name(entry: &str) -> Result<(), RaTlsError> {
    if entry.len() > MAX_SUBJECT_ALT_NAME_LENGTH {
        return Err(RaTlsError::InvalidArgument(format!(
            "subject alternative name exceeds {MAX_SUBJECT_ALT_NAME_LENGTH} bytes"
        )));
    }
    let (kind, value) = entry.split_once(':').ok_or_else(|| {
        RaTlsError::InvalidArgument(format!(
            "subject alternative name '{entry}' must use DNS:, IP:, or URI:"
        ))
    })?;
    if value.is_empty() {
        return Err(RaTlsError::InvalidArgument(
            "subject alternative name value is empty".into(),
        ));
    }
    match kind.to_ascii_uppercase().as_str() {
        "DNS" => validate_dns_name(value, true, "DNS subject alternative name"),
        "IP" => value.parse::<IpAddr>().map(|_| ()).map_err(|_| {
            RaTlsError::InvalidArgument(format!(
                "subject alternative name has an invalid IP address: {value}"
            ))
        }),
        "URI" => validate_uri(value),
        _ => Err(RaTlsError::InvalidArgument(format!(
            "unsupported subject alternative name kind '{kind}'"
        ))),
    }
}

fn validate_dns_name(name: &str, allow_wildcard: bool, context: &str) -> Result<(), RaTlsError> {
    if name.is_empty() || name.len() > MAX_EXPECTED_PEER_NAME_LENGTH || !name.is_ascii() {
        return Err(RaTlsError::InvalidArgument(format!(
            "invalid {context}: {name:?}"
        )));
    }
    let name = if let Some(suffix) = name.strip_prefix("*.") {
        if !allow_wildcard {
            return Err(RaTlsError::InvalidArgument(format!(
                "invalid {context}: wildcards are not allowed"
            )));
        }
        suffix
    } else {
        name
    };
    if name.contains('*') {
        return Err(RaTlsError::InvalidArgument(format!(
            "invalid {context}: wildcard must be the complete left-most label"
        )));
    }
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty()
        || name.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                || !label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                || !label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
    {
        return Err(RaTlsError::InvalidArgument(format!(
            "invalid {context}: {name:?}"
        )));
    }
    Ok(())
}

fn validate_uri(uri: &str) -> Result<(), RaTlsError> {
    if !uri.is_ascii()
        || uri
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(RaTlsError::InvalidArgument(format!(
            "invalid URI subject alternative name: {uri:?}"
        )));
    }
    let (scheme, remainder) = uri.split_once(':').ok_or_else(|| {
        RaTlsError::InvalidArgument(format!("invalid URI subject alternative name: {uri:?}"))
    })?;
    let mut bytes = scheme.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        || remainder.is_empty()
    {
        return Err(RaTlsError::InvalidArgument(format!(
            "invalid URI subject alternative name: {uri:?}"
        )));
    }
    Ok(())
}

fn check_size(name: &str, actual: usize, maximum: usize) -> Result<(), RaTlsError> {
    if actual > maximum {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} exceeds {maximum} bytes"
        )));
    }
    Ok(())
}

pub fn rats_tls_set_verification_callback(
    handle: &mut RaTlsHandle,
    callback: Option<VerificationCallback>,
) {
    handle.user_callback = callback;
}

pub fn rats_tls_negotiate(
    handle: &mut RaTlsHandle,
    stream: Box<dyn TransportStream>,
) -> Result<Box<dyn TransportStream>, RaTlsError> {
    crate::rtls_debug!(
        "starting TLS negotiation: server={} mutual={}",
        handle.conf.server,
        handle.conf.mutual
    );
    if handle.conf.server {
        return crate::core::engine::create_ratls_server(handle, stream);
    }
    crate::core::engine::create_ratls_client(handle, stream)
}

/// Write application bytes through the negotiated TLS stream.
pub fn rats_tls_transmit(
    stream: &mut dyn TransportStream,
    buf: &[u8],
) -> Result<usize, RaTlsError> {
    stream.write(buf).map_err(RaTlsError::from)
}

/// Read application bytes from the negotiated TLS stream.
pub fn rats_tls_receive(
    stream: &mut dyn TransportStream,
    buf: &mut [u8],
) -> Result<usize, RaTlsError> {
    stream.read(buf).map_err(RaTlsError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dice::CustomClaim;
    use crate::core::{CertificateConf, TlsVerifyConf};
    use crate::crypto_wrappers::CertAlgorithm;

    fn assert_init_error_contains(conf: RaTlsConf, expected: &str) {
        let error = rats_tls_init(conf).err().expect("configuration must fail");
        assert!(
            error.to_string().contains(expected),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn server_tls_peer_verification_requires_mutual_tls() {
        let conf = RaTlsConf {
            server: true,
            tls_verify: TlsVerifyConf {
                verify_peer_certificate: true,
                use_system_ca: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let error = rats_tls_init(conf).err().unwrap();
        assert!(error
            .to_string()
            .contains("only when mutual TLS is enabled"));
    }

    #[test]
    fn enabled_tls_verification_requires_at_least_one_trust_source() {
        let conf = RaTlsConf {
            tls_verify: TlsVerifyConf {
                verify_peer_certificate: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let error = rats_tls_init(conf).err().unwrap();
        assert!(error
            .to_string()
            .contains("requires the system CA store or a trusted CA chain"));
    }

    #[test]
    fn issuer_key_and_chain_must_be_configured_together() {
        let mut conf = RaTlsConf::default();
        conf.certificate.issuer_private_key = b"not-a-key".to_vec();
        let error = rats_tls_init(conf).err().unwrap();
        assert!(error
            .to_string()
            .contains("must either both be configured or both be empty"));
    }

    #[test]
    fn disabled_tls_verification_ignores_verification_material() {
        let conf = RaTlsConf {
            tls_verify: TlsVerifyConf {
                verify_peer_certificate: false,
                use_system_ca: true,
                trusted_ca_chain: b"not-a-certificate".to_vec(),
                expected_peer_name: Some("ignored.example".into()),
            },
            ..Default::default()
        };
        assert!(rats_tls_init(conf).is_ok());
    }

    #[test]
    fn rejects_a_leaf_algorithm_that_the_backend_cannot_generate() {
        let conf = RaTlsConf {
            cert_algo: CertAlgorithm::Ed25519,
            ..Default::default()
        };
        assert_init_error_contains(conf, "dynamic leaf certificate algorithm");
    }

    #[test]
    fn rejects_oversized_certificate_material_before_parsing_it() {
        let conf = RaTlsConf {
            certificate: CertificateConf {
                issuer_private_key: vec![b'x'; 64 * 1024 + 1],
                issuer_certificate_chain: b"not-a-certificate".to_vec(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_init_error_contains(conf, "issuer private key exceeds");
    }

    #[test]
    fn rejects_invalid_or_excessive_subject_alternative_names_at_init() {
        let invalid = RaTlsConf {
            certificate: CertificateConf {
                subject_alt_names: vec!["DNS:bad name.example".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        assert_init_error_contains(invalid, "invalid DNS");

        let excessive = RaTlsConf {
            certificate: CertificateConf {
                subject_alt_names: vec!["DNS:example.com".into(); 65],
                ..Default::default()
            },
            ..Default::default()
        };
        assert_init_error_contains(excessive, "too many subject alternative names");
    }

    #[test]
    fn rejects_invalid_expected_peer_names_at_init() {
        let conf = RaTlsConf {
            tls_verify: TlsVerifyConf {
                verify_peer_certificate: true,
                use_system_ca: true,
                expected_peer_name: Some("bad peer name".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_init_error_contains(conf, "invalid expected peer DNS name");
    }

    #[test]
    fn rejects_reserved_and_duplicate_custom_claim_names() {
        let reserved = RaTlsConf {
            custom_claims: vec![CustomClaim {
                name: "pubkey-hash".into(),
                value: vec![1],
            }],
            ..Default::default()
        };
        assert_init_error_contains(reserved, "reserved custom claim name");

        let duplicate = RaTlsConf {
            custom_claims: vec![
                CustomClaim {
                    name: "application".into(),
                    value: vec![1],
                },
                CustomClaim {
                    name: "application".into(),
                    value: vec![2],
                },
            ],
            ..Default::default()
        };
        assert_init_error_contains(duplicate, "duplicate custom claim name");
    }

    #[test]
    fn rejects_excessive_custom_claim_data() {
        let claims = (0..5)
            .map(|index| CustomClaim {
                name: format!("claim-{index}"),
                value: vec![0; 60 * 1024],
            })
            .collect();
        let conf = RaTlsConf {
            custom_claims: claims,
            ..Default::default()
        };
        assert_init_error_contains(conf, "total custom claim data exceeds");
    }

    #[test]
    fn rejects_an_oversized_trusted_ca_bundle_when_verification_is_enabled() {
        let conf = RaTlsConf {
            tls_verify: TlsVerifyConf {
                verify_peer_certificate: true,
                trusted_ca_chain: vec![b'x'; 4 * 1024 * 1024 + 1],
                ..Default::default()
            },
            ..Default::default()
        };
        assert_init_error_contains(conf, "trusted CA chain exceeds");
    }
}
