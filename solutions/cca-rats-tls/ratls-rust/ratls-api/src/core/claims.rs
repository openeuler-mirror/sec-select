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

use crate::core::dice::CustomClaim;
use crate::RaTlsError;
use ciborium::Value;

pub fn build_claims_buffer(
    pubkey_hash: &[u8],
    client_key_share_hash: &[u8],
    client_tls_random: &[u8],
    custom_claims: &[CustomClaim],
) -> Result<Vec<u8>, RaTlsError> {
    let mut claims = vec![
        (
            Value::Text("pubkey-hash".into()),
            Value::Bytes(encode_hash_claim(pubkey_hash)?),
        ),
        (
            Value::Text("client-key-share-hash".into()),
            Value::Bytes(encode_hash_claim(client_key_share_hash)?),
        ),
        (
            Value::Text("nonce".into()),
            Value::Bytes(client_tls_random.to_vec()),
        ),
    ];
    for claim in custom_claims {
        claims.push((
            Value::Text(claim.name.clone()),
            Value::Bytes(claim.value.clone()),
        ));
    }
    let mut output = Vec::new();
    ciborium::ser::into_writer(&Value::Map(claims), &mut output)
        .map_err(|err| RaTlsError::Cbor(err.to_string()))?;
    Ok(output)
}

fn encode_hash_claim(digest: &[u8]) -> Result<Vec<u8>, RaTlsError> {
    let value = Value::Array(vec![
        Value::Integer(1.into()),
        Value::Bytes(digest.to_vec()),
    ]);
    let mut output = Vec::new();
    ciborium::ser::into_writer(&value, &mut output)
        .map_err(|err| RaTlsError::Cbor(err.to_string()))?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dice::parse_claims_buffer;

    #[test]
    fn generated_claims_round_trip_through_the_parser() {
        let public_key_hash = [1; 32];
        let key_share_hash = [2; 32];
        let nonce = [3; 32];
        let custom_claims = [CustomClaim {
            name: "custom".into(),
            value: vec![4, 5],
        }];

        let encoded =
            build_claims_buffer(&public_key_hash, &key_share_hash, &nonce, &custom_claims).unwrap();
        let decoded = parse_claims_buffer(&encoded).unwrap();

        assert_eq!(decoded.pubkey_hash_algo, 1);
        assert_eq!(decoded.pubkey_hash, public_key_hash);
        assert_eq!(decoded.client_key_share_hash_algo, 1);
        assert_eq!(decoded.client_key_share_hash, key_share_hash);
        assert_eq!(decoded.nonce.as_deref(), Some(nonce.as_slice()));
        assert_eq!(decoded.custom_claims.len(), 1);
        assert_eq!(decoded.custom_claims[0].name, "custom");
        assert_eq!(decoded.custom_claims[0].value, vec![4, 5]);
    }
}
