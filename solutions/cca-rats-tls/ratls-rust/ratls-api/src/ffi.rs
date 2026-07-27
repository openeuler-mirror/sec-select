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

use std::cell::RefCell;
use std::ffi::c_void;
use std::ffi::CString;
use std::net::TcpStream;
use std::os::fd::FromRawFd;
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::slice;

use crate::api::{
    rats_tls_init, rats_tls_negotiate, rats_tls_receive, rats_tls_set_verification_callback,
    rats_tls_transmit, MAX_CUSTOM_CLAIMS, MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH,
    MAX_CUSTOM_CLAIM_NAME_LENGTH, MAX_CUSTOM_CLAIM_VALUE_LENGTH, MAX_EXPECTED_PEER_NAME_LENGTH,
    MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE, MAX_ISSUER_PRIVATE_KEY_PEM_SIZE, MAX_SUBJECT_ALT_NAMES,
    MAX_SUBJECT_ALT_NAME_LENGTH, MAX_TRUSTED_CA_CHAIN_PEM_SIZE,
};
use crate::attesters::AttesterPlugin;
use crate::core::dice::CustomClaim;
use crate::core::{CertificateConf, RaTlsConf, RaTlsHandle as InnerHandle, TlsVerifyConf};
use crate::crypto_wrappers::CertAlgorithm;
use crate::crypto_wrappers::CryptoWrapperEnum;
use crate::logger::{set_log_level, LogLevel};
use crate::tls_wrappers::{TlsWrapperEnum, TransportStream};
use crate::verifiers::VerifierPlugin;

/// C ABI success return code.
pub const RATLS_SUCCESS: i32 = 0;
/// C ABI error code for invalid input arguments.
pub const RATLS_INVALID_ARGUMENT: i32 = -1;
/// C ABI error code for handle initialization failures.
pub const RATLS_INIT_ERROR: i32 = -2;
/// C ABI error code for TLS negotiation failures.
pub const RATLS_NEGOTIATE_ERROR: i32 = -3;
/// C ABI error code for transmit failures.
pub const RATLS_TRANSMIT_ERROR: i32 = -4;
/// C ABI error code for receive failures.
pub const RATLS_RECEIVE_ERROR: i32 = -5;
/// C ABI error code for system-level failures.
pub const RATLS_SYSTEM_ERROR: i32 = -6;
/// C ABI error code for Rust panics caught at the FFI boundary.
pub const RATLS_PANIC: i32 = -7;

/// C-compatible RSA-3072/SHA-256 certificate algorithm value.
pub const RATLS_CERT_ALGO_RSA_3072_SHA256: u32 = 0;
/// C-compatible ECC P-256/SHA-256 certificate algorithm value.
pub const RATLS_CERT_ALGO_ECC_256_SHA256: u32 = 1;
/// C-compatible certificate algorithm sentinel. This is not a selectable algorithm.
pub const RATLS_CERT_ALGO_MAX: u32 = 2;
/// C-compatible default certificate algorithm value. Rust resolves this to ECC P-256/SHA-256.
pub const RATLS_CERT_ALGO_DEFAULT: u32 = 3;

/// C-compatible DEBUG log level.
pub const RATLS_LOG_LEVEL_DEBUG: u32 = 0;
/// C-compatible INFO log level.
pub const RATLS_LOG_LEVEL_INFO: u32 = 1;
/// C-compatible WARN log level.
pub const RATLS_LOG_LEVEL_WARN: u32 = 2;
/// C-compatible ERROR log level.
pub const RATLS_LOG_LEVEL_ERROR: u32 = 3;
/// C-compatible FATAL log level.
pub const RATLS_LOG_LEVEL_FATAL: u32 = 4;
/// C-compatible disabled log level.
pub const RATLS_LOG_LEVEL_NONE: u32 = 5;

/// Do not configure a local attester.
pub const RATLS_ATTESTER_NONE: u32 = 0;
/// Select the built-in CCA attester.
pub const RATLS_ATTESTER_CCA: u32 = 1;
/// Do not configure a peer verifier.
pub const RATLS_VERIFIER_NONE: u32 = 0;
/// Select the built-in CCA verifier.
pub const RATLS_VERIFIER_CCA: u32 = 1;

const MAX_WRAPPER_NAME_LENGTH: usize = 64;

/// C-compatible RATS-TLS configuration.
///
/// Zero-valued adapter fields disable the corresponding adapter.
#[repr(C)]
pub struct RatlsConf {
    /// Size of this structure, initialized by [`ratls_conf_init`].
    pub struct_size: usize,
    /// Built-in attester selector.
    pub attester_type: u32,
    /// Built-in verifier selector.
    pub verifier_type: u32,
    /// Optional TLS wrapper implementation name.
    pub tls_type: *const c_char,
    /// Optional crypto wrapper implementation name.
    pub crypto_type: *const c_char,
    /// Certificate private key algorithm. Zero-initialized configs select RSA for C compatibility;
    /// use [`RATLS_CERT_ALGO_DEFAULT`] to request the Rust default.
    pub cert_algo: u32,
    /// Non-zero enables mutual attestation.
    pub mutual: u8,
    /// Non-zero configures this endpoint as the TLS server.
    pub server: u8,
    /// Optional application-defined claims to bind into generated evidence.
    pub custom_claims: *const RatlsCustomClaim,
    /// Number of entries available at `custom_claims`.
    pub custom_claims_len: usize,
    /// Dynamic certificate signing configuration.
    pub certificate: RatlsCertificateConf,
    /// Standard TLS peer-certificate verification configuration.
    pub tls_verify: RatlsTlsVerifyConf,
}

/// C-compatible borrowed byte buffer.
#[repr(C)]
pub struct RatlsBuffer {
    /// Pointer to the first byte.
    pub data: *const u8,
    /// Number of bytes available at `data`.
    pub len: usize,
}

/// C-compatible dynamic certificate signing configuration.
#[repr(C)]
pub struct RatlsCertificateConf {
    /// Optional unencrypted PEM CA private key.
    pub issuer_private_key: RatlsBuffer,
    /// Optional PEM issuer certificate bundle.
    pub issuer_certificate_chain: RatlsBuffer,
    /// Optional array of NUL-terminated SAN strings.
    pub subject_alt_names: *const *const c_char,
    /// Number of entries at `subject_alt_names`.
    pub subject_alt_names_len: usize,
}

/// C-compatible standard TLS peer-verification configuration.
#[repr(C)]
pub struct RatlsTlsVerifyConf {
    /// Non-zero enables standard OpenSSL peer-certificate verification.
    pub verify_peer_certificate: u8,
    /// Non-zero adds OpenSSL's default system CA paths.
    pub use_system_ca: u8,
    /// Optional PEM CA bundle trusted for peer verification.
    pub trusted_ca_chain: RatlsBuffer,
    /// Optional NUL-terminated DNS name or IP address.
    pub expected_peer_name: *const c_char,
}

/// C-compatible custom claim view.
#[repr(C)]
pub struct RatlsCustomClaim {
    /// NUL-terminated claim name.
    pub name: *const c_char,
    /// Claim value bytes.
    pub value: RatlsBuffer,
}

/// C-compatible view passed to verification callbacks.
///
/// The pointed-to buffers are valid only for the duration of the callback.
#[repr(C)]
pub struct RatlsVerifiedEvidence {
    /// NUL-terminated evidence type string, currently `cca`.
    pub evidence_type: *const c_char,
    /// Raw CCA token bytes.
    pub raw_token: RatlsBuffer,
    /// JSON-encoded public CCA claims.
    pub claims_json: RatlsBuffer,
    /// Custom claims decoded from the RATS-TLS claims buffer.
    pub custom_claims: *const RatlsCustomClaim,
    /// Number of custom claims available at `custom_claims`.
    pub custom_claims_len: usize,
}

/// C verification callback type.
///
/// Return non-zero to accept the evidence. Return zero to reject it.
pub type RatlsVerificationCallback =
    Option<unsafe extern "C" fn(*const RatlsVerifiedEvidence, *mut c_void) -> c_int>;

/// Opaque C handle that owns a Rust [`RatsTlsHandle`](crate::api::RaTlsHandle).
pub struct RatlsHandle {
    inner: InnerHandle,
    stream: Option<Box<dyn TransportStream>>,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").expect("empty CString"));
}

#[no_mangle]
/// Return the C ABI version string.
pub extern "C" fn ratls_api_version() -> *const c_char {
    c"ratls-api-rust-0.1.0".as_ptr()
}

#[no_mangle]
/// Return the last error message for the current thread.
///
/// The returned pointer remains valid until the same thread calls another FFI
/// function that updates the thread-local error state.
pub extern "C" fn ratls_last_error() -> *const c_char {
    LAST_ERROR.with(|cell| cell.borrow().as_ptr())
}

#[no_mangle]
/// Set the process-wide RATS-TLS log level.
pub extern "C" fn ratls_set_log_level(level: u32) -> i32 {
    let level = match level {
        RATLS_LOG_LEVEL_DEBUG => LogLevel::Debug,
        RATLS_LOG_LEVEL_INFO => LogLevel::Info,
        RATLS_LOG_LEVEL_WARN => LogLevel::Warn,
        RATLS_LOG_LEVEL_ERROR => LogLevel::Error,
        RATLS_LOG_LEVEL_FATAL => LogLevel::Fatal,
        RATLS_LOG_LEVEL_NONE => LogLevel::None,
        _ => {
            set_last_error(format!("unknown log level {level}"));
            return RATLS_INVALID_ARGUMENT;
        }
    };
    set_log_level(level);
    RATLS_SUCCESS
}

#[no_mangle]
/// Fill a C configuration with the library defaults.
///
/// The resulting configuration uses the CCA attester and verifier, OpenSSL,
/// ECC P-256/SHA-256, one-way attestation, and the client role. Callers can
/// overwrite individual fields after this function returns.
///
/// # Safety
///
/// `conf` must point to writable storage for one [`RatlsConf`].
pub unsafe extern "C" fn ratls_conf_init(conf: *mut RatlsConf) -> i32 {
    clear_last_error();
    if conf.is_null() {
        set_last_error("ratls_conf_init conf is null");
        return RATLS_INVALID_ARGUMENT;
    }
    *conf = RatlsConf {
        struct_size: std::mem::size_of::<RatlsConf>(),
        attester_type: RATLS_ATTESTER_CCA,
        verifier_type: RATLS_VERIFIER_CCA,
        tls_type: ptr::null(),
        crypto_type: ptr::null(),
        cert_algo: RATLS_CERT_ALGO_DEFAULT,
        mutual: 0,
        server: 0,
        custom_claims: ptr::null(),
        custom_claims_len: 0,
        certificate: RatlsCertificateConf {
            issuer_private_key: empty_buffer(),
            issuer_certificate_chain: empty_buffer(),
            subject_alt_names: ptr::null(),
            subject_alt_names_len: 0,
        },
        tls_verify: RatlsTlsVerifyConf {
            verify_peer_certificate: 0,
            use_system_ca: 0,
            trusted_ca_chain: empty_buffer(),
            expected_peer_name: ptr::null(),
        },
    };
    RATLS_SUCCESS
}

#[no_mangle]
/// Create a new RATS-TLS handle from a C configuration.
///
/// On success, `handle_out` receives an opaque handle that must be released with
/// [`ratls_cleanup`].
///
/// # Safety
///
/// `conf` must be null or point to a valid [`RatlsConf`]. `handle_out` must
/// point to writable storage for one handle pointer.
pub unsafe extern "C" fn ratls_init(
    conf: *const RatlsConf,
    handle_out: *mut *mut RatlsHandle,
) -> i32 {
    clear_last_error();
    if handle_out.is_null() {
        set_last_error("ratls_init handle_out is null");
        return RATLS_INVALID_ARGUMENT;
    }
    *handle_out = ptr::null_mut();

    let result = catch_unwind(AssertUnwindSafe(|| {
        let conf = conf_from_c(conf)?;
        let inner = rats_tls_init(conf)?;
        Ok::<RatlsHandle, crate::RaTlsError>(RatlsHandle {
            inner,
            stream: None,
        })
    }));

    match result {
        Ok(Ok(handle)) => {
            *handle_out = Box::into_raw(Box::new(handle));
            RATLS_SUCCESS
        }
        Ok(Err(err)) => {
            set_last_error(err.to_string());
            RATLS_INIT_ERROR
        }
        Err(_) => {
            set_last_error("panic in ratls_init");
            RATLS_PANIC
        }
    }
}

#[no_mangle]
/// Release a handle created by [`ratls_init`].
///
/// # Safety
///
/// `handle` must be null or a pointer returned by [`ratls_init`] that has not
/// already been released.
pub unsafe extern "C" fn ratls_cleanup(handle: *mut RatlsHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
/// Register an optional application verification callback.
///
/// Passing `NULL` disables the callback. `user_data` is stored opaquely and
/// passed back to the callback without interpretation.
///
/// # Safety
///
/// `handle` must point to a live [`RatlsHandle`]. When supplied, `callback`
/// must remain valid until it is replaced or the handle is released.
pub unsafe extern "C" fn ratls_set_verification_callback(
    handle: *mut RatlsHandle,
    callback: RatlsVerificationCallback,
    user_data: *mut c_void,
) -> i32 {
    clear_last_error();
    if handle.is_null() {
        set_last_error("ratls_set_verification_callback handle is null");
        return RATLS_INVALID_ARGUMENT;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(callback) = callback else {
            rats_tls_set_verification_callback(&mut (*handle).inner, None);
            return Ok::<(), crate::RaTlsError>(());
        };
        let user_data_addr = user_data as usize;
        rats_tls_set_verification_callback(
            &mut (*handle).inner,
            Some(Box::new(move |evidence| {
                let evidence_type = CString::new(evidence.name.to_string()).map_err(|_| {
                    crate::RaTlsError::InvalidData("evidence type contains NUL".into())
                })?;
                let parsed_claims =
                    crate::core::dice::parse_claims_buffer(&evidence.claims_buffer)?;
                let custom_claim_names = parsed_claims
                    .custom_claims
                    .iter()
                    .map(|claim| CString::new(claim.name.as_str()))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| {
                        crate::RaTlsError::InvalidData("custom claim name contains NUL".into())
                    })?;
                let custom_claims = parsed_claims
                    .custom_claims
                    .iter()
                    .zip(custom_claim_names.iter())
                    .map(|(claim, name)| RatlsCustomClaim {
                        name: name.as_ptr(),
                        value: RatlsBuffer {
                            data: claim.value.as_ptr(),
                            len: claim.value.len(),
                        },
                    })
                    .collect::<Vec<_>>();
                let raw_token = crate::verifiers::cca::evidence::CcaEvidence::decode_raw_native(
                    &evidence.raw_evidence,
                )?
                .token;
                let claims_json = serde_json::to_vec(&evidence.evidence_json)?;
                let view = RatlsVerifiedEvidence {
                    evidence_type: evidence_type.as_ptr(),
                    raw_token: RatlsBuffer {
                        data: raw_token.as_ptr(),
                        len: raw_token.len(),
                    },
                    claims_json: RatlsBuffer {
                        data: claims_json.as_ptr(),
                        len: claims_json.len(),
                    },
                    custom_claims: custom_claims.as_ptr(),
                    custom_claims_len: custom_claims.len(),
                };
                let rc = unsafe { callback(&view, user_data_addr as *mut c_void) };
                if rc == 0 {
                    return Err(crate::RaTlsError::InvalidData(
                        "verification callback rejected evidence".into(),
                    ));
                }
                Ok(())
            })),
        );
        Ok(())
    }));

    match result {
        Ok(Ok(())) => RATLS_SUCCESS,
        Ok(Err(err)) => {
            set_last_error(err.to_string());
            RATLS_INVALID_ARGUMENT
        }
        Err(_) => {
            set_last_error("panic in ratls_set_verification_callback");
            RATLS_PANIC
        }
    }
}

#[no_mangle]
/// Negotiate RATS-TLS over an existing connected socket file descriptor.
///
/// The file descriptor is duplicated before being wrapped, so the caller
/// remains responsible for closing the original descriptor.
///
/// # Safety
///
/// `handle` must point to a live [`RatlsHandle`] and `fd` must identify a valid
/// connected stream socket.
pub unsafe extern "C" fn ratls_negotiate_fd(handle: *mut RatlsHandle, fd: c_int) -> i32 {
    clear_last_error();
    if handle.is_null() || fd < 0 {
        set_last_error("invalid ratls_negotiate_fd argument");
        return RATLS_INVALID_ARGUMENT;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let duplicated = libc::dup(fd);
        if duplicated < 0 {
            return Err(crate::RaTlsError::Io(std::io::Error::last_os_error()));
        }
        let stream = TcpStream::from_raw_fd(duplicated);
        let stream = rats_tls_negotiate(&mut (*handle).inner, Box::new(stream))?;
        (*handle).stream = Some(stream);
        Ok::<(), crate::RaTlsError>(())
    }));

    match result {
        Ok(Ok(())) => RATLS_SUCCESS,
        Ok(Err(err)) => {
            set_last_error(err.to_string());
            RATLS_NEGOTIATE_ERROR
        }
        Err(_) => {
            set_last_error("panic in ratls_negotiate_fd");
            RATLS_PANIC
        }
    }
}

#[no_mangle]
/// Transmit bytes over a negotiated RATS-TLS session.
///
/// # Safety
///
/// `handle` must point to a live negotiated handle. When `data_len` is
/// non-zero, `data` must point to at least `data_len` readable bytes.
/// `written_out`, when non-null, must be writable.
pub unsafe extern "C" fn ratls_transmit(
    handle: *mut RatlsHandle,
    data: *const u8,
    data_len: usize,
    written_out: *mut usize,
) -> i32 {
    clear_last_error();
    if handle.is_null() || (data.is_null() && data_len != 0) {
        set_last_error("invalid ratls_transmit argument");
        return RATLS_INVALID_ARGUMENT;
    }
    if !written_out.is_null() {
        *written_out = 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let data = if data_len == 0 {
            &[]
        } else {
            slice::from_raw_parts(data, data_len)
        };
        let stream = (*handle).stream.as_deref_mut().ok_or_else(|| {
            crate::RaTlsError::InvalidArgument("RATS-TLS stream is not negotiated".into())
        })?;
        rats_tls_transmit(stream, data)
    }));

    match result {
        Ok(Ok(written)) => {
            if !written_out.is_null() {
                *written_out = written;
            }
            RATLS_SUCCESS
        }
        Ok(Err(err)) => {
            set_last_error(err.to_string());
            RATLS_TRANSMIT_ERROR
        }
        Err(_) => {
            set_last_error("panic in ratls_transmit");
            RATLS_PANIC
        }
    }
}

#[no_mangle]
/// Receive bytes from a negotiated RATS-TLS session.
///
/// # Safety
///
/// `handle` must point to a live negotiated handle. `buffer` must point to at
/// least `buffer_len` writable bytes. `read_out`, when non-null, must be
/// writable.
pub unsafe extern "C" fn ratls_receive(
    handle: *mut RatlsHandle,
    buffer: *mut u8,
    buffer_len: usize,
    read_out: *mut usize,
) -> i32 {
    clear_last_error();
    if handle.is_null() || buffer.is_null() || buffer_len == 0 {
        set_last_error("invalid ratls_receive argument");
        return RATLS_INVALID_ARGUMENT;
    }
    if !read_out.is_null() {
        *read_out = 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let buffer = slice::from_raw_parts_mut(buffer, buffer_len);
        let stream = (*handle).stream.as_deref_mut().ok_or_else(|| {
            crate::RaTlsError::InvalidArgument("RATS-TLS stream is not negotiated".into())
        })?;
        rats_tls_receive(stream, buffer)
    }));

    match result {
        Ok(Ok(read)) => {
            if !read_out.is_null() {
                *read_out = read;
            }
            RATLS_SUCCESS
        }
        Ok(Err(err)) => {
            set_last_error(err.to_string());
            RATLS_RECEIVE_ERROR
        }
        Err(_) => {
            set_last_error("panic in ratls_receive");
            RATLS_PANIC
        }
    }
}

unsafe fn conf_from_c(conf: *const RatlsConf) -> crate::Result<RaTlsConf> {
    let mut out = RaTlsConf::default();
    if conf.is_null() {
        return Ok(out);
    }
    let conf = &*conf;
    if conf.struct_size != std::mem::size_of::<RatlsConf>() {
        return Err(crate::RaTlsError::InvalidArgument(format!(
            "ratls_conf_t size mismatch: expected {}, got {}; initialize it with ratls_conf_init",
            std::mem::size_of::<RatlsConf>(),
            conf.struct_size
        )));
    }
    out.attester = match conf.attester_type {
        RATLS_ATTESTER_NONE => None,
        RATLS_ATTESTER_CCA => Some(AttesterPlugin::Cca),
        value => {
            return Err(crate::RaTlsError::Unsupported(format!(
                "unknown C attester selector {value}"
            )))
        }
    };
    out.verifier = match conf.verifier_type {
        RATLS_VERIFIER_NONE => None,
        RATLS_VERIFIER_CCA => Some(VerifierPlugin::Cca),
        value => {
            return Err(crate::RaTlsError::Unsupported(format!(
                "unknown C verifier selector {value}"
            )))
        }
    };
    if let Some(tls_type) = optional_c_string(conf.tls_type, "tls_type", MAX_WRAPPER_NAME_LENGTH)? {
        out.tls_type = match tls_type.to_ascii_lowercase().as_str() {
            "openssl" => TlsWrapperEnum::OpenSsl,
            _ => {
                return Err(crate::RaTlsError::Unsupported(format!(
                    "unknown TLS wrapper '{tls_type}'"
                )))
            }
        };
    }
    if let Some(crypto_type) =
        optional_c_string(conf.crypto_type, "crypto_type", MAX_WRAPPER_NAME_LENGTH)?
    {
        out.crypto_type = match crypto_type.to_ascii_lowercase().as_str() {
            "openssl" => CryptoWrapperEnum::OpenSsl,
            _ => {
                return Err(crate::RaTlsError::Unsupported(format!(
                    "unknown crypto wrapper '{crypto_type}'"
                )))
            }
        };
    }
    out.cert_algo = cert_algorithm_from_c(conf.cert_algo)?;
    out.mutual = conf.mutual != 0;
    out.server = conf.server != 0;
    out.custom_claims = custom_claims_from_c(conf.custom_claims, conf.custom_claims_len)?;
    out.certificate = CertificateConf {
        issuer_private_key: buffer_from_c(
            &conf.certificate.issuer_private_key,
            "issuer_private_key",
            MAX_ISSUER_PRIVATE_KEY_PEM_SIZE,
        )?,
        issuer_certificate_chain: buffer_from_c(
            &conf.certificate.issuer_certificate_chain,
            "issuer_certificate_chain",
            MAX_ISSUER_CERTIFICATE_CHAIN_PEM_SIZE,
        )?,
        subject_alt_names: c_string_array(
            conf.certificate.subject_alt_names,
            conf.certificate.subject_alt_names_len,
            "subject_alt_names",
            MAX_SUBJECT_ALT_NAMES,
            MAX_SUBJECT_ALT_NAME_LENGTH,
        )?,
    };
    out.tls_verify = if conf.tls_verify.verify_peer_certificate == 0 {
        TlsVerifyConf::default()
    } else {
        TlsVerifyConf {
            verify_peer_certificate: true,
            use_system_ca: conf.tls_verify.use_system_ca != 0,
            trusted_ca_chain: buffer_from_c(
                &conf.tls_verify.trusted_ca_chain,
                "trusted_ca_chain",
                MAX_TRUSTED_CA_CHAIN_PEM_SIZE,
            )?,
            expected_peer_name: optional_c_string(
                conf.tls_verify.expected_peer_name,
                "expected_peer_name",
                MAX_EXPECTED_PEER_NAME_LENGTH,
            )?,
        }
    };
    Ok(out)
}

const fn empty_buffer() -> RatlsBuffer {
    RatlsBuffer {
        data: ptr::null(),
        len: 0,
    }
}

unsafe fn buffer_from_c(
    buffer: &RatlsBuffer,
    name: &str,
    maximum: usize,
) -> crate::Result<Vec<u8>> {
    if buffer.len > maximum {
        return Err(crate::RaTlsError::InvalidArgument(format!(
            "{name} exceeds {maximum} bytes"
        )));
    }
    if buffer.len == 0 {
        return Ok(Vec::new());
    }
    if buffer.data.is_null() {
        return Err(crate::RaTlsError::InvalidArgument(format!(
            "{name} data is null but its length is non-zero"
        )));
    }
    Ok(slice::from_raw_parts(buffer.data, buffer.len).to_vec())
}

unsafe fn c_string_array(
    values: *const *const c_char,
    len: usize,
    name: &str,
    maximum_count: usize,
    maximum_string_length: usize,
) -> crate::Result<Vec<String>> {
    if len > maximum_count {
        return Err(crate::RaTlsError::InvalidArgument(format!(
            "{name} contains too many entries: maximum is {maximum_count}"
        )));
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    if values.is_null() {
        return Err(crate::RaTlsError::InvalidArgument(format!(
            "{name} is null but its length is non-zero"
        )));
    }
    slice::from_raw_parts(values, len)
        .iter()
        .map(|value| required_c_string(*value, name, maximum_string_length))
        .collect()
}

unsafe fn custom_claims_from_c(
    claims: *const RatlsCustomClaim,
    len: usize,
) -> crate::Result<Vec<CustomClaim>> {
    if len > MAX_CUSTOM_CLAIMS {
        return Err(crate::RaTlsError::InvalidArgument(format!(
            "too many custom claims: maximum is {MAX_CUSTOM_CLAIMS}"
        )));
    }
    if claims.is_null() {
        if len == 0 {
            return Ok(Vec::new());
        }
        return Err(crate::RaTlsError::InvalidArgument(
            "custom_claims is null but custom_claims_len is non-zero".into(),
        ));
    }
    let claims = slice::from_raw_parts(claims, len);
    let mut output = Vec::with_capacity(len);
    let mut total_value_length = 0usize;
    for claim in claims {
        let name = required_c_string(
            claim.name,
            "custom claim name",
            MAX_CUSTOM_CLAIM_NAME_LENGTH,
        )?;
        total_value_length = total_value_length
            .checked_add(claim.value.len)
            .ok_or_else(|| {
                crate::RaTlsError::InvalidArgument(
                    "total custom claim data length overflows".into(),
                )
            })?;
        if total_value_length > MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH {
            return Err(crate::RaTlsError::InvalidArgument(format!(
                "total custom claim data exceeds {MAX_CUSTOM_CLAIMS_TOTAL_VALUE_LENGTH} bytes"
            )));
        }
        let value = buffer_from_c(
            &claim.value,
            "custom claim value",
            MAX_CUSTOM_CLAIM_VALUE_LENGTH,
        )?;
        output.push(CustomClaim { name, value });
    }
    Ok(output)
}

fn cert_algorithm_from_c(value: u32) -> crate::Result<CertAlgorithm> {
    match value {
        RATLS_CERT_ALGO_RSA_3072_SHA256 => Ok(CertAlgorithm::Rsa3072Sha256),
        RATLS_CERT_ALGO_ECC_256_SHA256 | RATLS_CERT_ALGO_DEFAULT => Ok(CertAlgorithm::Ecc256Sha256),
        RATLS_CERT_ALGO_MAX => Err(crate::RaTlsError::InvalidArgument(
            "RATLS_CERT_ALGO_MAX is not a selectable certificate algorithm".into(),
        )),
        _ => Err(crate::RaTlsError::InvalidArgument(format!(
            "unknown certificate algorithm {value}"
        ))),
    }
}

unsafe fn optional_c_string(
    ptr: *const c_char,
    name: &str,
    maximum: usize,
) -> crate::Result<Option<String>> {
    if ptr.is_null() {
        return Ok(None);
    }
    let scan_length = maximum
        .checked_add(1)
        .ok_or_else(|| crate::RaTlsError::InvalidArgument(format!("{name} limit overflows")))?;
    let len = libc::strnlen(ptr, scan_length);
    if len > maximum {
        return Err(crate::RaTlsError::InvalidArgument(format!(
            "{name} exceeds {maximum} bytes or is not NUL-terminated"
        )));
    }
    let value = std::str::from_utf8(slice::from_raw_parts(ptr.cast::<u8>(), len))
        .map_err(|_| crate::RaTlsError::InvalidArgument(format!("{name} is not valid UTF-8")))?;
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.to_string()))
    }
}

unsafe fn required_c_string(
    ptr: *const c_char,
    name: &str,
    maximum: usize,
) -> crate::Result<String> {
    optional_c_string(ptr, name, maximum)?
        .ok_or_else(|| crate::RaTlsError::InvalidArgument(format!("{name} is null or empty")))
}

fn clear_last_error() {
    set_last_error("");
}

fn set_last_error(message: impl AsRef<str>) {
    let sanitized = message.as_ref().replace('\0', "\\0");
    let cstr = CString::new(sanitized).unwrap_or_else(|_| CString::new("invalid error").unwrap());
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = cstr;
    });
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};
    use std::io::Cursor;
    use std::mem::MaybeUninit;

    use ciborium::Value;

    use super::*;
    use crate::verifiers::cca::evidence::CcaEvidence;
    use crate::verifiers::Evidence;

    unsafe fn default_c_conf() -> RatlsConf {
        let mut conf = MaybeUninit::<RatlsConf>::uninit();
        assert_eq!(ratls_conf_init(conf.as_mut_ptr()), RATLS_SUCCESS);
        conf.assume_init()
    }

    unsafe fn new_handle() -> *mut RatlsHandle {
        let mut handle = ptr::null_mut();
        assert_eq!(ratls_init(ptr::null(), &mut handle), RATLS_SUCCESS);
        assert!(!handle.is_null());
        handle
    }

    fn last_error() -> String {
        unsafe { CStr::from_ptr(ratls_last_error()) }
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn encoded_hash_claim(digest: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(
            &Value::Array(vec![
                Value::Integer(1.into()),
                Value::Bytes(digest.to_vec()),
            ]),
            &mut encoded,
        )
        .unwrap();
        encoded
    }

    fn claims_buffer() -> Vec<u8> {
        let claims = Value::Map(vec![
            (
                Value::Text("pubkey-hash".into()),
                Value::Bytes(encoded_hash_claim(&[1; 32])),
            ),
            (
                Value::Text("client-key-share-hash".into()),
                Value::Bytes(encoded_hash_claim(&[2; 32])),
            ),
            (Value::Text("nonce".into()), Value::Bytes(vec![3; 32])),
            (Value::Text("application".into()), Value::Bytes(vec![4, 5])),
        ]);
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&claims, &mut encoded).unwrap();
        encoded
    }

    #[test]
    fn exposes_version_log_levels_defaults_and_thread_local_errors() {
        unsafe {
            assert_eq!(
                CStr::from_ptr(ratls_api_version()).to_str().unwrap(),
                "ratls-api-rust-0.1.0"
            );
            for level in 0..=5 {
                assert_eq!(ratls_set_log_level(level), RATLS_SUCCESS);
            }
            assert_eq!(ratls_set_log_level(99), RATLS_INVALID_ARGUMENT);
            assert!(last_error().contains("unknown log level"));

            assert_eq!(ratls_conf_init(ptr::null_mut()), RATLS_INVALID_ARGUMENT);
            assert!(last_error().contains("conf is null"));

            let conf = default_c_conf();
            assert_eq!(conf.struct_size, std::mem::size_of::<RatlsConf>());
            assert_eq!(conf.attester_type, RATLS_ATTESTER_CCA);
            assert_eq!(conf.verifier_type, RATLS_VERIFIER_CCA);
            assert_eq!(conf.cert_algo, RATLS_CERT_ALGO_DEFAULT);
            assert_eq!(conf.mutual, 0);
            assert_eq!(conf.server, 0);
        }
    }

    #[test]
    fn initializes_c_handles_and_rejects_invalid_direct_arguments() {
        unsafe {
            assert_eq!(
                ratls_init(ptr::null(), ptr::null_mut()),
                RATLS_INVALID_ARGUMENT
            );

            let handle = new_handle();
            assert_eq!(
                ratls_set_verification_callback(ptr::null_mut(), None, ptr::null_mut()),
                RATLS_INVALID_ARGUMENT
            );
            assert_eq!(
                ratls_negotiate_fd(ptr::null_mut(), 0),
                RATLS_INVALID_ARGUMENT
            );
            assert_eq!(ratls_negotiate_fd(handle, -1), RATLS_INVALID_ARGUMENT);

            let mut written = usize::MAX;
            assert_eq!(
                ratls_transmit(ptr::null_mut(), ptr::null(), 0, &mut written),
                RATLS_INVALID_ARGUMENT
            );
            assert_eq!(
                ratls_transmit(handle, ptr::null(), 1, &mut written),
                RATLS_INVALID_ARGUMENT
            );
            assert_eq!(
                ratls_transmit(handle, ptr::null(), 0, &mut written),
                RATLS_TRANSMIT_ERROR
            );
            assert_eq!(written, 0);

            let mut byte = 0;
            let mut read = usize::MAX;
            assert_eq!(
                ratls_receive(ptr::null_mut(), &mut byte, 1, &mut read),
                RATLS_INVALID_ARGUMENT
            );
            assert_eq!(
                ratls_receive(handle, ptr::null_mut(), 1, &mut read),
                RATLS_INVALID_ARGUMENT
            );
            assert_eq!(
                ratls_receive(handle, &mut byte, 0, &mut read),
                RATLS_INVALID_ARGUMENT
            );
            assert_eq!(
                ratls_receive(handle, &mut byte, 1, &mut read),
                RATLS_RECEIVE_ERROR
            );
            assert_eq!(read, 0);

            ratls_cleanup(handle);
            ratls_cleanup(ptr::null_mut());
        }
    }

    #[test]
    fn c_send_and_receive_delegate_to_the_negotiated_stream() {
        unsafe {
            let handle = new_handle();
            (*handle).stream = Some(Box::new(Cursor::new(Vec::new())));
            let data = b"abc";
            let mut written = 0;
            assert_eq!(
                ratls_transmit(handle, data.as_ptr(), data.len(), &mut written),
                RATLS_SUCCESS
            );
            assert_eq!(written, data.len());

            (*handle).stream = Some(Box::new(Cursor::new(b"xyz".to_vec())));
            let mut output = [0; 3];
            let mut read = 0;
            assert_eq!(
                ratls_receive(handle, output.as_mut_ptr(), output.len(), &mut read),
                RATLS_SUCCESS
            );
            assert_eq!(read, 3);
            assert_eq!(&output, b"xyz");
            ratls_cleanup(handle);
        }
    }

    unsafe extern "C" fn inspect_callback(
        evidence: *const RatlsVerifiedEvidence,
        user_data: *mut c_void,
    ) -> c_int {
        assert!(!evidence.is_null());
        let evidence = &*evidence;
        assert_eq!(
            CStr::from_ptr(evidence.evidence_type).to_str().unwrap(),
            "Cca"
        );
        assert_eq!(
            slice::from_raw_parts(evidence.raw_token.data, evidence.raw_token.len),
            &[9, 8]
        );
        assert_eq!(evidence.custom_claims_len, 1);
        let claim = &*evidence.custom_claims;
        assert_eq!(CStr::from_ptr(claim.name).to_str().unwrap(), "application");
        assert_eq!(
            slice::from_raw_parts(claim.value.data, claim.value.len),
            &[4, 5]
        );
        *(user_data.cast::<bool>()) = true;
        1
    }

    #[test]
    fn callback_adapter_exposes_verified_evidence_and_can_be_disabled() {
        unsafe {
            let handle = new_handle();
            let mut called = false;
            assert_eq!(
                ratls_set_verification_callback(
                    handle,
                    Some(inspect_callback),
                    (&mut called as *mut bool).cast()
                ),
                RATLS_SUCCESS
            );
            let evidence = Evidence {
                tag: crate::verifiers::cca::EVIDENCE_TAG,
                name: VerifierPlugin::Cca,
                raw_evidence: CcaEvidence {
                    token: vec![9, 8],
                    dev_cert: vec![7],
                }
                .encode_raw_native(),
                claims_buffer: claims_buffer(),
                public_key_der: vec![],
                expected_nonce: vec![],
                expected_client_key_share_hash: vec![],
                evidence_json: serde_json::json!({"verified":true}),
            };
            (*handle).inner.user_callback.as_mut().unwrap()(&evidence).unwrap();
            assert!(called);

            assert_eq!(
                ratls_set_verification_callback(handle, None, ptr::null_mut()),
                RATLS_SUCCESS
            );
            assert!((*handle).inner.user_callback.is_none());
            ratls_cleanup(handle);
        }
    }

    #[test]
    fn converts_full_c_configuration_and_ignores_disabled_tls_material() {
        unsafe {
            let mut conf = default_c_conf();
            conf.attester_type = RATLS_ATTESTER_NONE;
            conf.verifier_type = RATLS_VERIFIER_CCA;
            conf.cert_algo = RATLS_CERT_ALGO_RSA_3072_SHA256;
            conf.mutual = 1;
            conf.server = 1;
            let tls = CString::new("OpEnSsL").unwrap();
            let crypto = CString::new("OPENSSL").unwrap();
            conf.tls_type = tls.as_ptr();
            conf.crypto_type = crypto.as_ptr();

            let san = CString::new("DNS:test.example").unwrap();
            let sans = [san.as_ptr()];
            conf.certificate.subject_alt_names = sans.as_ptr();
            conf.certificate.subject_alt_names_len = sans.len();

            let claim_name = CString::new("role").unwrap();
            let claim_value = [1, 2, 3];
            let claims = [RatlsCustomClaim {
                name: claim_name.as_ptr(),
                value: RatlsBuffer {
                    data: claim_value.as_ptr(),
                    len: claim_value.len(),
                },
            }];
            conf.custom_claims = claims.as_ptr();
            conf.custom_claims_len = claims.len();

            conf.tls_verify.verify_peer_certificate = 0;
            conf.tls_verify.trusted_ca_chain = RatlsBuffer {
                data: ptr::null(),
                len: usize::MAX,
            };
            let converted = conf_from_c(&conf).unwrap();
            assert!(converted.attester.is_none());
            assert!(converted.verifier.is_some());
            assert!(converted.mutual);
            assert!(converted.server);
            assert!(matches!(converted.cert_algo, CertAlgorithm::Rsa3072Sha256));
            assert_eq!(
                converted.certificate.subject_alt_names,
                ["DNS:test.example"]
            );
            assert_eq!(converted.custom_claims[0].name, "role");
            assert!(converted.tls_verify.trusted_ca_chain.is_empty());
        }
    }

    #[test]
    fn configuration_conversion_rejects_invalid_selectors_and_pointers() {
        unsafe {
            let mut conf = default_c_conf();
            conf.struct_size = 0;
            assert!(conf_from_c(&conf).is_err());
            conf.struct_size = std::mem::size_of::<RatlsConf>();

            conf.attester_type = 99;
            assert!(conf_from_c(&conf).is_err());
            conf.attester_type = RATLS_ATTESTER_CCA;
            conf.verifier_type = 99;
            assert!(conf_from_c(&conf).is_err());
            conf.verifier_type = RATLS_VERIFIER_CCA;

            let unknown = CString::new("unknown").unwrap();
            conf.tls_type = unknown.as_ptr();
            assert!(conf_from_c(&conf).is_err());
            conf.tls_type = ptr::null();
            conf.crypto_type = unknown.as_ptr();
            assert!(conf_from_c(&conf).is_err());
            conf.crypto_type = ptr::null();

            conf.cert_algo = RATLS_CERT_ALGO_MAX;
            assert!(conf_from_c(&conf).is_err());
            conf.cert_algo = 99;
            assert!(conf_from_c(&conf).is_err());
            conf.cert_algo = RATLS_CERT_ALGO_DEFAULT;

            conf.custom_claims = ptr::null();
            conf.custom_claims_len = 1;
            assert!(conf_from_c(&conf).is_err());
            conf.custom_claims_len = 0;

            conf.certificate.subject_alt_names = ptr::null();
            conf.certificate.subject_alt_names_len = 1;
            assert!(conf_from_c(&conf).is_err());
            conf.certificate.subject_alt_names_len = 0;

            conf.certificate.issuer_private_key = RatlsBuffer {
                data: ptr::null(),
                len: 1,
            };
            assert!(conf_from_c(&conf).is_err());
        }
    }

    #[test]
    fn string_buffer_and_claim_helpers_enforce_all_bounds() {
        unsafe {
            let bytes = [1, 2];
            assert_eq!(
                buffer_from_c(
                    &RatlsBuffer {
                        data: bytes.as_ptr(),
                        len: bytes.len()
                    },
                    "buffer",
                    2
                )
                .unwrap(),
                bytes
            );
            assert!(buffer_from_c(
                &RatlsBuffer {
                    data: bytes.as_ptr(),
                    len: 3
                },
                "buffer",
                2
            )
            .is_err());

            let valid = CString::new("valid").unwrap();
            assert_eq!(
                optional_c_string(valid.as_ptr(), "text", 5).unwrap(),
                Some("valid".into())
            );
            assert_eq!(optional_c_string(ptr::null(), "text", 5).unwrap(), None);
            assert!(required_c_string(ptr::null(), "text", 5).is_err());
            assert!(optional_c_string(valid.as_ptr(), "text", usize::MAX).is_err());
            let invalid_utf8 = [0xff_u8, 0];
            assert!(optional_c_string(invalid_utf8.as_ptr().cast(), "text", 2).is_err());

            assert!(c_string_array(ptr::null(), 1, "values", 1, 5).is_err());
            assert!(c_string_array(ptr::null(), 2, "values", 1, 5).is_err());

            assert!(custom_claims_from_c(ptr::null(), 1).is_err());
            assert!(custom_claims_from_c(ptr::null(), MAX_CUSTOM_CLAIMS + 1).is_err());
            assert!(custom_claims_from_c(ptr::null(), 0).unwrap().is_empty());
        }
    }
}
