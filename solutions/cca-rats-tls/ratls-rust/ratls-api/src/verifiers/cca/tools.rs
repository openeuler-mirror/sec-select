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

use std::cmp::Ordering::{Greater, Less};

use ciborium::Value;
use coset::{iana, CoseKey, KeyType, Label};
use openssl::asn1::Asn1Time;
use openssl::bn::BigNumContext;
use openssl::ec::{EcGroup, EcKey, EcPoint};
use openssl::nid::Nid;
use openssl::pkey::{PKey, Public};
use openssl::x509::X509;

use crate::{RaTlsError, Result};

pub fn parse_cert(cert_bytes: &[u8], name: &str) -> Result<X509> {
    X509::from_pem(cert_bytes)
        .or_else(|_| X509::from_der(cert_bytes))
        .map_err(|err| {
            RaTlsError::InvalidData(format!("failed to parse {name} certificate: {err}"))
        })
}

pub fn verify_cert_validity(cert: &X509, name: &str) -> Result<()> {
    let now = Asn1Time::days_from_now(0)?;
    if now.compare(cert.not_before())? != Greater || now.compare(cert.not_after())? != Less {
        return Err(RaTlsError::InvalidData(format!(
            "{name} certificate is expired or not yet valid"
        )));
    }
    Ok(())
}

pub fn verify_cert_by_cert(issuer: &X509, subject: &X509, name: &str) -> Result<()> {
    let issuer_key = issuer.public_key()?;
    if !subject.verify(&issuer_key)? {
        return Err(RaTlsError::InvalidData(format!(
            "{name} certificate signature verification failed"
        )));
    }
    Ok(())
}

pub fn p521_public_key(raw_key: &[u8]) -> Result<PKey<Public>> {
    let group = EcGroup::from_curve_name(Nid::SECP521R1)?;
    let mut context = BigNumContext::new()?;
    let point = EcPoint::from_bytes(&group, raw_key, &mut context)?;
    let ec_key = EcKey::from_public_key(&group, &point)?;
    Ok(PKey::from_ec_key(ec_key)?)
}

pub fn cose_key_to_uncompressed_bytes(key: &CoseKey) -> Result<Vec<u8>> {
    if key.kty != KeyType::Assigned(iana::KeyType::EC2) {
        return Err(RaTlsError::InvalidData(
            "CCA Realm attestation key is not an EC2 key".into(),
        ));
    }
    let curve = key
        .params
        .iter()
        .find(|(label, _)| *label == Label::Int(-1))
        .and_then(|(_, value)| value.as_integer())
        .map(i128::from);
    // COSE curve identifier 3 is P-521.
    if curve != Some(3) {
        return Err(RaTlsError::InvalidData(
            "CCA Realm attestation key is not P-521".into(),
        ));
    }
    let x = key
        .params
        .iter()
        .find(|(label, _)| *label == Label::Int(-2))
        .and_then(|(_, value)| value.as_bytes())
        .ok_or_else(|| RaTlsError::InvalidData("CCA RAK is missing its x coordinate".into()))?;
    let y = key
        .params
        .iter()
        .find(|(label, _)| *label == Label::Int(-3))
        .and_then(|(_, value)| value.as_bytes())
        .ok_or_else(|| RaTlsError::InvalidData("CCA RAK is missing its y coordinate".into()))?;
    if x.len() != 66 || y.len() != 66 {
        return Err(RaTlsError::InvalidData(format!(
            "CCA P-521 RAK coordinates must be 66 bytes, got x={} and y={}",
            x.len(),
            y.len()
        )));
    }

    let mut uncompressed = Vec::with_capacity(133);
    uncompressed.push(0x04);
    uncompressed.extend_from_slice(x);
    uncompressed.extend_from_slice(y);
    Ok(uncompressed)
}

#[derive(Debug, Default)]
pub struct Decode;

impl Decode {
    pub fn get_bytes(value: &Value, name: &str, sizes: &[usize]) -> Result<Vec<u8>> {
        let bytes = value
            .as_bytes()
            .ok_or_else(|| RaTlsError::InvalidData(format!("{name} must be a byte string")))?
            .clone();
        if !sizes.is_empty() && !sizes.contains(&bytes.len()) {
            return Err(RaTlsError::InvalidData(format!(
                "{name} must contain {sizes:?} bytes, got {}",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    pub fn get_string(value: &Value, name: &str) -> Result<String> {
        Ok(value
            .as_text()
            .ok_or_else(|| RaTlsError::InvalidData(format!("{name} must be text")))?
            .trim_end_matches('\0')
            .to_string())
    }

    pub fn get_num(value: &Value, name: &str) -> Result<i128> {
        value
            .as_integer()
            .map(i128::from)
            .ok_or_else(|| RaTlsError::InvalidData(format!("{name} must be an integer")))
    }

    pub fn get_array(value: &Value, name: &str, size: Option<usize>) -> Result<Vec<Value>> {
        let array = value
            .as_array()
            .ok_or_else(|| RaTlsError::InvalidData(format!("{name} must be an array")))?
            .clone();
        if let Some(size) = size {
            if array.len() != size {
                return Err(RaTlsError::InvalidData(format!(
                    "{name} must contain {size} entries, got {}",
                    array.len()
                )));
            }
        }
        Ok(array)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::CoseKey;

    #[test]
    fn decode_helpers_validate_types_and_sizes() {
        assert!(Decode::get_bytes(&Value::Bytes(vec![1, 2]), "bytes", &[3]).is_err());
        assert!(Decode::get_string(&Value::Integer(1.into()), "text").is_err());
        assert!(Decode::get_num(&Value::Text("x".into()), "number").is_err());
        assert!(Decode::get_array(&Value::Array(vec![]), "array", Some(1)).is_err());
    }

    #[test]
    fn cose_key_conversion_rejects_a_non_ec2_key() {
        assert!(cose_key_to_uncompressed_bytes(&CoseKey::default()).is_err());
    }

    fn p521_cose_key(x: Vec<u8>, y: Vec<u8>) -> CoseKey {
        CoseKey {
            kty: KeyType::Assigned(iana::KeyType::EC2),
            params: vec![
                (Label::Int(-1), Value::Integer(3.into())),
                (Label::Int(-2), Value::Bytes(x)),
                (Label::Int(-3), Value::Bytes(y)),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn converts_valid_p521_cose_coordinates_and_checks_missing_fields() {
        let group = EcGroup::from_curve_name(Nid::SECP521R1).unwrap();
        let key = EcKey::generate(&group).unwrap();
        let mut context = BigNumContext::new().unwrap();
        let generated = key
            .public_key()
            .to_bytes(
                &group,
                openssl::ec::PointConversionForm::UNCOMPRESSED,
                &mut context,
            )
            .unwrap();
        let raw = cose_key_to_uncompressed_bytes(&p521_cose_key(
            generated[1..67].to_vec(),
            generated[67..].to_vec(),
        ))
        .unwrap();
        assert_eq!(raw.len(), 133);
        assert_eq!(raw[0], 4);
        assert!(p521_public_key(&raw).is_ok());

        let mut wrong_curve = p521_cose_key(vec![1; 66], vec![2; 66]);
        wrong_curve.params[0].1 = Value::Integer(2.into());
        assert!(cose_key_to_uncompressed_bytes(&wrong_curve).is_err());

        let mut missing_x = p521_cose_key(vec![1; 66], vec![2; 66]);
        missing_x.params.remove(1);
        assert!(cose_key_to_uncompressed_bytes(&missing_x).is_err());

        let mut missing_y = p521_cose_key(vec![1; 66], vec![2; 66]);
        missing_y.params.remove(2);
        assert!(cose_key_to_uncompressed_bytes(&missing_y).is_err());
        assert!(cose_key_to_uncompressed_bytes(&p521_cose_key(vec![1], vec![2])).is_err());
    }

    #[test]
    fn decode_helpers_accept_valid_values() {
        assert_eq!(
            Decode::get_bytes(&Value::Bytes(vec![1, 2]), "bytes", &[2]).unwrap(),
            [1, 2]
        );
        assert_eq!(
            Decode::get_string(&Value::Text("text\0".into()), "text").unwrap(),
            "text"
        );
        assert_eq!(
            Decode::get_num(&Value::Integer(7.into()), "number").unwrap(),
            7
        );
        assert_eq!(
            Decode::get_array(
                &Value::Array(vec![Value::Integer(1.into())]),
                "array",
                Some(1)
            )
            .unwrap()
            .len(),
            1
        );
    }
}
