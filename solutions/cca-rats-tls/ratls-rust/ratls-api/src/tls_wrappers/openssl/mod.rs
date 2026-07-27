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

use std::cmp::Ordering;
use std::io::{Read, Result as IoResult, Write};
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::slice;
use std::sync::{Arc, Mutex};

use foreign_types_shared::ForeignTypeRef;
use openssl::asn1::Asn1Time;
use openssl::error::ErrorStack;
use openssl::pkey::PKey;
use openssl::ssl::{
    ClientHelloResponse, ErrorCode, HandshakeError, MidHandshakeSslStream, Ssl, SslAcceptor,
    SslContextBuilder, SslMethod, SslStream, SslVerifyMode, SslVersion,
};
use openssl::x509::store::{X509Store, X509StoreBuilder};
use openssl::x509::verify::{X509CheckFlags, X509VerifyParamRef};
use openssl::x509::X509;

use crate::core::TlsVerifyConf;
use crate::crypto_wrappers::CertificateRole;
use crate::tls_wrappers::{
    ClientHelloData, TlsEvidenceBinding, TlsHandshake, TlsIdentity, TlsNegotiated, TlsPeerData,
    TlsWrapper, TransportStream,
};
use crate::RaTlsError;

extern "C" {
    fn SSL_CTX_set_cert_cb(
        ctx: *mut openssl_sys::SSL_CTX,
        cb: Option<unsafe extern "C" fn(*mut openssl_sys::SSL, *mut c_void) -> c_int>,
        arg: *mut c_void,
    );
    fn SSL_set_cert_cb(
        ssl: *mut openssl_sys::SSL,
        cb: Option<unsafe extern "C" fn(*mut openssl_sys::SSL, *mut c_void) -> c_int>,
        arg: *mut c_void,
    );
    fn SSL_CTX_set_msg_callback(
        ctx: *mut openssl_sys::SSL_CTX,
        cb: Option<
            unsafe extern "C" fn(
                c_int,
                c_int,
                c_int,
                *const c_void,
                usize,
                *mut openssl_sys::SSL,
                *mut c_void,
            ),
        >,
    );
    fn SSL_set_msg_callback(
        ssl: *mut openssl_sys::SSL,
        cb: Option<
            unsafe extern "C" fn(
                c_int,
                c_int,
                c_int,
                *const c_void,
                usize,
                *mut openssl_sys::SSL,
                *mut c_void,
            ),
        >,
    );
}

/// OpenSSL-backed TLS wrapper.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenSslTlsWrapper;

impl TlsWrapper for OpenSslTlsWrapper {
    fn name(&self) -> &'static str {
        "openssl"
    }

    fn validate_peer_verification(&self, verify: &TlsVerifyConf) -> Result<(), RaTlsError> {
        if verify.verify_peer_certificate {
            build_verify_store(verify)?;
        }
        Ok(())
    }

    fn start_server_handshake(
        &self,
        stream: Box<dyn TransportStream>,
        mutual: bool,
        verify: &TlsVerifyConf,
    ) -> Result<Box<dyn TlsHandshake>, RaTlsError> {
        let state = Arc::new(Mutex::new(ServerHandshakeState::WaitingForClientHello));
        let callback_state = Arc::clone(&state);

        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls_server())?;
        builder.set_min_proto_version(Some(SslVersion::TLS1_3))?;
        configure_server_verification(&mut builder, mutual, verify)?;
        builder.set_client_hello_callback(move |ssl, _alert| {
            let mut state = callback_state.lock().map_err(|_| ErrorStack::get())?;

            match &*state {
                ServerHandshakeState::WaitingForClientHello => {
                    let client_random = ssl
                        .client_hello_random()
                        .ok_or_else(ErrorStack::get)?
                        .to_vec();
                    let client_key_share = read_client_key_share_extension(ssl)?;
                    *state = ServerHandshakeState::ClientHello(ClientHelloData {
                        client_random,
                        client_key_share,
                    });
                    Ok(ClientHelloResponse::RETRY)
                }
                ServerHandshakeState::ClientHello(_) => Ok(ClientHelloResponse::RETRY),
                ServerHandshakeState::Identity { identity, .. } => {
                    attach_tls_identity_ref(ssl, identity)?;
                    Ok(ClientHelloResponse::SUCCESS)
                }
            }
        });
        let acceptor = builder.build();
        let stream = match acceptor.accept(stream) {
            Err(HandshakeError::Failure(stream))
                if stream.error().code() == ErrorCode::WANT_CLIENT_HELLO_CB =>
            {
                stream
            }
            Err(HandshakeError::SetupFailure(error)) => {
                return Err(RaTlsError::OpenSsl(error));
            }
            Err(HandshakeError::Failure(_)) | Err(HandshakeError::WouldBlock(_)) => {
                return Err(RaTlsError::InvalidData(
                    "OpenSSL failed before the server received ClientHello".into(),
                ));
            }
            Ok(_) => {
                return Err(RaTlsError::InvalidData(
                    "OpenSSL completed the server handshake before an identity was installed"
                        .into(),
                ));
            }
        };
        Ok(Box::new(OpenSslServerHandshake {
            stream: Some(stream),
            state,
            mutual,
        }))
    }

    fn start_client_handshake(
        &self,
        stream: Box<dyn TransportStream>,
        mutual: bool,
        verify: &TlsVerifyConf,
    ) -> Result<Box<dyn TlsHandshake>, RaTlsError> {
        start_client_handshake(stream, mutual, verify)
    }
}

struct OpenSslBoxStream {
    inner: SslStream<Box<dyn TransportStream>>,
}

enum ServerHandshakeState {
    WaitingForClientHello,
    ClientHello(ClientHelloData),
    Identity {
        identity: TlsIdentity,
        client_hello: ClientHelloData,
    },
}

struct OpenSslServerHandshake {
    stream: Option<MidHandshakeSslStream<Box<dyn TransportStream>>>,
    state: Arc<Mutex<ServerHandshakeState>>,
    mutual: bool,
}

struct ClientHandshakeState {
    client_hello: Option<ClientHelloData>,
    binding: Option<TlsEvidenceBinding>,
    identity: Option<TlsIdentity>,
    error: Option<String>,
}

struct OpenSslClientHandshake {
    stream: Option<SslStream<Box<dyn TransportStream>>>,
    state: Box<ClientHandshakeState>,
    mutual: bool,
    complete: bool,
}

impl Read for OpenSslBoxStream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        self.inner.read(buf)
    }
}

impl Write for OpenSslBoxStream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.inner.flush()
    }
}

impl TlsHandshake for OpenSslServerHandshake {
    fn evidence_binding(&self) -> Result<Option<TlsEvidenceBinding>, RaTlsError> {
        let state = self.state.lock().map_err(|_| {
            RaTlsError::InvalidData("OpenSSL server handshake state is poisoned".into())
        })?;
        match &*state {
            ServerHandshakeState::ClientHello(client_hello) => Ok(Some(TlsEvidenceBinding {
                nonce: client_hello.client_random.clone(),
                client_key_share: client_hello.client_key_share.clone(),
            })),
            ServerHandshakeState::WaitingForClientHello => Err(RaTlsError::InvalidData(
                "OpenSSL has not received ClientHello".into(),
            )),
            ServerHandshakeState::Identity { client_hello, .. } => Ok(Some(TlsEvidenceBinding {
                nonce: client_hello.client_random.clone(),
                client_key_share: client_hello.client_key_share.clone(),
            })),
        }
    }

    fn install_identity(&mut self, identity: TlsIdentity) -> Result<(), RaTlsError> {
        let mut state = self.state.lock().map_err(|_| {
            RaTlsError::InvalidData("OpenSSL server handshake state is poisoned".into())
        })?;
        let ServerHandshakeState::ClientHello(client_hello) = &*state else {
            return Err(RaTlsError::InvalidData(
                "server identity can only be installed after ClientHello".into(),
            ));
        };
        *state = ServerHandshakeState::Identity {
            identity,
            client_hello: client_hello.clone(),
        };
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<TlsNegotiated, RaTlsError> {
        {
            let state = self.state.lock().map_err(|_| {
                RaTlsError::InvalidData("OpenSSL server handshake state is poisoned".into())
            })?;
            if !matches!(&*state, ServerHandshakeState::Identity { .. }) {
                return Err(RaTlsError::InvalidData(
                    "server identity must be installed before finishing the handshake".into(),
                ));
            }
        }

        let stream = self.stream.take().ok_or_else(|| {
            RaTlsError::InvalidData("OpenSSL server handshake was already consumed".into())
        })?;
        let stream = stream.handshake().map_err(|error| match error {
            HandshakeError::SetupFailure(error) => RaTlsError::OpenSsl(error),
            HandshakeError::Failure(_) | HandshakeError::WouldBlock(_) => {
                RaTlsError::InvalidData("OpenSSL server handshake failed".into())
            }
        })?;
        let peer = if self.mutual {
            let client_key_share = {
                let state = self.state.lock().map_err(|_| {
                    RaTlsError::InvalidData("OpenSSL server handshake state is poisoned".into())
                })?;
                let ServerHandshakeState::Identity { client_hello, .. } = &*state else {
                    return Err(RaTlsError::InvalidData(
                        "server handshake lost ClientHello state".into(),
                    ));
                };
                client_hello.client_key_share.clone()
            };
            let nonce = read_server_random(stream.ssl())?;
            Some(read_peer_data(
                stream.ssl(),
                TlsEvidenceBinding {
                    nonce,
                    client_key_share,
                },
                CertificateRole::Client,
            )?)
        } else {
            None
        };
        Ok(TlsNegotiated {
            stream: Box::new(OpenSslBoxStream { inner: stream }),
            peer,
        })
    }
}

impl TlsHandshake for OpenSslClientHandshake {
    fn evidence_binding(&self) -> Result<Option<TlsEvidenceBinding>, RaTlsError> {
        if !self.mutual {
            return Ok(None);
        }
        self.state.binding.clone().map(Some).ok_or_else(|| {
            RaTlsError::InvalidData(
                "OpenSSL client handshake has not reached CertificateRequest".into(),
            )
        })
    }

    fn install_identity(&mut self, identity: TlsIdentity) -> Result<(), RaTlsError> {
        if !self.mutual {
            return Err(RaTlsError::InvalidData(
                "a non-mutual client handshake does not send a local identity".into(),
            ));
        }
        if self.state.binding.is_none() {
            return Err(RaTlsError::InvalidData(
                "client identity can only be installed after CertificateRequest".into(),
            ));
        }
        self.state.identity = Some(identity);
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<TlsNegotiated, RaTlsError> {
        let mut stream = self.stream.take().ok_or_else(|| {
            RaTlsError::InvalidData("OpenSSL client handshake was already consumed".into())
        })?;
        if !self.complete {
            if self.state.identity.is_none() {
                return Err(RaTlsError::InvalidData(
                    "client identity must be installed before finishing mutual TLS".into(),
                ));
            }
            stream.connect().map_err(|error| {
                RaTlsError::InvalidData(
                    self.state
                        .error
                        .clone()
                        .unwrap_or_else(|| format!("OpenSSL client handshake failed: {error}")),
                )
            })?;
        }
        unsafe { clear_handshake_callbacks(stream.ssl().as_ptr()) };
        let client_hello = self.state.client_hello.as_ref().ok_or_else(|| {
            RaTlsError::InvalidData("OpenSSL client handshake lost ClientHello state".into())
        })?;
        let peer = read_peer_data(
            stream.ssl(),
            TlsEvidenceBinding {
                nonce: client_hello.client_random.clone(),
                client_key_share: client_hello.client_key_share.clone(),
            },
            CertificateRole::Server,
        )?;
        Ok(TlsNegotiated {
            stream: Box::new(OpenSslBoxStream { inner: stream }),
            peer: Some(peer),
        })
    }
}

fn read_peer_data(
    ssl: &openssl::ssl::SslRef,
    evidence_binding: TlsEvidenceBinding,
    role: CertificateRole,
) -> Result<TlsPeerData, RaTlsError> {
    let certificate = ssl
        .peer_certificate()
        .ok_or_else(|| RaTlsError::InvalidData("peer certificate is missing".into()))?;
    validate_peer_certificate_basics(&certificate, role)?;
    Ok(TlsPeerData {
        certificate_der: certificate.to_der()?,
        evidence_binding,
    })
}

fn validate_peer_certificate_basics(
    certificate: &X509,
    role: CertificateRole,
) -> Result<(), RaTlsError> {
    let now = Asn1Time::days_from_now(0)?;
    if certificate.not_before().compare(now.as_ref())? == Ordering::Greater {
        return Err(RaTlsError::InvalidData(
            "peer certificate is not yet valid".into(),
        ));
    }
    if certificate.not_after().compare(now.as_ref())? == Ordering::Less {
        return Err(RaTlsError::InvalidData(
            "peer certificate has expired".into(),
        ));
    }

    let extension_flags = unsafe { openssl_sys::X509_get_extension_flags(certificate.as_ptr()) };
    if extension_flags & openssl_sys::EXFLAG_INVALID != 0 {
        return Err(RaTlsError::InvalidData(
            "peer certificate contains invalid extensions".into(),
        ));
    }
    if extension_flags & openssl_sys::EXFLAG_BCONS == 0
        || extension_flags & openssl_sys::EXFLAG_CA != 0
    {
        return Err(RaTlsError::InvalidData(
            "peer leaf certificate must contain BasicConstraints CA:FALSE".into(),
        ));
    }

    let key_usage = unsafe { openssl_sys::X509_get_key_usage(certificate.as_ptr()) };
    if extension_flags & openssl_sys::EXFLAG_KUSAGE == 0
        || key_usage & openssl_sys::X509v3_KU_DIGITAL_SIGNATURE == 0
    {
        return Err(RaTlsError::InvalidData(
            "peer leaf certificate must allow digitalSignature".into(),
        ));
    }

    let extended_key_usage =
        unsafe { openssl_sys::X509_get_extended_key_usage(certificate.as_ptr()) };
    let required_eku = match role {
        CertificateRole::Server => openssl_sys::XKU_SSL_SERVER,
        CertificateRole::Client => openssl_sys::XKU_SSL_CLIENT,
    };
    if extension_flags & openssl_sys::EXFLAG_XKUSAGE == 0 || extended_key_usage & required_eku == 0
    {
        return Err(RaTlsError::InvalidData(format!(
            "peer leaf certificate does not allow {}",
            match role {
                CertificateRole::Server => "serverAuth",
                CertificateRole::Client => "clientAuth",
            }
        )));
    }

    if certificate.subject_name().to_der()? == certificate.issuer_name().to_der()? {
        let public_key = certificate.public_key()?;
        if !certificate.verify(public_key.as_ref())? {
            return Err(RaTlsError::InvalidData(
                "peer self-signed certificate has an invalid signature".into(),
            ));
        }
    }
    Ok(())
}

fn read_server_random(ssl: &openssl::ssl::SslRef) -> Result<Vec<u8>, RaTlsError> {
    let mut random = vec![0_u8; 64];
    let len = ssl.server_random(&mut random);
    if len == 0 {
        return Err(RaTlsError::InvalidData(
            "failed to obtain TLS server random".into(),
        ));
    }
    random.truncate(len);
    Ok(random)
}

fn read_client_key_share_extension(ssl: &openssl::ssl::SslRef) -> Result<Vec<u8>, ErrorStack> {
    const TLS_EXTENSION_KEY_SHARE: u32 = 51;

    let mut data = ptr::null();
    let mut len = 0usize;
    let result = unsafe {
        openssl_sys::SSL_client_hello_get0_ext(
            ssl.as_ptr(),
            TLS_EXTENSION_KEY_SHARE,
            &mut data,
            &mut len,
        )
    };
    if result != 1 || data.is_null() {
        return Err(ErrorStack::get());
    }
    Ok(unsafe { slice::from_raw_parts(data, len) }.to_vec())
}

fn start_client_handshake(
    stream: Box<dyn TransportStream>,
    mutual: bool,
    verify: &TlsVerifyConf,
) -> Result<Box<dyn TlsHandshake>, RaTlsError> {
    crate::rtls_debug!("OpenSSL client handshake started");
    let mut state = Box::new(ClientHandshakeState {
        client_hello: None,
        binding: None,
        identity: None,
        error: None,
    });
    let mut builder = SslContextBuilder::new(SslMethod::tls_client())?;
    builder.set_min_proto_version(Some(SslVersion::TLS1_3))?;
    configure_client_verification(&mut builder, verify)?;
    unsafe {
        set_client_message_callback(builder.as_ptr(), state.as_mut());
    }
    if mutual {
        unsafe {
            SSL_CTX_set_cert_cb(
                builder.as_ptr(),
                Some(client_cert_callback),
                (state.as_mut() as *mut ClientHandshakeState).cast::<c_void>(),
            );
        }
    }

    let context = builder.build();
    let mut ssl = Ssl::new(&context)?;
    if verify.verify_peer_certificate {
        if let Some(name) = verify
            .expected_peer_name
            .as_deref()
            .filter(|name| !name.is_empty())
        {
            if name.parse::<std::net::IpAddr>().is_err() {
                ssl.set_hostname(name)?;
            }
        }
    }
    let mut stream = SslStream::new(ssl, stream)?;
    let complete = match stream.connect() {
        Ok(()) if !mutual => true,
        Ok(()) => {
            unsafe { clear_handshake_callbacks(stream.ssl().as_ptr()) };
            return Err(RaTlsError::InvalidData(
                "mutual TLS server did not request a client certificate".into(),
            ));
        }
        Err(error)
            if mutual
                && error.code() == ErrorCode::from_raw(openssl_sys::SSL_ERROR_WANT_X509_LOOKUP)
                && state.binding.is_some() =>
        {
            false
        }
        Err(error) => {
            return Err(RaTlsError::InvalidData(state.error.clone().unwrap_or_else(
                || format!("OpenSSL client handshake failed: {error}"),
            )));
        }
    };

    Ok(Box::new(OpenSslClientHandshake {
        stream: Some(stream),
        state,
        mutual,
        complete,
    }))
}

unsafe fn set_client_message_callback(
    context: *mut openssl_sys::SSL_CTX,
    state: &mut ClientHandshakeState,
) {
    const SSL_CTRL_SET_MSG_CALLBACK_ARG: c_int = 16;
    SSL_CTX_set_msg_callback(context, Some(client_message_callback));
    openssl_sys::SSL_CTX_ctrl(
        context,
        SSL_CTRL_SET_MSG_CALLBACK_ARG,
        0,
        (state as *mut ClientHandshakeState).cast::<c_void>(),
    );
}

unsafe fn clear_handshake_callbacks(ssl: *mut openssl_sys::SSL) {
    const SSL_CTRL_SET_MSG_CALLBACK_ARG: c_int = 16;
    SSL_set_msg_callback(ssl, None);
    openssl_sys::SSL_ctrl(ssl, SSL_CTRL_SET_MSG_CALLBACK_ARG, 0, std::ptr::null_mut());
    SSL_set_cert_cb(ssl, None, std::ptr::null_mut());
}

unsafe extern "C" fn client_message_callback(
    write_p: c_int,
    _version: c_int,
    content_type: c_int,
    buffer: *const c_void,
    len: usize,
    _ssl: *mut openssl_sys::SSL,
    arg: *mut c_void,
) {
    const SSL3_RT_HANDSHAKE: c_int = 22;
    const SSL3_MT_CLIENT_HELLO: u8 = 1;

    if write_p != 1
        || content_type != SSL3_RT_HANDSHAKE
        || buffer.is_null()
        || arg.is_null()
        || len < 4
    {
        return;
    }
    let message = slice::from_raw_parts(buffer.cast::<u8>(), len);
    if message[0] != SSL3_MT_CLIENT_HELLO {
        return;
    }
    let state = &mut *(arg as *mut ClientHandshakeState);
    match parse_client_hello(message) {
        Ok(client_hello) => state.client_hello = Some(client_hello),
        Err(err) => state.error = Some(err.to_string()),
    }
}

unsafe extern "C" fn client_cert_callback(ssl: *mut openssl_sys::SSL, arg: *mut c_void) -> c_int {
    if ssl.is_null() || arg.is_null() {
        return 0;
    }
    let state = &mut *(arg as *mut ClientHandshakeState);
    if let Some(identity) = state.identity.as_ref() {
        return match attach_tls_identity(ssl, identity) {
            Ok(()) => 1,
            Err(err) => {
                state.error = Some(err.to_string());
                0
            }
        };
    }

    let result = (|| {
        let client_hello = state.client_hello.as_ref().ok_or_else(|| {
            RaTlsError::InvalidData("outgoing ClientHello was not captured".into())
        })?;
        Ok::<TlsEvidenceBinding, RaTlsError>(TlsEvidenceBinding {
            nonce: server_random(ssl)?,
            client_key_share: client_hello.client_key_share.clone(),
        })
    })();
    match result {
        Ok(binding) => {
            state.binding = Some(binding);
            -1
        }
        Err(err) => {
            crate::rtls_err!("failed to prepare client RA-TLS identity binding: {err}");
            state.error = Some(err.to_string());
            0
        }
    }
}

unsafe fn attach_tls_identity(
    ssl: *mut openssl_sys::SSL,
    identity: &TlsIdentity,
) -> Result<(), RaTlsError> {
    let ssl = openssl::ssl::SslRef::from_ptr_mut(ssl);
    attach_tls_identity_ref(ssl, identity)?;
    Ok(())
}

fn attach_tls_identity_ref(
    ssl: &mut openssl::ssl::SslRef,
    identity: &TlsIdentity,
) -> Result<(), ErrorStack> {
    let (leaf, chain) = identity
        .certificate_chain_pem
        .split_first()
        .ok_or_else(ErrorStack::get)?;
    let certificate = X509::from_pem(leaf)?;
    let private_key = PKey::private_key_from_pkcs8(&identity.private_key_pkcs8)?;
    ssl.set_certificate(&certificate)?;
    ssl.set_private_key(&private_key)?;
    for certificate in chain {
        ssl.add_chain_cert(X509::from_pem(certificate)?)?;
    }
    Ok(())
}

fn configure_server_verification(
    builder: &mut openssl::ssl::SslAcceptorBuilder,
    mutual: bool,
    verify: &TlsVerifyConf,
) -> Result<(), RaTlsError> {
    if !mutual {
        return Ok(());
    }
    if verify.verify_peer_certificate {
        builder.set_verify_cert_store(build_verify_store(verify)?)?;
        configure_expected_peer_name(builder.verify_param_mut(), verify)?;
        builder.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
    } else {
        builder.set_verify_callback(
            SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT,
            basic_certificate_verify_callback,
        );
    }
    Ok(())
}

fn configure_client_verification(
    builder: &mut SslContextBuilder,
    verify: &TlsVerifyConf,
) -> Result<(), RaTlsError> {
    if verify.verify_peer_certificate {
        builder.set_verify_cert_store(build_verify_store(verify)?)?;
        configure_expected_peer_name(builder.verify_param_mut(), verify)?;
        builder.set_verify(SslVerifyMode::PEER);
    } else {
        builder.set_verify_callback(SslVerifyMode::PEER, basic_certificate_verify_callback);
    }
    Ok(())
}

fn basic_certificate_verify_callback(
    preverify_ok: bool,
    context: &mut openssl::x509::X509StoreContextRef,
) -> bool {
    preverify_ok || is_ignored_trust_error(context.error().as_raw())
}

fn is_ignored_trust_error(error: c_int) -> bool {
    matches!(
        error,
        openssl_sys::X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT
            | openssl_sys::X509_V_ERR_DEPTH_ZERO_SELF_SIGNED_CERT
            | openssl_sys::X509_V_ERR_SELF_SIGNED_CERT_IN_CHAIN
            | openssl_sys::X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY
            | openssl_sys::X509_V_ERR_UNABLE_TO_VERIFY_LEAF_SIGNATURE
            | openssl_sys::X509_V_ERR_CERT_UNTRUSTED
    )
}

fn build_verify_store(verify: &TlsVerifyConf) -> Result<X509Store, RaTlsError> {
    let mut store = X509StoreBuilder::new()?;
    if verify.use_system_ca {
        store.set_default_paths().map_err(|err| {
            RaTlsError::InvalidData(format!("failed to load the OpenSSL system CA store: {err}"))
        })?;
    }
    if !verify.trusted_ca_chain.is_empty() {
        let certificates = X509::stack_from_pem(&verify.trusted_ca_chain).map_err(|err| {
            RaTlsError::InvalidArgument(format!("invalid trusted CA PEM bundle: {err}"))
        })?;
        if certificates.is_empty() {
            return Err(RaTlsError::InvalidArgument(
                "trusted CA PEM bundle contains no certificates".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for certificate in certificates {
            if seen.insert(certificate.to_der()?) {
                store.add_cert(certificate)?;
            }
        }
    }
    Ok(store.build())
}

fn configure_expected_peer_name(
    param: &mut X509VerifyParamRef,
    verify: &TlsVerifyConf,
) -> Result<(), RaTlsError> {
    let Some(name) = verify
        .expected_peer_name
        .as_deref()
        .filter(|name| !name.is_empty())
    else {
        return Ok(());
    };
    param.set_hostflags(X509CheckFlags::NO_PARTIAL_WILDCARDS);
    match name.parse::<std::net::IpAddr>() {
        Ok(ip) => param.set_ip(ip)?,
        Err(_) => param.set_host(name)?,
    }
    Ok(())
}

fn server_random(ssl: *mut openssl_sys::SSL) -> Result<Vec<u8>, RaTlsError> {
    let mut random = vec![0_u8; 64];
    let len = unsafe { openssl_sys::SSL_get_server_random(ssl, random.as_mut_ptr(), random.len()) };
    if len == 0 {
        return Err(RaTlsError::InvalidData(
            "failed to obtain TLS server random".into(),
        ));
    }
    random.truncate(len);
    Ok(random)
}

fn parse_client_hello(message: &[u8]) -> Result<ClientHelloData, RaTlsError> {
    let mut offset = 0usize;
    if take_u8(message, &mut offset)? != 1 {
        return Err(RaTlsError::InvalidData(
            "TLS handshake message is not ClientHello".into(),
        ));
    }
    let declared_len = take_u24(message, &mut offset)?;
    if declared_len != message.len().saturating_sub(4) {
        return Err(RaTlsError::InvalidData(
            "ClientHello handshake length is inconsistent".into(),
        ));
    }
    take_bytes(message, &mut offset, 2)?;
    let client_random = take_bytes(message, &mut offset, 32)?.to_vec();
    let session_id_len = take_u8(message, &mut offset)? as usize;
    take_bytes(message, &mut offset, session_id_len)?;
    let cipher_suites_len = take_u16(message, &mut offset)? as usize;
    take_bytes(message, &mut offset, cipher_suites_len)?;
    let compression_methods_len = take_u8(message, &mut offset)? as usize;
    take_bytes(message, &mut offset, compression_methods_len)?;
    let extensions_len = take_u16(message, &mut offset)? as usize;
    let extensions = take_bytes(message, &mut offset, extensions_len)?;
    if offset != message.len() {
        return Err(RaTlsError::InvalidData(
            "ClientHello has trailing bytes after extensions".into(),
        ));
    }

    let mut extension_offset = 0usize;
    while extension_offset < extensions.len() {
        let extension_type = take_u16(extensions, &mut extension_offset)?;
        let extension_len = take_u16(extensions, &mut extension_offset)? as usize;
        let extension = take_bytes(extensions, &mut extension_offset, extension_len)?;
        if extension_type == 51 {
            validate_client_key_share_extension(extension)?;
            return Ok(ClientHelloData {
                client_random,
                client_key_share: extension.to_vec(),
            });
        }
    }
    Err(RaTlsError::InvalidData(
        "ClientHello does not contain a key_share extension".into(),
    ))
}

fn validate_client_key_share_extension(extension: &[u8]) -> Result<(), RaTlsError> {
    let mut offset = 0usize;
    let shares_len = take_u16(extension, &mut offset)? as usize;
    let shares = take_bytes(extension, &mut offset, shares_len)?;
    if offset != extension.len() {
        return Err(RaTlsError::InvalidData(
            "ClientHello key_share extension has trailing bytes".into(),
        ));
    }
    let mut share_offset = 0usize;
    let mut count = 0usize;
    while share_offset < shares.len() {
        take_u16(shares, &mut share_offset)?;
        let key_exchange_len = take_u16(shares, &mut share_offset)? as usize;
        if key_exchange_len == 0 {
            return Err(RaTlsError::InvalidData(
                "ClientHello key_share entry is empty".into(),
            ));
        }
        take_bytes(shares, &mut share_offset, key_exchange_len)?;
        count += 1;
    }
    if count == 0 {
        return Err(RaTlsError::InvalidData(
            "ClientHello key_share extension contains no entries".into(),
        ));
    }
    Ok(())
}

fn take_u8(input: &[u8], offset: &mut usize) -> Result<u8, RaTlsError> {
    Ok(take_bytes(input, offset, 1)?[0])
}

fn take_u16(input: &[u8], offset: &mut usize) -> Result<u16, RaTlsError> {
    let bytes = take_bytes(input, offset, 2)?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn take_u24(input: &[u8], offset: &mut usize) -> Result<usize, RaTlsError> {
    let bytes = take_bytes(input, offset, 3)?;
    Ok(((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | bytes[2] as usize)
}

fn take_bytes<'a>(input: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8], RaTlsError> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| RaTlsError::InvalidData("TLS handshake field offset overflow".into()))?;
    if end > input.len() {
        return Err(RaTlsError::InvalidData(
            "TLS handshake field exceeds message length".into(),
        ));
    }
    let bytes = &input[*offset..end];
    *offset = end;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto_wrappers::openssl::OpenSslCrypto;
    use crate::crypto_wrappers::{
        CertAlgorithm, CertificateRole, CryptoWrapper, RatlsCertificateInfo,
    };
    use openssl::asn1::{Asn1Integer, Asn1Time};
    use openssl::bn::BigNum;
    use openssl::ec::{EcGroup, EcKey};
    use openssl::hash::MessageDigest;
    use openssl::nid::Nid;
    use openssl::pkey::{PKey, Private};
    use openssl::x509::extension::{
        BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectKeyIdentifier,
    };
    use openssl::x509::X509NameBuilder;
    use std::os::unix::net::UnixStream;
    use std::thread;

    #[test]
    fn validates_a_ca_signed_server_certificate_and_dns_name() {
        let (root_key, root) = root_ca();
        let server_identity = identity(
            &root_key,
            &root,
            CertificateRole::Server,
            &["DNS:server.example.com".into()],
        );
        let trusted_root = root.to_pem().unwrap();
        let server_verify = TlsVerifyConf::default();
        let client_verify = TlsVerifyConf {
            verify_peer_certificate: true,
            trusted_ca_chain: trusted_root,
            expected_peer_name: Some("server.example.com".into()),
            ..Default::default()
        };
        negotiate(false, server_identity, None, server_verify, client_verify);
    }

    #[test]
    fn validates_both_ca_signed_certificates_in_mutual_tls() {
        let (root_key, root) = root_ca();
        let server_identity = identity(
            &root_key,
            &root,
            CertificateRole::Server,
            &["DNS:server.example.com".into()],
        );
        let client_identity = identity(&root_key, &root, CertificateRole::Client, &[]);
        let trusted_root = root.to_pem().unwrap();
        let server_verify = TlsVerifyConf {
            verify_peer_certificate: true,
            trusted_ca_chain: trusted_root.clone(),
            ..Default::default()
        };
        let client_verify = TlsVerifyConf {
            verify_peer_certificate: true,
            trusted_ca_chain: trusted_root,
            expected_peer_name: Some("server.example.com".into()),
            ..Default::default()
        };
        negotiate(
            true,
            server_identity,
            Some(client_identity),
            server_verify,
            client_verify,
        );
    }

    #[test]
    fn mutual_tls_rejects_a_client_signed_by_an_untrusted_ca() {
        let (server_root_key, server_root) = root_ca();
        let (client_root_key, client_root) = root_ca();
        let server_identity =
            identity(&server_root_key, &server_root, CertificateRole::Server, &[]);
        let client_identity =
            identity(&client_root_key, &client_root, CertificateRole::Client, &[]);
        let server_verify = TlsVerifyConf {
            verify_peer_certificate: true,
            trusted_ca_chain: server_root.to_pem().unwrap(),
            ..Default::default()
        };
        let client_verify = TlsVerifyConf {
            verify_peer_certificate: true,
            trusted_ca_chain: server_root.to_pem().unwrap(),
            ..Default::default()
        };
        let (server_stream, client_stream) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let wrapper = OpenSslTlsWrapper;
            let mut handshake = wrapper
                .start_server_handshake(Box::new(server_stream), true, &server_verify)
                .unwrap();
            handshake.install_identity(server_identity).unwrap();
            handshake.finish().is_err()
        });

        let wrapper = OpenSslTlsWrapper;
        let mut handshake = wrapper
            .start_client_handshake(Box::new(client_stream), true, &client_verify)
            .unwrap();
        handshake.install_identity(client_identity).unwrap();
        let _client_result = handshake.finish();
        assert!(server.join().unwrap());
    }

    #[test]
    fn disabled_trust_verification_accepts_a_valid_self_signed_peer_certificate() {
        let identity = self_signed_identity(SelfSignedCertificateKind::Valid);
        let certificate = X509::from_pem(&identity.certificate_chain_pem[0]).unwrap();
        validate_peer_certificate_basics(&certificate, CertificateRole::Server).unwrap();
    }

    #[test]
    fn disabled_trust_verification_still_rejects_an_expired_peer_certificate() {
        assert_invalid_basic_certificate(
            SelfSignedCertificateKind::Expired,
            CertificateRole::Server,
        );
    }

    #[test]
    fn disabled_trust_verification_only_ignores_trust_anchor_errors() {
        assert!(is_ignored_trust_error(
            openssl_sys::X509_V_ERR_DEPTH_ZERO_SELF_SIGNED_CERT
        ));
        assert!(is_ignored_trust_error(
            openssl_sys::X509_V_ERR_UNABLE_TO_GET_ISSUER_CERT_LOCALLY
        ));
        assert!(!is_ignored_trust_error(
            openssl_sys::X509_V_ERR_CERT_HAS_EXPIRED
        ));
        assert!(!is_ignored_trust_error(
            openssl_sys::X509_V_ERR_CERT_SIGNATURE_FAILURE
        ));
        assert!(!is_ignored_trust_error(
            openssl_sys::X509_V_ERR_INVALID_PURPOSE
        ));
    }

    #[test]
    fn disabled_trust_verification_still_rejects_the_wrong_eku() {
        assert_invalid_basic_certificate(
            SelfSignedCertificateKind::ClientEku,
            CertificateRole::Server,
        );
        assert_invalid_basic_certificate(SelfSignedCertificateKind::Valid, CertificateRole::Client);
    }

    #[test]
    fn disabled_trust_verification_still_requires_digital_signature_key_usage() {
        assert_invalid_basic_certificate(
            SelfSignedCertificateKind::MissingKeyUsage,
            CertificateRole::Server,
        );
    }

    #[test]
    fn disabled_trust_verification_still_requires_leaf_basic_constraints() {
        assert_invalid_basic_certificate(
            SelfSignedCertificateKind::MissingBasicConstraints,
            CertificateRole::Server,
        );
    }

    #[test]
    fn disabled_trust_verification_still_rejects_an_invalid_self_signature() {
        assert_invalid_basic_certificate(
            SelfSignedCertificateKind::InvalidSignature,
            CertificateRole::Server,
        );
    }

    fn negotiate(
        mutual: bool,
        server_identity: TlsIdentity,
        client_identity: Option<TlsIdentity>,
        server_verify: TlsVerifyConf,
        client_verify: TlsVerifyConf,
    ) {
        let (server_stream, client_stream) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let wrapper = OpenSslTlsWrapper;
            let mut handshake = wrapper
                .start_server_handshake(Box::new(server_stream), mutual, &server_verify)
                .unwrap();
            handshake.install_identity(server_identity).unwrap();
            handshake.finish().unwrap();
        });

        let wrapper = OpenSslTlsWrapper;
        let mut handshake = wrapper
            .start_client_handshake(Box::new(client_stream), mutual, &client_verify)
            .unwrap();
        if let Some(identity) = client_identity {
            handshake.install_identity(identity).unwrap();
        }
        handshake.finish().unwrap();
        server.join().unwrap();
    }

    fn assert_invalid_basic_certificate(kind: SelfSignedCertificateKind, role: CertificateRole) {
        let identity = self_signed_identity(kind);
        let certificate = X509::from_pem(&identity.certificate_chain_pem[0]).unwrap();
        assert!(validate_peer_certificate_basics(&certificate, role).is_err());
    }

    fn identity(
        root_key: &PKey<Private>,
        root: &X509,
        role: CertificateRole,
        subject_alt_names: &[String],
    ) -> TlsIdentity {
        let crypto = OpenSslCrypto;
        let issuer = crypto
            .prepare_certificate_issuer(
                &root_key.private_key_to_pem_pkcs8().unwrap(),
                &root.to_pem().unwrap(),
            )
            .unwrap();
        let mut leaf = crypto
            .generate_private_key(CertAlgorithm::Ecc256Sha256)
            .unwrap();
        crypto
            .generate_ra_certificate(
                &mut leaf,
                RatlsCertificateInfo {
                    organization: "Test",
                    common_name: "RA-TLS",
                    evidence_buffer: None,
                    issuer: Some(&issuer),
                    role,
                    subject_alt_names,
                },
            )
            .unwrap();
        let mut certificate_chain_pem = vec![leaf.cert.unwrap()];
        certificate_chain_pem.extend(issuer.certificate_chain_pem);
        TlsIdentity {
            certificate_chain_pem,
            private_key_pkcs8: leaf.private_key,
        }
    }

    #[derive(Clone, Copy)]
    enum SelfSignedCertificateKind {
        Valid,
        Expired,
        ClientEku,
        MissingKeyUsage,
        MissingBasicConstraints,
        InvalidSignature,
    }

    fn self_signed_identity(kind: SelfSignedCertificateKind) -> TlsIdentity {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "RATS-TLS Self-Signed Test")
            .unwrap();
        let name = name.build();
        let mut certificate = X509::builder().unwrap();
        certificate.set_version(2).unwrap();
        let serial = Asn1Integer::from_bn(BigNum::from_u32(2).unwrap().as_ref()).unwrap();
        certificate.set_serial_number(&serial).unwrap();
        certificate.set_subject_name(&name).unwrap();
        certificate.set_issuer_name(&name).unwrap();
        certificate.set_pubkey(&key).unwrap();
        if matches!(kind, SelfSignedCertificateKind::Expired) {
            certificate
                .set_not_before(Asn1Time::from_unix(1).unwrap().as_ref())
                .unwrap();
            certificate
                .set_not_after(Asn1Time::from_unix(2).unwrap().as_ref())
                .unwrap();
        } else {
            certificate
                .set_not_before(Asn1Time::days_from_now(0).unwrap().as_ref())
                .unwrap();
            certificate
                .set_not_after(Asn1Time::days_from_now(30).unwrap().as_ref())
                .unwrap();
        }
        if !matches!(kind, SelfSignedCertificateKind::MissingBasicConstraints) {
            certificate
                .append_extension(BasicConstraints::new().critical().build().unwrap())
                .unwrap();
        }
        if !matches!(kind, SelfSignedCertificateKind::MissingKeyUsage) {
            certificate
                .append_extension(
                    KeyUsage::new()
                        .critical()
                        .digital_signature()
                        .build()
                        .unwrap(),
                )
                .unwrap();
        }
        let mut extended_key_usage = ExtendedKeyUsage::new();
        if matches!(kind, SelfSignedCertificateKind::ClientEku) {
            extended_key_usage.client_auth();
        } else {
            extended_key_usage.server_auth();
        }
        certificate
            .append_extension(extended_key_usage.build().unwrap())
            .unwrap();
        if matches!(kind, SelfSignedCertificateKind::InvalidSignature) {
            let other_key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
            certificate
                .sign(&other_key, MessageDigest::sha256())
                .unwrap();
        } else {
            certificate.sign(&key, MessageDigest::sha256()).unwrap();
        }
        TlsIdentity {
            certificate_chain_pem: vec![certificate.build().to_pem().unwrap()],
            private_key_pkcs8: key.private_key_to_pkcs8().unwrap(),
        }
    }

    fn root_ca() -> (PKey<Private>, X509) {
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
        let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "RATS-TLS Test Root")
            .unwrap();
        let name = name.build();
        let mut certificate = X509::builder().unwrap();
        certificate.set_version(2).unwrap();
        let serial = Asn1Integer::from_bn(BigNum::from_u32(1).unwrap().as_ref()).unwrap();
        certificate.set_serial_number(&serial).unwrap();
        certificate.set_subject_name(&name).unwrap();
        certificate.set_issuer_name(&name).unwrap();
        certificate.set_pubkey(&key).unwrap();
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
        let subject_key_identifier = {
            let context = certificate.x509v3_context(None, None);
            SubjectKeyIdentifier::new().build(&context).unwrap()
        };
        certificate
            .append_extension(subject_key_identifier)
            .unwrap();
        certificate.sign(&key, MessageDigest::sha256()).unwrap();
        (key, certificate.build())
    }
}
