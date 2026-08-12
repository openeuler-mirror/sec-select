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

//! End-to-end tests for the public Rust API.
//!
//! The mock verifier deliberately accepts arbitrary CCA evidence. These tests
//! cover API orchestration, dynamic certificates, TLS policy, callbacks, and
//! application I/O; they do not replace real CCA evidence-verification tests.

use openssl::asn1::{Asn1Integer, Asn1Time};
use openssl::bn::BigNum;
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::PKey;
use openssl::x509::extension::{BasicConstraints, KeyUsage, SubjectKeyIdentifier};
use openssl::x509::{X509NameBuilder, X509};
use ratls_api::api::{
    rats_tls_init, rats_tls_negotiate, rats_tls_receive, rats_tls_set_verification_callback,
    rats_tls_transmit,
};
use ratls_api::attesters::{Attester, AttesterPlugin};
use ratls_api::core::dice::{parse_claims_buffer, CustomClaim};
use ratls_api::core::evidence::AttestationEvidence;
use ratls_api::core::{CertificateConf, RaTlsConf, RaTlsHandle, TlsVerifyConf};
use ratls_api::verifiers::cca::EVIDENCE_TAG;
use ratls_api::verifiers::{Evidence, Verifier, VerifierPlugin};
use ratls_api::RaTlsError;
use serde_json::json;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

const MOCK_EVIDENCE: &[u8] = b"ratls-api integration-test CCA evidence";
const SERVER_DNS_NAME: &str = "server.ratls.test";
const CLIENT_DNS_NAME: &str = "client.ratls.test";
const APPLICATION_DATA_SIZE: usize = 128 * 1024;

#[derive(Clone, Default)]
struct MockCalls {
    attester: Arc<AtomicUsize>,
    verifier: Arc<AtomicUsize>,
    callback: Arc<AtomicUsize>,
}

struct MockCcaAttester {
    calls: Arc<AtomicUsize>,
}

impl Attester for MockCcaAttester {
    fn name(&self) -> &'static str {
        "mock-cca"
    }

    fn evidence_tag(&self) -> u64 {
        EVIDENCE_TAG
    }

    fn collect_evidence(&self, _challenge: &[u8]) -> Result<AttestationEvidence, RaTlsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AttestationEvidence {
            raw: MOCK_EVIDENCE.to_vec(),
        })
    }
}

struct MockCcaVerifier {
    calls: Arc<AtomicUsize>,
}

impl Verifier for MockCcaVerifier {
    fn name(&self) -> &'static str {
        "mock-cca"
    }

    fn evidence_tag(&self) -> u64 {
        EVIDENCE_TAG
    }

    fn verify_evidence(&mut self, evidence: &mut Evidence) -> Result<(), RaTlsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        evidence.evidence_json = json!({
            "mock": true,
            "raw_evidence_len": evidence.raw_evidence.len(),
        });
        Ok(())
    }
}

struct TestCa {
    private_key_pem: Vec<u8>,
    certificate_pem: Vec<u8>,
}

#[test]
fn one_way_self_signed_full_flow() {
    let server_calls = MockCalls::default();
    let client_calls = MockCalls::default();
    let mut server = initialise_with_mocks(
        one_way_server_conf("server-claim", b"from-server"),
        &server_calls,
    );
    let mut client = initialise_with_mocks(one_way_client_conf(), &client_calls);
    install_accepting_callback(&mut client, "server-claim", b"from-server", &client_calls);

    exchange_application_data(&mut server, &mut client);

    assert_mock_calls(&server_calls, 1, 0, 0);
    assert_mock_calls(&client_calls, 0, 1, 1);
}

#[test]
fn mutual_ca_signed_full_flow() {
    let server_ca = test_ca("RATS-TLS Server Test Root", 1);
    let client_ca = test_ca("RATS-TLS Client Test Root", 2);
    let (server_conf, client_conf) =
        mutual_ca_signed_confs(&server_ca, &client_ca, None, None, None, None);
    let server_calls = MockCalls::default();
    let client_calls = MockCalls::default();
    let mut server = initialise_with_mocks(server_conf, &server_calls);
    let mut client = initialise_with_mocks(client_conf, &client_calls);
    install_accepting_callback(&mut server, "client-claim", b"from-client", &server_calls);
    install_accepting_callback(&mut client, "server-claim", b"from-server", &client_calls);

    exchange_application_data(&mut server, &mut client);

    assert_mock_calls(&server_calls, 1, 1, 1);
    assert_mock_calls(&client_calls, 1, 1, 1);
}

#[test]
fn rejects_wrong_server_name() {
    let server_ca = test_ca("RATS-TLS Server Test Root", 3);
    let client_ca = test_ca("RATS-TLS Client Test Root", 4);
    let (server_conf, client_conf) = mutual_ca_signed_confs(
        &server_ca,
        &client_ca,
        None,
        None,
        Some("wrong-server.ratls.test"),
        None,
    );
    let mut server = initialise_with_mocks(server_conf, &MockCalls::default());
    let mut client = initialise_with_mocks(client_conf, &MockCalls::default());

    let (_server_result, client_result) = negotiate_results(&mut server, &mut client);

    assert!(
        client_result.is_err(),
        "client accepted a server certificate for the wrong DNS name"
    );
}

#[test]
fn client_rejects_untrusted_server_ca() {
    let server_ca = test_ca("RATS-TLS Server Test Root", 5);
    let client_ca = test_ca("RATS-TLS Client Test Root", 6);
    let untrusted_ca = test_ca("Untrusted Server Test Root", 7);
    let (server_conf, client_conf) = mutual_ca_signed_confs(
        &server_ca,
        &client_ca,
        None,
        None,
        None,
        Some(&untrusted_ca.certificate_pem),
    );
    let mut server = initialise_with_mocks(server_conf, &MockCalls::default());
    let mut client = initialise_with_mocks(client_conf, &MockCalls::default());

    let (_server_result, client_result) = negotiate_results(&mut server, &mut client);

    assert!(
        client_result.is_err(),
        "client accepted a server certificate from an untrusted CA"
    );
}

#[test]
fn server_rejects_untrusted_client_ca() {
    let server_ca = test_ca("RATS-TLS Server Test Root", 8);
    let client_ca = test_ca("RATS-TLS Client Test Root", 9);
    let untrusted_ca = test_ca("Untrusted Client Test Root", 10);
    let (server_conf, client_conf) = mutual_ca_signed_confs(
        &server_ca,
        &client_ca,
        None,
        Some(&untrusted_ca.certificate_pem),
        None,
        None,
    );
    let mut server = initialise_with_mocks(server_conf, &MockCalls::default());
    let mut client = initialise_with_mocks(client_conf, &MockCalls::default());

    let (server_result, _client_result) = negotiate_results(&mut server, &mut client);

    assert!(
        server_result.is_err(),
        "server accepted a client certificate from an untrusted CA"
    );
}

#[test]
fn verification_callback_rejects_peer() {
    let server_calls = MockCalls::default();
    let client_calls = MockCalls::default();
    let mut server = initialise_with_mocks(
        one_way_server_conf("server-claim", b"from-server"),
        &server_calls,
    );
    let mut client = initialise_with_mocks(one_way_client_conf(), &client_calls);
    let callback_calls = Arc::clone(&client_calls.callback);
    rats_tls_set_verification_callback(
        &mut client,
        Some(Box::new(move |_| {
            callback_calls.fetch_add(1, Ordering::SeqCst);
            Err(RaTlsError::InvalidData(
                "test policy rejected peer evidence".into(),
            ))
        })),
    );

    let (_server_result, client_result) = negotiate_results(&mut server, &mut client);

    let error = client_result.expect_err("client callback should reject the peer");
    assert!(
        error.contains("test policy rejected peer evidence"),
        "{error}"
    );
    assert_mock_calls(&server_calls, 1, 0, 0);
    assert_mock_calls(&client_calls, 0, 1, 1);
}

fn one_way_server_conf(custom_claim_name: &str, custom_claim_value: &[u8]) -> RaTlsConf {
    RaTlsConf {
        attester: Some(AttesterPlugin::Cca),
        verifier: None,
        server: true,
        custom_claims: vec![CustomClaim {
            name: custom_claim_name.into(),
            value: custom_claim_value.to_vec(),
        }],
        certificate: CertificateConf {
            subject_alt_names: vec![format!("DNS:{SERVER_DNS_NAME}")],
            ..Default::default()
        },
        ..Default::default()
    }
}

fn one_way_client_conf() -> RaTlsConf {
    RaTlsConf {
        attester: None,
        verifier: Some(VerifierPlugin::Cca),
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn mutual_ca_signed_confs(
    server_ca: &TestCa,
    client_ca: &TestCa,
    server_expected_client_name: Option<&str>,
    server_trusted_ca_override: Option<&[u8]>,
    client_expected_server_name: Option<&str>,
    client_trusted_ca_override: Option<&[u8]>,
) -> (RaTlsConf, RaTlsConf) {
    let server = RaTlsConf {
        attester: Some(AttesterPlugin::Cca),
        verifier: Some(VerifierPlugin::Cca),
        mutual: true,
        server: true,
        custom_claims: vec![CustomClaim {
            name: "server-claim".into(),
            value: b"from-server".to_vec(),
        }],
        certificate: CertificateConf {
            issuer_private_key: server_ca.private_key_pem.clone(),
            issuer_certificate_chain: server_ca.certificate_pem.clone(),
            subject_alt_names: vec![
                format!("DNS:{SERVER_DNS_NAME}"),
                "URI:spiffe://ratls.test/server".into(),
            ],
        },
        tls_verify: TlsVerifyConf {
            verify_peer_certificate: true,
            use_system_ca: true,
            trusted_ca_chain: server_trusted_ca_override
                .unwrap_or(&client_ca.certificate_pem)
                .to_vec(),
            expected_peer_name: Some(
                server_expected_client_name
                    .unwrap_or(CLIENT_DNS_NAME)
                    .into(),
            ),
        },
        ..Default::default()
    };
    let client = RaTlsConf {
        attester: Some(AttesterPlugin::Cca),
        verifier: Some(VerifierPlugin::Cca),
        mutual: true,
        server: false,
        custom_claims: vec![CustomClaim {
            name: "client-claim".into(),
            value: b"from-client".to_vec(),
        }],
        certificate: CertificateConf {
            issuer_private_key: client_ca.private_key_pem.clone(),
            issuer_certificate_chain: client_ca.certificate_pem.clone(),
            subject_alt_names: vec![
                format!("DNS:{CLIENT_DNS_NAME}"),
                "URI:spiffe://ratls.test/client".into(),
            ],
        },
        tls_verify: TlsVerifyConf {
            verify_peer_certificate: true,
            use_system_ca: true,
            trusted_ca_chain: client_trusted_ca_override
                .unwrap_or(&server_ca.certificate_pem)
                .to_vec(),
            expected_peer_name: Some(
                client_expected_server_name
                    .unwrap_or(SERVER_DNS_NAME)
                    .into(),
            ),
        },
        ..Default::default()
    };
    (server, client)
}

fn initialise_with_mocks(conf: RaTlsConf, calls: &MockCalls) -> RaTlsHandle {
    let mut handle = rats_tls_init(conf).expect("test configuration should initialize");
    if handle.attester.is_some() {
        handle.attester = Some(Box::new(MockCcaAttester {
            calls: Arc::clone(&calls.attester),
        }));
    }
    if handle.verifier.is_some() {
        handle.verifier = Some(Box::new(MockCcaVerifier {
            calls: Arc::clone(&calls.verifier),
        }));
    }
    handle
}

fn install_accepting_callback(
    handle: &mut RaTlsHandle,
    expected_claim_name: &'static str,
    expected_claim_value: &'static [u8],
    calls: &MockCalls,
) {
    let callback_calls = Arc::clone(&calls.callback);
    rats_tls_set_verification_callback(
        handle,
        Some(Box::new(move |evidence| {
            callback_calls.fetch_add(1, Ordering::SeqCst);
            if evidence.raw_evidence != MOCK_EVIDENCE {
                return Err(RaTlsError::InvalidData(
                    "callback received unexpected mock evidence".into(),
                ));
            }
            if evidence.evidence_json.get("mock") != Some(&json!(true)) {
                return Err(RaTlsError::InvalidData(
                    "callback did not receive the mock verifier result".into(),
                ));
            }
            let claims = parse_claims_buffer(&evidence.claims_buffer)?;
            let expected_claim = claims
                .custom_claims
                .iter()
                .find(|claim| claim.name == expected_claim_name)
                .ok_or_else(|| {
                    RaTlsError::InvalidData(format!(
                        "callback did not receive custom claim {expected_claim_name}"
                    ))
                })?;
            if expected_claim.value != expected_claim_value {
                return Err(RaTlsError::InvalidData(format!(
                    "callback received the wrong value for custom claim {expected_claim_name}"
                )));
            }
            Ok(())
        })),
    );
}

fn exchange_application_data(server: &mut RaTlsHandle, client: &mut RaTlsHandle) {
    let (server_stream, client_stream) = stream_pair();
    let request = test_payload(0x35);
    let response = test_payload(0xa7);
    let expected_request = request.clone();
    let server_response = response.clone();

    let server_thread = thread::scope(|scope| {
        let server_thread = scope.spawn(move || -> Result<(), String> {
            let mut stream =
                rats_tls_negotiate(server, Box::new(server_stream)).map_err(|e| e.to_string())?;
            let received = receive_exact(stream.as_mut(), expected_request.len())?;
            if received != expected_request {
                return Err("server received corrupted application data".into());
            }
            transmit_all(stream.as_mut(), &server_response)?;
            Ok(())
        });

        let client_result = (|| -> Result<(), String> {
            let mut stream =
                rats_tls_negotiate(client, Box::new(client_stream)).map_err(|e| e.to_string())?;
            transmit_all(stream.as_mut(), &request)?;
            let received = receive_exact(stream.as_mut(), response.len())?;
            if received != response {
                return Err("client received corrupted application data".into());
            }
            Ok(())
        })();
        let server_result = server_thread.join().expect("server thread panicked");
        (server_result, client_result)
    });

    server_thread
        .0
        .unwrap_or_else(|error| panic!("server flow failed: {error}"));
    server_thread
        .1
        .unwrap_or_else(|error| panic!("client flow failed: {error}"));
}

fn negotiate_results(
    server: &mut RaTlsHandle,
    client: &mut RaTlsHandle,
) -> (Result<(), String>, Result<(), String>) {
    let (server_stream, client_stream) = stream_pair();
    thread::scope(|scope| {
        let server_thread = scope.spawn(move || {
            rats_tls_negotiate(server, Box::new(server_stream))
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        let client_result = rats_tls_negotiate(client, Box::new(client_stream))
            .map(|_| ())
            .map_err(|error| error.to_string());
        let server_result = server_thread.join().expect("server thread panicked");
        (server_result, client_result)
    })
}

fn stream_pair() -> (UnixStream, UnixStream) {
    UnixStream::pair().expect("create Unix stream pair")
}

fn transmit_all(
    stream: &mut dyn ratls_api::tls_wrappers::TransportStream,
    mut data: &[u8],
) -> Result<(), String> {
    while !data.is_empty() {
        let written = rats_tls_transmit(stream, data).map_err(|error| error.to_string())?;
        if written == 0 {
            return Err("TLS stream returned a zero-length write".into());
        }
        data = &data[written..];
    }
    Ok(())
}

fn receive_exact(
    stream: &mut dyn ratls_api::tls_wrappers::TransportStream,
    len: usize,
) -> Result<Vec<u8>, String> {
    let mut data = vec![0; len];
    let mut offset = 0;
    while offset < data.len() {
        let read =
            rats_tls_receive(stream, &mut data[offset..]).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err(format!(
                "TLS stream reached EOF after {offset} of {} bytes",
                data.len()
            ));
        }
        offset += read;
    }
    Ok(data)
}

fn test_payload(seed: u8) -> Vec<u8> {
    (0..APPLICATION_DATA_SIZE)
        .map(|index| seed.wrapping_add((index % 251) as u8))
        .collect()
}

fn assert_mock_calls(
    calls: &MockCalls,
    expected_attester: usize,
    expected_verifier: usize,
    expected_callback: usize,
) {
    assert_eq!(
        calls.attester.load(Ordering::SeqCst),
        expected_attester,
        "unexpected attester call count"
    );
    assert_eq!(
        calls.verifier.load(Ordering::SeqCst),
        expected_verifier,
        "unexpected verifier call count"
    );
    assert_eq!(
        calls.callback.load(Ordering::SeqCst),
        expected_callback,
        "unexpected callback call count"
    );
}

fn test_ca(common_name: &str, serial: u32) -> TestCa {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).expect("load P-256");
    let key = PKey::from_ec_key(EcKey::generate(&group).expect("generate CA EC key"))
        .expect("create CA key");
    let mut name = X509NameBuilder::new().expect("create CA name");
    name.append_entry_by_text("CN", common_name)
        .expect("set CA common name");
    let name = name.build();

    let mut certificate = X509::builder().expect("create CA certificate");
    certificate.set_version(2).expect("set CA X.509 version");
    let serial = Asn1Integer::from_bn(BigNum::from_u32(serial).expect("create serial").as_ref())
        .expect("create ASN.1 serial");
    certificate
        .set_serial_number(&serial)
        .expect("set CA serial number");
    certificate
        .set_subject_name(&name)
        .expect("set CA subject name");
    certificate
        .set_issuer_name(&name)
        .expect("set CA issuer name");
    certificate.set_pubkey(&key).expect("set CA public key");
    certificate
        .set_not_before(
            Asn1Time::days_from_now(0)
                .expect("create notBefore")
                .as_ref(),
        )
        .expect("set CA notBefore");
    certificate
        .set_not_after(
            Asn1Time::days_from_now(30)
                .expect("create notAfter")
                .as_ref(),
        )
        .expect("set CA notAfter");
    certificate
        .append_extension(
            BasicConstraints::new()
                .critical()
                .ca()
                .build()
                .expect("create CA basic constraints"),
        )
        .expect("set CA basic constraints");
    certificate
        .append_extension(
            KeyUsage::new()
                .critical()
                .key_cert_sign()
                .crl_sign()
                .build()
                .expect("create CA key usage"),
        )
        .expect("set CA key usage");
    let subject_key_identifier = {
        let context = certificate.x509v3_context(None, None);
        SubjectKeyIdentifier::new()
            .build(&context)
            .expect("create CA subject key identifier")
    };
    certificate
        .append_extension(subject_key_identifier)
        .expect("set CA subject key identifier");
    certificate
        .sign(&key, MessageDigest::sha256())
        .expect("sign CA certificate");
    let certificate = certificate.build();

    TestCa {
        private_key_pem: key
            .private_key_to_pem_pkcs8()
            .expect("encode CA private key"),
        certificate_pem: certificate.to_pem().expect("encode CA certificate"),
    }
}
