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

//! Reusable Rust implementation of the RATS-TLS API surface.
//!
//! This crate contains the library side of the project: API dispatch, attester
//! and verifier registries, CCA evidence handling, OpenSSL-backed cryptography,
//! OpenSSL-backed TLS streams, and the C ABI shim.

/// High-level RATS-TLS operations such as init, negotiate_client, transmit, and receive.
pub mod api;
/// Attester implementations and the attester registry.
pub mod attesters;
/// Protocol-independent evidence and DICE data models.
pub mod core;
/// Cryptographic wrapper implementations.
pub mod crypto_wrappers;
/// C ABI wrapper around the Rust API layer.
pub mod ffi;
/// Minimal RATS-TLS style logging.
pub mod logger;
/// TLS wrapper implementations.
pub mod tls_wrappers;
/// Verifier implementations and the verifier registry.
pub mod verifiers;

/// Error type used by the Rust API.
#[derive(Debug, thiserror::Error)]
pub enum RaTlsError {
    /// The caller supplied an invalid argument.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The input data was syntactically valid at the transport boundary but semantically invalid.
    #[error("invalid data: {0}")]
    InvalidData(String),
    /// An operating-system or filesystem error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// An OpenSSL operation failed.
    #[error("openssl error: {0}")]
    OpenSsl(#[from] openssl::error::ErrorStack),
    /// JSON parsing or serialization failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// CBOR parsing or serialization failed.
    #[error("cbor error: {0}")]
    Cbor(String),
    /// The requested wrapper, mode, or algorithm is not implemented.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Result type used throughout the RATS-TLS library.
pub type Result<T> = std::result::Result<T, RaTlsError>;

/// Encode bytes as lower-case hexadecimal text.
pub fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Decode hexadecimal text into bytes.
pub fn hex_decode(text: &str) -> Result<Vec<u8>> {
    let clean = text.trim();
    if clean.len() & 1 != 0 {
        return Err(RaTlsError::InvalidData("hex string has odd length".into()));
    }
    let mut out = Vec::with_capacity(clean.len() / 2);
    let bytes = clean.as_bytes();
    for i in (0..bytes.len()).step_by(2) {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(RaTlsError::InvalidData(format!(
            "invalid hex byte: 0x{b:02x}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexadecimal_codec_round_trips_and_rejects_bad_text() {
        assert_eq!(hex_encode(&[0, 0xab, 0xff]), "00abff");
        assert_eq!(hex_decode(" 00ABff ").unwrap(), [0, 0xab, 0xff]);
        assert!(hex_decode("0").is_err());
        assert!(hex_decode("gg").is_err());
    }
}
