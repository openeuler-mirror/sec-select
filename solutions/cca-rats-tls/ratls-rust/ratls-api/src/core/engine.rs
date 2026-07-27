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

use crate::attesters::Attester;
use crate::core::claims::build_claims_buffer;
use crate::core::dice::{
    decode_evidence_buffer, encode_evidence_buffer, CustomClaim, EvidenceBuffer,
};
use crate::core::{
    RaTlsHandle, CERT_SUBJECT_COMMON_NAME, CERT_SUBJECT_ORGANIZATION, DICE_TAGGED_EVIDENCE_OID,
};
use crate::crypto_wrappers::{CertAlgorithm, CryptoWrapper, RatlsCertificateInfo};
use crate::crypto_wrappers::{CertificateRole, RatlsCertificateIssuer};
use crate::tls_wrappers::{TlsEvidenceBinding, TlsIdentity, TlsPeerData, TransportStream};
use crate::verifiers::Evidence;
use crate::RaTlsError;
use foreign_types_shared::ForeignTypeRef;
use openssl::x509::{X509Ref, X509};
use std::os::raw::c_char;
use std::slice;

pub fn create_ratls_server(
    handle: &mut RaTlsHandle,
    stream: Box<dyn TransportStream>,
) -> Result<Box<dyn TransportStream>, RaTlsError> {
    let mut handshake =
        handle
            .tls
            .start_server_handshake(stream, handle.conf.mutual, &handle.conf.tls_verify)?;
    let binding = handshake.evidence_binding()?.ok_or_else(|| {
        RaTlsError::InvalidData("server handshake did not provide evidence binding".into())
    })?;
    let identity = generate_ratls_identity(
        handle.crypto.as_ref(),
        handle.attester.as_deref_mut().ok_or_else(|| {
            RaTlsError::InvalidArgument("server configuration requires an attester".into())
        })?,
        IdentityCertificateConfig {
            cert_algo: handle.conf.cert_algo,
            custom_claims: &handle.conf.custom_claims,
            issuer: handle.certificate_issuer.as_ref(),
            subject_alt_names: &handle.conf.certificate.subject_alt_names,
            role: CertificateRole::Server,
        },
        &binding,
    )?;
    handshake.install_identity(identity)?;
    let negotiated = handshake.finish()?;
    if handle.conf.mutual {
        let peer = negotiated.peer.as_ref().ok_or_else(|| {
            RaTlsError::InvalidData("mutual TLS client certificate is missing".into())
        })?;
        let evidence = verify_handshake_evidence(handle, peer)?;
        verify_user_callback(handle, &evidence)?;
    }

    crate::rtls_info!("RATS-TLS server negotiation completed");
    Ok(negotiated.stream)
}

struct IdentityCertificateConfig<'a> {
    cert_algo: CertAlgorithm,
    custom_claims: &'a [CustomClaim],
    issuer: Option<&'a RatlsCertificateIssuer>,
    subject_alt_names: &'a [String],
    role: CertificateRole,
}

fn generate_ratls_identity(
    crypto: &dyn CryptoWrapper,
    attester: &mut dyn Attester,
    config: IdentityCertificateConfig<'_>,
    binding: &TlsEvidenceBinding,
) -> Result<TlsIdentity, RaTlsError> {
    if binding.nonce.len() != 32 {
        return Err(RaTlsError::InvalidData(format!(
            "invalid TLS nonce length: expected 32, got {}",
            binding.nonce.len()
        )));
    }
    if binding.client_key_share.is_empty() {
        return Err(RaTlsError::InvalidData(
            "TLS ClientHello does not contain a key_share".into(),
        ));
    }

    let mut key = crypto.generate_private_key(config.cert_algo)?;
    let public_key_hash = openssl::sha::sha256(key.public_key.as_slice());
    let client_key_share_hash = openssl::sha::sha256(binding.client_key_share.as_slice());

    let claims_buffer = build_claims_buffer(
        &public_key_hash,
        &client_key_share_hash,
        &binding.nonce,
        config.custom_claims,
    )?;
    let challenge = openssl::sha::sha256(claims_buffer.as_slice());
    let evidence = attester.collect_evidence(&challenge)?;
    let evidence_buffer = encode_evidence_buffer(&EvidenceBuffer {
        tag: attester.evidence_tag(),
        evidence_raw: evidence.raw,
        claims_buffer,
    })?;
    crypto.generate_ra_certificate(
        &mut key,
        RatlsCertificateInfo {
            organization: CERT_SUBJECT_ORGANIZATION,
            common_name: CERT_SUBJECT_COMMON_NAME,
            evidence_buffer: Some(&evidence_buffer),
            issuer: config.issuer,
            role: config.role,
            subject_alt_names: config.subject_alt_names,
        },
    )?;

    let leaf_certificate_pem = key
        .cert
        .take()
        .ok_or_else(|| RaTlsError::InvalidData("RA-TLS certificate was not generated".into()))?;
    let mut certificate_chain_pem = vec![leaf_certificate_pem];
    if let Some(issuer) = config.issuer {
        certificate_chain_pem.extend(issuer.certificate_chain_pem.iter().cloned());
    }
    Ok(TlsIdentity {
        certificate_chain_pem,
        private_key_pkcs8: key.private_key,
    })
}

pub fn create_ratls_client(
    handle: &mut RaTlsHandle,
    stream: Box<dyn TransportStream>,
) -> Result<Box<dyn TransportStream>, RaTlsError> {
    if handle.conf.server {
        return Err(RaTlsError::InvalidArgument(
            "create_ratls_client cannot be used with server configuration".into(),
        ));
    }

    crate::rtls_debug!("RATS-TLS client negotiation started");
    let mutual = handle.conf.mutual;
    let mut handshake =
        handle
            .tls
            .start_client_handshake(stream, mutual, &handle.conf.tls_verify)?;
    if let Some(binding) = handshake.evidence_binding()? {
        let identity = generate_ratls_identity(
            handle.crypto.as_ref(),
            handle.attester.as_deref_mut().ok_or_else(|| {
                RaTlsError::InvalidArgument(
                    "mutual client configuration requires an attester".into(),
                )
            })?,
            IdentityCertificateConfig {
                cert_algo: handle.conf.cert_algo,
                custom_claims: &handle.conf.custom_claims,
                issuer: handle.certificate_issuer.as_ref(),
                subject_alt_names: &handle.conf.certificate.subject_alt_names,
                role: CertificateRole::Client,
            },
            &binding,
        )?;
        handshake.install_identity(identity)?;
    } else if mutual {
        return Err(RaTlsError::InvalidData(
            "mutual client handshake did not request a local identity".into(),
        ));
    }
    let negotiated = handshake.finish()?;
    let peer = negotiated
        .peer
        .as_ref()
        .ok_or_else(|| RaTlsError::InvalidData("TLS server certificate is missing".into()))?;
    let evidence = verify_handshake_evidence(handle, peer)?;
    verify_user_callback(handle, &evidence)?;
    crate::rtls_info!("RATS-TLS client negotiation completed");
    Ok(negotiated.stream)
}

fn verify_handshake_evidence(
    handle: &mut RaTlsHandle,
    peer: &TlsPeerData,
) -> Result<Evidence, RaTlsError> {
    let certificate = X509::from_der(&peer.certificate_der)?;
    let evidence_extension = extract_evidence_extension(certificate.as_ref())?;
    let evidence_buffer = decode_evidence_buffer(&evidence_extension)?;
    let public_key_der = certificate.public_key()?.public_key_to_der()?;
    let expected_client_key_share_hash =
        openssl::sha::sha256(&peer.evidence_binding.client_key_share).to_vec();
    let verifier_plugin = handle.conf.verifier.ok_or_else(|| {
        RaTlsError::InvalidArgument("peer verification requires a verifier".into())
    })?;
    let mut evidence = Evidence {
        tag: evidence_buffer.tag,
        name: verifier_plugin,
        raw_evidence: evidence_buffer.evidence_raw,
        claims_buffer: evidence_buffer.claims_buffer,
        public_key_der,
        expected_nonce: peer.evidence_binding.nonce.clone(),
        expected_client_key_share_hash,
        evidence_json: serde_json::Value::Null,
    };

    crate::rtls_debug!("verifying peer handshake evidence");
    let verifier = handle.verifier.as_deref_mut().ok_or_else(|| {
        RaTlsError::InvalidArgument("peer verification requires a loaded verifier".into())
    })?;
    verifier.verify_evidence(&mut evidence)?;
    crate::rtls_info!("peer handshake evidence verified by '{}'", verifier.name());
    Ok(evidence)
}

fn verify_user_callback(handle: &mut RaTlsHandle, evidence: &Evidence) -> Result<(), RaTlsError> {
    let Some(callback) = handle.user_callback.as_mut() else {
        crate::rtls_debug!("no user verification callback configured");
        return Ok(());
    };
    crate::rtls_debug!("calling user verification callback");
    callback(evidence)?;
    crate::rtls_info!("user verification callback accepted peer evidence");
    Ok(())
}

fn extract_evidence_extension(certificate: &X509Ref) -> Result<Vec<u8>, RaTlsError> {
    let mut found = None;
    unsafe {
        let extensions = openssl_sys::X509_get0_extensions(certificate.as_ptr());
        if extensions.is_null() {
            return Err(RaTlsError::InvalidData(
                "peer certificate has no extensions".into(),
            ));
        }
        let count = openssl_sys::OPENSSL_sk_num(extensions as *const _);
        for index in 0..count {
            let extension = openssl_sys::OPENSSL_sk_value(extensions as *const _, index);
            if extension.is_null() {
                continue;
            }
            let extension = extension as *mut openssl_sys::X509_EXTENSION;
            let object = openssl_sys::X509_EXTENSION_get_object(extension);
            if object.is_null() || object_to_text(object) != DICE_TAGGED_EVIDENCE_OID {
                continue;
            }
            if found.is_some() {
                return Err(RaTlsError::InvalidData(
                    "peer certificate contains duplicate RA-TLS evidence extensions".into(),
                ));
            }
            let data = openssl_sys::X509_EXTENSION_get_data(extension);
            if data.is_null() {
                return Err(RaTlsError::InvalidData(
                    "RA-TLS evidence extension is empty".into(),
                ));
            }
            let len = openssl_sys::ASN1_STRING_length(data as *const _) as usize;
            let data = openssl_sys::ASN1_STRING_get0_data(data as *const _);
            if data.is_null() {
                return Err(RaTlsError::InvalidData(
                    "RA-TLS evidence extension has null data".into(),
                ));
            }
            found = Some(slice::from_raw_parts(data, len).to_vec());
        }
    }
    found.ok_or_else(|| {
        RaTlsError::InvalidData("peer certificate has no RA-TLS evidence extension".into())
    })
}

unsafe fn object_to_text(object: *const openssl_sys::ASN1_OBJECT) -> String {
    let mut buffer = [0 as c_char; 128];
    let len = openssl_sys::OBJ_obj2txt(buffer.as_mut_ptr(), buffer.len() as i32, object, 1);
    if len <= 0 {
        return String::new();
    }
    let len = (len as usize).min(buffer.len().saturating_sub(1));
    let bytes = slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), len);
    String::from_utf8_lossy(bytes).into_owned()
}
