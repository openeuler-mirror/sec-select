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

use crate::core::DICE_TAGGED_EVIDENCE_OID;
use crate::crypto_wrappers::{
    CertAlgorithm, CertificateRole, CryptoWrapper, RatlsCertificateInfo, RatlsCertificateIssuer,
    RatlsPrivateKey,
};
use crate::{RaTlsError, Result};
use foreign_types_shared::ForeignTypeRef;
use openssl::asn1::{Asn1Integer, Asn1Object, Asn1OctetString, Asn1Time};
use openssl::bn::MsbOption;
use openssl::pkey::Id;
use openssl::{
    bn::BigNum,
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::extension::{
        AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage,
        SubjectAlternativeName, SubjectKeyIdentifier,
    },
    x509::{X509Extension, X509NameBuilder, X509Ref, X509},
};
use std::cmp::Ordering;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

const CERT_NOT_BEFORE_SKEW_SECONDS: i64 = 5 * 60;
const SELF_SIGNED_CERT_VALIDITY_SECONDS: i64 = 3600 * 24 * 365;
const CA_SIGNED_CERT_VALIDITY_SECONDS: i64 = 3600 * 24;

/// OpenSSL-backed implementation of the crypto wrapper interface.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenSslCrypto;

impl CryptoWrapper for OpenSslCrypto {
    fn name(&self) -> &'static str {
        "openssl"
    }

    fn generate_private_key(&self, cert_algo: CertAlgorithm) -> Result<RatlsPrivateKey> {
        let key = match cert_algo {
            CertAlgorithm::Rsa3072Sha256 => PKey::from_rsa(Rsa::generate(3072)?)?,
            CertAlgorithm::Ecc256Sha256 => {
                let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
                PKey::from_ec_key(EcKey::generate(&group)?)?
            }
            algorithm => {
                return Err(RaTlsError::Unsupported(format!(
                    "cannot generate certificate key for {algorithm:?}"
                )))
            }
        };
        Ok(RatlsPrivateKey {
            cert_algo,
            private_key: key.private_key_to_pkcs8()?,
            public_key: key.public_key_to_der()?,
            cert: None,
        })
    }

    fn prepare_certificate_issuer(
        &self,
        private_key: &[u8],
        certificate_chain: &[u8],
    ) -> Result<RatlsCertificateIssuer> {
        let key = parse_signing_private_key(private_key)?;
        let mut certificates = X509::stack_from_pem(certificate_chain).map_err(|err| {
            RaTlsError::InvalidArgument(format!("invalid issuer certificate PEM bundle: {err}"))
        })?;
        if certificates.is_empty() {
            return Err(RaTlsError::InvalidArgument(
                "issuer certificate chain is empty".into(),
            ));
        }

        let issuer_index = certificates
            .iter()
            .position(|certificate| {
                certificate
                    .public_key()
                    .is_ok_and(|public_key| public_key.public_eq(&key))
            })
            .ok_or_else(|| {
                RaTlsError::InvalidArgument(
                    "issuer certificate chain has no certificate matching the issuer private key"
                        .into(),
                )
            })?;
        let issuer = certificates.remove(issuer_index);
        validate_ca_certificate(&issuer, "issuer")?;

        let mut ordered = vec![issuer];
        loop {
            let current = ordered.last().expect("issuer chain is non-empty");
            if is_self_signed(current)? {
                break;
            }
            let current_issuer = current.issuer_name().to_der()?;
            let next_index = certificates.iter().position(|candidate| {
                let subject_matches = candidate
                    .subject_name()
                    .to_der()
                    .is_ok_and(|subject| subject == current_issuer);
                subject_matches
                    && candidate
                        .public_key()
                        .is_ok_and(|public_key| current.verify(&public_key).unwrap_or(false))
            });
            let Some(next_index) = next_index else {
                break;
            };
            let next = certificates.remove(next_index);
            validate_ca_certificate(&next, "issuer chain")?;
            ordered.push(next);
        }

        let issuer_certificate_pem = ordered[0].to_pem()?;
        let last_is_root = ordered.last().is_some_and(|cert| {
            cert.subject_name().to_der().ok() == cert.issuer_name().to_der().ok()
        });
        let send_len = ordered.len().saturating_sub(usize::from(last_is_root));
        let certificate_chain_pem = ordered[..send_len]
            .iter()
            .map(|certificate| certificate.to_pem())
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(RatlsCertificateIssuer {
            private_key_pkcs8: key.private_key_to_pkcs8()?,
            issuer_certificate_pem,
            certificate_chain_pem,
        })
    }

    fn generate_ra_certificate(
        &self,
        conf: &mut RatlsPrivateKey,
        cert_info: RatlsCertificateInfo,
    ) -> Result<()> {
        let mut name = X509NameBuilder::new()?;
        name.append_entry_by_text("O", cert_info.organization)?;
        name.append_entry_by_text("CN", cert_info.common_name)?;
        let name = name.build();

        let mut cert = X509::builder()?;
        cert.set_version(2)?;
        let mut serial_bn = BigNum::new()?;
        serial_bn.rand(128, MsbOption::ONE, false)?;
        let serial = Asn1Integer::from_bn(serial_bn.as_ref())?;
        cert.set_serial_number(&serial)?;
        cert.set_subject_name(&name)?;
        let leaf_private_key = PKey::private_key_from_pkcs8(&conf.private_key)?;
        cert.set_pubkey(&leaf_private_key)?;

        let issuer_certificate = cert_info
            .issuer
            .map(|issuer| X509::from_pem(&issuer.issuer_certificate_pem))
            .transpose()?;
        let signing_private_key = cert_info
            .issuer
            .map(|issuer| PKey::private_key_from_pkcs8(&issuer.private_key_pkcs8))
            .transpose()?;
        match issuer_certificate.as_ref() {
            Some(issuer) => cert.set_issuer_name(issuer.subject_name())?,
            None => cert.set_issuer_name(&name)?,
        }

        let now = unix_timestamp_now()?;
        cert.set_not_before(Asn1Time::from_unix(now - CERT_NOT_BEFORE_SKEW_SECONDS)?.as_ref())?;
        let validity = if issuer_certificate.is_some() {
            CA_SIGNED_CERT_VALIDITY_SECONDS
        } else {
            SELF_SIGNED_CERT_VALIDITY_SECONDS
        };
        let desired_not_after = Asn1Time::from_unix(now + validity)?;
        match issuer_certificate.as_ref() {
            Some(issuer)
                if issuer.not_after().compare(desired_not_after.as_ref())? == Ordering::Less =>
            {
                cert.set_not_after(issuer.not_after())?
            }
            _ => cert.set_not_after(desired_not_after.as_ref())?,
        }

        append_common_x509_extensions(
            &mut cert,
            issuer_certificate.as_deref(),
            cert_info.role,
            cert_info.subject_alt_names,
        )?;
        if let Some(evidence_buffer) = cert_info.evidence_buffer {
            let oid = Asn1Object::from_str(DICE_TAGGED_EVIDENCE_OID)?;
            let value = Asn1OctetString::new_from_bytes(evidence_buffer)?;
            let extension = X509Extension::new_from_der(&oid, false, &value)?;
            cert.append_extension(extension)?;
        }
        let signing_private_key = signing_private_key.as_ref().unwrap_or(&leaf_private_key);
        cert.sign(signing_private_key, signing_digest(signing_private_key)?)?;
        conf.cert = Some(cert.build().to_pem()?);
        Ok(())
    }
}

fn parse_signing_private_key(private_key: &[u8]) -> Result<PKey<Private>> {
    let key = PKey::private_key_from_pem(private_key).map_err(|err| {
        RaTlsError::InvalidArgument(format!(
            "invalid or encrypted issuer private key PEM: {err}"
        ))
    })?;
    match key.id() {
        Id::RSA => {
            let bits = key.rsa()?.size() * 8;
            if bits < 2048 {
                return Err(RaTlsError::InvalidArgument(format!(
                    "RSA issuer private key must be at least 2048 bits, got {bits}"
                )));
            }
        }
        Id::EC => match key.ec_key()?.group().curve_name() {
            Some(Nid::X9_62_PRIME256V1 | Nid::SECP384R1 | Nid::SECP521R1) => {}
            curve => {
                return Err(RaTlsError::Unsupported(format!(
                    "unsupported issuer EC curve: {curve:?}"
                )))
            }
        },
        Id::ED25519 | Id::ED448 => {}
        Id::X25519 | Id::X448 => {
            return Err(RaTlsError::Unsupported(
                "X25519/X448 keys cannot sign certificates".into(),
            ))
        }
        other => {
            return Err(RaTlsError::Unsupported(format!(
                "unsupported issuer private key type: {other:?}"
            )))
        }
    }
    Ok(key)
}

fn validate_ca_certificate(certificate: &X509Ref, name: &str) -> Result<()> {
    let now = Asn1Time::days_from_now(0)?;
    if certificate.not_before().compare(now.as_ref())? == Ordering::Greater
        || certificate.not_after().compare(now.as_ref())? == Ordering::Less
    {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} CA certificate is expired or not yet valid"
        )));
    }
    let extension_flags = unsafe { openssl_sys::X509_get_extension_flags(certificate.as_ptr()) };
    if extension_flags & openssl_sys::EXFLAG_CA == 0 {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} certificate is not a CA certificate"
        )));
    }
    let key_usage = unsafe { openssl_sys::X509_get_key_usage(certificate.as_ptr()) };
    if key_usage != u32::MAX && key_usage & openssl_sys::X509v3_KU_KEY_CERT_SIGN == 0 {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} CA certificate cannot sign certificates"
        )));
    }
    Ok(())
}

fn is_self_signed(certificate: &X509Ref) -> Result<bool> {
    if certificate.subject_name().to_der()? != certificate.issuer_name().to_der()? {
        return Ok(false);
    }
    let public_key = certificate.public_key()?;
    Ok(certificate.verify(public_key.as_ref())?)
}

fn signing_digest(private_key: &PKey<Private>) -> Result<MessageDigest> {
    match private_key.id() {
        Id::ED25519 | Id::ED448 => Ok(MessageDigest::null()),
        Id::EC => match private_key.ec_key()?.group().curve_name() {
            Some(Nid::SECP384R1) => Ok(MessageDigest::sha384()),
            Some(Nid::SECP521R1) => Ok(MessageDigest::sha512()),
            _ => Ok(MessageDigest::sha256()),
        },
        _ => Ok(MessageDigest::sha256()),
    }
}

fn unix_timestamp_now() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| {
            RaTlsError::InvalidData(format!("system time is before UNIX epoch: {err}"))
        })?;
    i64::try_from(duration.as_secs())
        .map_err(|_| RaTlsError::InvalidData("system time exceeds OpenSSL ASN.1 range".into()))
}

fn append_common_x509_extensions(
    cert: &mut openssl::x509::X509Builder,
    issuer: Option<&X509Ref>,
    role: CertificateRole,
    subject_alt_names: &[String],
) -> Result<()> {
    cert.append_extension(BasicConstraints::new().critical().build()?)?;
    cert.append_extension(KeyUsage::new().critical().digital_signature().build()?)?;
    let extended_key_usage = match role {
        CertificateRole::Server => ExtendedKeyUsage::new().server_auth().build()?,
        CertificateRole::Client => ExtendedKeyUsage::new().client_auth().build()?,
    };
    cert.append_extension(extended_key_usage)?;

    let subject_key_identifier = {
        let ctx = cert.x509v3_context(issuer, None);
        SubjectKeyIdentifier::new().build(&ctx)?
    };
    cert.append_extension(subject_key_identifier)?;

    let authority_key_identifier = {
        let ctx = cert.x509v3_context(issuer, None);
        AuthorityKeyIdentifier::new().keyid(false).build(&ctx)?
    };
    cert.append_extension(authority_key_identifier)?;

    if !subject_alt_names.is_empty() {
        let mut san = SubjectAlternativeName::new();
        for entry in subject_alt_names {
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
                "DNS" => {
                    san.dns(value);
                }
                "IP" => {
                    let ip = value.parse::<IpAddr>().map_err(|_| {
                        RaTlsError::InvalidArgument(format!(
                            "subject alternative name has an invalid IP address: {value}"
                        ))
                    })?;
                    san.ip(&ip.to_string());
                }
                "URI" => {
                    san.uri(value);
                }
                _ => {
                    return Err(RaTlsError::InvalidArgument(format!(
                        "unsupported subject alternative name kind '{kind}'"
                    )))
                }
            }
        }
        let ctx = cert.x509v3_context(issuer, None);
        cert.append_extension(san.build(&ctx)?)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::x509::extension::{BasicConstraints, KeyUsage};

    #[test]
    fn generates_the_selected_certificate_key_type() {
        let crypto = OpenSslCrypto;

        let rsa = crypto
            .generate_private_key(CertAlgorithm::Rsa3072Sha256)
            .unwrap();
        let rsa = PKey::private_key_from_pkcs8(&rsa.private_key).unwrap();
        assert_eq!(rsa.id(), Id::RSA);
        assert_eq!(rsa.rsa().unwrap().size() * 8, 3072);

        let ec = crypto
            .generate_private_key(CertAlgorithm::Ecc256Sha256)
            .unwrap();
        let ec = PKey::private_key_from_pkcs8(&ec.private_key).unwrap();
        assert_eq!(ec.id(), Id::EC);
        assert_eq!(
            ec.ec_key().unwrap().group().curve_name(),
            Some(Nid::X9_62_PRIME256V1)
        );
    }

    #[test]
    fn prepares_an_unordered_issuer_chain_and_signs_a_dynamic_leaf() {
        let root_key = ec_key();
        let root = ca_certificate("Test Root", &root_key, None);
        let intermediate_key = ec_key();
        let intermediate = ca_certificate(
            "Test Intermediate",
            &intermediate_key,
            Some((&root, &root_key)),
        );

        let mut unordered_bundle = root.to_pem().unwrap();
        unordered_bundle.extend_from_slice(&intermediate.to_pem().unwrap());
        let crypto = OpenSslCrypto;
        let issuer = crypto
            .prepare_certificate_issuer(
                &intermediate_key.private_key_to_pem_pkcs8().unwrap(),
                &unordered_bundle,
            )
            .unwrap();
        assert_eq!(issuer.certificate_chain_pem.len(), 1);
        let prepared_intermediate = X509::from_pem(&issuer.issuer_certificate_pem).unwrap();
        assert!(prepared_intermediate
            .public_key()
            .unwrap()
            .public_eq(&intermediate_key));

        let mut leaf = crypto
            .generate_private_key(CertAlgorithm::Ecc256Sha256)
            .unwrap();
        crypto
            .generate_ra_certificate(
                &mut leaf,
                RatlsCertificateInfo {
                    organization: "Test",
                    common_name: "RA-TLS",
                    evidence_buffer: Some(b"evidence"),
                    issuer: Some(&issuer),
                    role: CertificateRole::Server,
                    subject_alt_names: &["DNS:server.example.com".into()],
                },
            )
            .unwrap();
        let leaf = X509::from_pem(leaf.cert.as_ref().unwrap()).unwrap();
        assert_eq!(
            leaf.issuer_name().to_der().unwrap(),
            intermediate.subject_name().to_der().unwrap()
        );
        assert!(leaf.verify(&intermediate_key).unwrap());
        assert!(intermediate.verify(&root_key).unwrap());
        let names = leaf.subject_alt_names().unwrap();
        assert_eq!(names[0].dnsname(), Some("server.example.com"));
    }

    #[test]
    fn rejects_an_issuer_chain_that_does_not_match_the_private_key() {
        let root_key = ec_key();
        let root = ca_certificate("Test Root", &root_key, None);
        let unrelated_key = ec_key();
        let error = OpenSslCrypto
            .prepare_certificate_issuer(
                &unrelated_key.private_key_to_pem_pkcs8().unwrap(),
                &root.to_pem().unwrap(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("no certificate matching"));
    }

    fn ec_key() -> PKey<Private> {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap()
    }

    fn ca_certificate(
        common_name: &str,
        key: &PKey<Private>,
        issuer: Option<(&X509Ref, &PKey<Private>)>,
    ) -> X509 {
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", common_name).unwrap();
        let name = name.build();
        let mut certificate = X509::builder().unwrap();
        certificate.set_version(2).unwrap();
        let serial = Asn1Integer::from_bn(BigNum::from_u32(42).unwrap().as_ref()).unwrap();
        certificate.set_serial_number(&serial).unwrap();
        certificate.set_subject_name(&name).unwrap();
        certificate
            .set_issuer_name(
                issuer
                    .map(|(certificate, _)| certificate.subject_name())
                    .unwrap_or(&name),
            )
            .unwrap();
        certificate.set_pubkey(key).unwrap();
        certificate
            .set_not_before(Asn1Time::days_from_now(0).unwrap().as_ref())
            .unwrap();
        certificate
            .set_not_after(Asn1Time::days_from_now(30).unwrap().as_ref())
            .unwrap();
        certificate
            .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        certificate
            .append_extension(KeyUsage::new().key_cert_sign().crl_sign().build().unwrap())
            .unwrap();
        certificate
            .sign(
                issuer.map(|(_, key)| key).unwrap_or(key),
                MessageDigest::sha256(),
            )
            .unwrap();
        certificate.build()
    }
}
