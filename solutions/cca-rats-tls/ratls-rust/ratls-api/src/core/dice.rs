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

use crate::RaTlsError;
use ciborium::value::{Integer, Value};
use std::io::Cursor;

/// Application-defined claim embedded into the RATS-TLS claims buffer.
#[derive(Debug, Clone)]
pub struct CustomClaim {
    /// Claim name.
    pub name: String,
    /// Claim value bytes.
    pub value: Vec<u8>,
}

/// Tagged DICE evidence buffer payload.
///
/// The encoded form is a CBOR tag containing a two-element array:
/// `h'evidence_raw'` and `h'claims_buffer'`.
#[derive(Debug, Clone)]
pub struct EvidenceBuffer {
    /// CBOR tag identifying the evidence kind.
    pub tag: u64,
    /// Raw evidence bytes owned by the attester implementation.
    pub evidence_raw: Vec<u8>,
    /// Claims buffer used to bind the TLS public key and optional nonce.
    pub claims_buffer: Vec<u8>,
}

/// Parsed claims buffer fields used by RATS-TLS certificate verification.
#[derive(Debug, Clone)]
pub struct ClaimsBuffer {
    /// Hash algorithm identifier for the public key hash.
    pub pubkey_hash_algo: u64,
    /// Hash of the certificate public key in SubjectPublicKeyInfo DER format.
    pub pubkey_hash: Vec<u8>,
    /// Hash algorithm identifier for the ClientHello key_share extension.
    pub client_key_share_hash_algo: u64,
    /// Hash of the complete encoded ClientHello key_share extension.
    pub client_key_share_hash: Vec<u8>,
    /// Optional TLS handshake nonce/random bound into the evidence.
    pub nonce: Option<Vec<u8>>,
    /// Application-defined claims preserved in the order they appeared.
    pub custom_claims: Vec<CustomClaim>,
}

/// Encode an evidence buffer as tagged CBOR.
pub fn encode_evidence_buffer(buffer: &EvidenceBuffer) -> Result<Vec<u8>, RaTlsError> {
    let value = Value::Tag(
        buffer.tag,
        Box::new(Value::Array(vec![
            Value::Bytes(buffer.evidence_raw.clone()),
            Value::Bytes(buffer.claims_buffer.clone()),
        ])),
    );
    let mut out = Vec::new();
    ciborium::ser::into_writer(&value, &mut out)
        .map_err(|err| RaTlsError::Cbor(err.to_string()))?;
    Ok(out)
}

/// Decode a tagged CBOR evidence buffer.
pub fn decode_evidence_buffer(data: &[u8]) -> Result<EvidenceBuffer, RaTlsError> {
    let value: Value = ciborium::de::from_reader(Cursor::new(data))
        .map_err(|err| RaTlsError::Cbor(err.to_string()))?;
    let Value::Tag(tag, item) = value else {
        return Err(RaTlsError::InvalidData(
            "evidence buffer is not a tagged CBOR item".into(),
        ));
    };
    let Value::Array(items) = *item else {
        return Err(RaTlsError::InvalidData(
            "evidence buffer tag does not contain an array".into(),
        ));
    };
    if items.len() != 2 {
        return Err(RaTlsError::InvalidData(
            "evidence buffer array length must be 2".into(),
        ));
    }
    let evidence_raw = match &items[0] {
        Value::Bytes(bytes) => bytes.clone(),
        _ => {
            return Err(RaTlsError::InvalidData(
                "evidence raw is not a CBOR byte string".into(),
            ))
        }
    };
    let claims_buffer = match &items[1] {
        Value::Bytes(bytes) => bytes.clone(),
        _ => {
            return Err(RaTlsError::InvalidData(
                "claims buffer is not a CBOR byte string".into(),
            ))
        }
    };
    Ok(EvidenceBuffer {
        tag,
        evidence_raw,
        claims_buffer,
    })
}

/// Parse a RATS-TLS claims buffer.
pub fn parse_claims_buffer(data: &[u8]) -> Result<ClaimsBuffer, RaTlsError> {
    let value: Value = ciborium::de::from_reader(Cursor::new(data))
        .map_err(|err| RaTlsError::Cbor(err.to_string()))?;
    let Value::Map(entries) = value else {
        return Err(RaTlsError::InvalidData(
            "claims buffer is not a CBOR map".into(),
        ));
    };
    let mut pubkey_hash_algo = None;
    let mut pubkey_hash = None;
    let mut client_key_share_hash_algo = None;
    let mut client_key_share_hash = None;
    let mut nonce = None;
    let mut custom_claims = Vec::new();
    for (key, value) in entries {
        let Value::Text(name) = key else {
            continue;
        };
        match name.as_str() {
            "pubkey-hash" => {
                let Value::Bytes(encoded) = value else {
                    return Err(RaTlsError::InvalidData(
                        "pubkey-hash claim is not bytes".into(),
                    ));
                };
                let parsed = parse_hash_claim_value("pubkey-hash", &encoded)?;
                pubkey_hash_algo = Some(parsed.0);
                pubkey_hash = Some(parsed.1);
            }
            "client-key-share-hash" => {
                let Value::Bytes(encoded) = value else {
                    return Err(RaTlsError::InvalidData(
                        "client-key-share-hash claim is not bytes".into(),
                    ));
                };
                let parsed = parse_hash_claim_value("client-key-share-hash", &encoded)?;
                client_key_share_hash_algo = Some(parsed.0);
                client_key_share_hash = Some(parsed.1);
            }
            "nonce" => {
                let Value::Bytes(bytes) = value else {
                    return Err(RaTlsError::InvalidData("nonce claim is not bytes".into()));
                };
                nonce = Some(bytes);
            }
            _ => {
                let Value::Bytes(bytes) = value else {
                    return Err(RaTlsError::InvalidData(format!(
                        "custom claim '{name}' is not bytes"
                    )));
                };
                custom_claims.push(CustomClaim { name, value: bytes });
            }
        }
    }
    Ok(ClaimsBuffer {
        pubkey_hash_algo: pubkey_hash_algo
            .ok_or_else(|| RaTlsError::InvalidData("claims buffer misses pubkey-hash".into()))?,
        pubkey_hash: pubkey_hash.ok_or_else(|| {
            RaTlsError::InvalidData("claims buffer misses pubkey-hash value".into())
        })?,
        client_key_share_hash_algo: client_key_share_hash_algo.ok_or_else(|| {
            RaTlsError::InvalidData("claims buffer misses client-key-share-hash".into())
        })?,
        client_key_share_hash: client_key_share_hash.ok_or_else(|| {
            RaTlsError::InvalidData("claims buffer misses client-key-share-hash value".into())
        })?,
        nonce,
        custom_claims,
    })
}

fn parse_hash_claim_value(name: &str, data: &[u8]) -> Result<(u64, Vec<u8>), RaTlsError> {
    let value: Value = ciborium::de::from_reader(Cursor::new(data))
        .map_err(|err| RaTlsError::Cbor(err.to_string()))?;
    let Value::Array(items) = value else {
        return Err(RaTlsError::InvalidData(format!(
            "{name} value is not an array"
        )));
    };
    if items.len() != 2 {
        return Err(RaTlsError::InvalidData(format!(
            "{name} value length must be 2"
        )));
    }
    let algo = match &items[0] {
        Value::Integer(value) => integer_to_u64(value)?,
        _ => {
            return Err(RaTlsError::InvalidData(
                "pubkey-hash algorithm is not integer".into(),
            ))
        }
    };
    let hash = match &items[1] {
        Value::Bytes(bytes) => bytes.clone(),
        _ => {
            return Err(RaTlsError::InvalidData(format!(
                "{name} digest is not bytes"
            )))
        }
    };
    Ok((algo, hash))
}

fn integer_to_u64(value: &Integer) -> Result<u64, RaTlsError> {
    let raw: i128 = (*value).into();
    u64::try_from(raw).map_err(|_| RaTlsError::InvalidData("negative CBOR integer".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: &Value) -> Vec<u8> {
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(value, &mut encoded).unwrap();
        encoded
    }

    #[test]
    fn evidence_buffer_round_trips_and_rejects_wrong_shapes() {
        let input = EvidenceBuffer {
            tag: 7,
            evidence_raw: vec![1, 2],
            claims_buffer: vec![3, 4],
        };
        let decoded = decode_evidence_buffer(&encode_evidence_buffer(&input).unwrap()).unwrap();
        assert_eq!(decoded.tag, 7);
        assert_eq!(decoded.evidence_raw, [1, 2]);
        assert_eq!(decoded.claims_buffer, [3, 4]);

        for value in [
            Value::Array(vec![]),
            Value::Tag(1, Box::new(Value::Map(vec![]))),
            Value::Tag(1, Box::new(Value::Array(vec![]))),
            Value::Tag(
                1,
                Box::new(Value::Array(vec![
                    Value::Text("not bytes".into()),
                    Value::Bytes(vec![]),
                ])),
            ),
            Value::Tag(
                1,
                Box::new(Value::Array(vec![
                    Value::Bytes(vec![]),
                    Value::Text("not bytes".into()),
                ])),
            ),
        ] {
            assert!(decode_evidence_buffer(&encode(&value)).is_err());
        }
    }

    #[test]
    fn claims_parser_rejects_missing_and_malformed_reserved_claims() {
        assert!(parse_claims_buffer(&encode(&Value::Array(vec![]))).is_err());
        assert!(parse_claims_buffer(&encode(&Value::Map(vec![]))).is_err());

        let malformed = [
            ("pubkey-hash", Value::Text("bad".into())),
            ("client-key-share-hash", Value::Text("bad".into())),
            ("nonce", Value::Text("bad".into())),
            ("custom", Value::Text("bad".into())),
        ];
        for (name, value) in malformed {
            let map = Value::Map(vec![(Value::Text(name.into()), value)]);
            assert!(parse_claims_buffer(&encode(&map)).is_err());
        }
    }

    #[test]
    fn hash_claim_parser_checks_array_algorithm_and_digest_types() {
        for value in [
            Value::Text("bad".into()),
            Value::Array(vec![]),
            Value::Array(vec![Value::Text("bad".into()), Value::Bytes(vec![1])]),
            Value::Array(vec![Value::Integer(1.into()), Value::Text("bad".into())]),
            Value::Array(vec![Value::Integer((-1_i64).into()), Value::Bytes(vec![1])]),
        ] {
            assert!(parse_hash_claim_value("hash", &encode(&value)).is_err());
        }
        assert_eq!(
            parse_hash_claim_value(
                "hash",
                &encode(&Value::Array(vec![
                    Value::Integer(1.into()),
                    Value::Bytes(vec![2])
                ]))
            )
            .unwrap(),
            (1, vec![2])
        );
    }
}
