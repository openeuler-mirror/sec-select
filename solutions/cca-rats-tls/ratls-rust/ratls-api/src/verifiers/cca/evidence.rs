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

use openssl::x509::X509;

use super::token::CCAToken;
use super::tools::{parse_cert, verify_cert_by_cert, verify_cert_validity};
use super::trust_anchors::{HUAWEI_EQUIPMENT_ROOT_CA_PEM, HUAWEI_PRODUCT_CA_PEM};
use crate::{RaTlsError, Result};

/// Raw CCA evidence carried inside the DICE evidence buffer.
///
/// This matches the native RATS-TLS layout:
/// `size_t token_len || token || size_t cert_len || device_cert`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcaEvidence {
    pub token: Vec<u8>,
    pub dev_cert: Vec<u8>,
}

impl CcaEvidence {
    pub fn encode_raw_native(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(
            std::mem::size_of::<usize>() * 2 + self.token.len() + self.dev_cert.len(),
        );
        encoded.extend_from_slice(&self.token.len().to_ne_bytes());
        encoded.extend_from_slice(&self.token);
        encoded.extend_from_slice(&self.dev_cert.len().to_ne_bytes());
        encoded.extend_from_slice(&self.dev_cert);
        encoded
    }

    pub fn decode_raw_native(raw: &[u8]) -> Result<Self> {
        let word_size = std::mem::size_of::<usize>();
        if raw.len() < word_size * 2 {
            return Err(RaTlsError::InvalidData(
                "CCA evidence is shorter than its length headers".into(),
            ));
        }

        let token_len = read_native_usize(&raw[..word_size]);
        if token_len == 0 || token_len > raw.len() - word_size * 2 {
            return Err(RaTlsError::InvalidData(
                "CCA evidence contains an invalid token length".into(),
            ));
        }
        let cert_len_offset = word_size
            .checked_add(token_len)
            .ok_or_else(|| RaTlsError::InvalidData("CCA token length overflows".into()))?;
        let cert_data_offset = cert_len_offset
            .checked_add(word_size)
            .ok_or_else(|| RaTlsError::InvalidData("CCA certificate offset overflows".into()))?;
        if cert_data_offset > raw.len() {
            return Err(RaTlsError::InvalidData(
                "CCA evidence is truncated before the certificate length".into(),
            ));
        }
        let cert_len = read_native_usize(&raw[cert_len_offset..cert_data_offset]);
        if cert_len == 0 || cert_len != raw.len() - cert_data_offset {
            return Err(RaTlsError::InvalidData(
                "CCA evidence contains an invalid device certificate length".into(),
            ));
        }

        Ok(Self {
            token: raw[word_size..cert_len_offset].to_vec(),
            dev_cert: raw[cert_data_offset..].to_vec(),
        })
    }
}

fn read_native_usize(bytes: &[u8]) -> usize {
    let mut value = [0_u8; std::mem::size_of::<usize>()];
    value.copy_from_slice(bytes);
    usize::from_ne_bytes(value)
}

/// CCA token and device certificate extracted from RATS-TLS evidence.
#[derive(Debug)]
pub struct CCAEvidence {
    pub cca_token: Vec<u8>,
    pub dev_cert: Vec<u8>,
}

impl CCAEvidence {
    pub fn from_raw(raw: &[u8]) -> Result<Self> {
        let evidence = CcaEvidence::decode_raw_native(raw)?;
        Ok(Self {
            cca_token: evidence.token,
            dev_cert: evidence.dev_cert,
        })
    }

    fn verify_cert_chain(&self) -> Result<X509> {
        let device_cert = parse_cert(&self.dev_cert, "device")?;
        let product_ca = X509::from_pem(HUAWEI_PRODUCT_CA_PEM).map_err(|err| {
            RaTlsError::InvalidData(format!("failed to parse product CA certificate: {err}"))
        })?;
        let root_ca = X509::from_pem(HUAWEI_EQUIPMENT_ROOT_CA_PEM).map_err(|err| {
            RaTlsError::InvalidData(format!("failed to parse equipment root certificate: {err}"))
        })?;

        verify_cert_validity(&device_cert, "device")?;
        verify_cert_validity(&product_ca, "product CA")?;
        verify_cert_validity(&root_ca, "equipment root")?;
        verify_cert_by_cert(&product_ca, &device_cert, "device")?;
        verify_cert_by_cert(&root_ca, &product_ca, "product CA")?;
        verify_cert_by_cert(&root_ca, &root_ca, "equipment root")?;
        Ok(device_cert)
    }

    /// Verify the fixed certificate chain and both CCA COSE_Sign1 tokens.
    pub fn crypto_verification(&self, nonce: Option<&[u8]>) -> Result<CCAToken> {
        let device_cert = self.verify_cert_chain()?;
        let cca_token = CCAToken::parse(&self.cca_token)?;
        cca_token.verify_platform_token(&device_cert)?;
        cca_token.verify_realm_token(nonce)?;
        Ok(cca_token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_native_evidence() {
        let raw = CcaEvidence {
            token: vec![1, 2],
            dev_cert: vec![3, 4],
        }
        .encode_raw_native();
        let evidence = CCAEvidence::from_raw(&raw).unwrap();
        assert_eq!(evidence.cca_token, vec![1, 2]);
        assert_eq!(evidence.dev_cert, vec![3, 4]);
    }

    #[test]
    fn native_evidence_rejects_trailing_or_empty_data() {
        assert!(CcaEvidence::decode_raw_native(&[]).is_err());

        let mut encoded = CcaEvidence {
            token: vec![1],
            dev_cert: vec![2],
        }
        .encode_raw_native();
        encoded.push(3);
        assert!(CcaEvidence::decode_raw_native(&encoded).is_err());
    }

    #[test]
    fn pinned_product_ca_chains_to_the_pinned_root() {
        let product_ca = X509::from_pem(HUAWEI_PRODUCT_CA_PEM).unwrap();
        let root_ca = X509::from_pem(HUAWEI_EQUIPMENT_ROOT_CA_PEM).unwrap();

        verify_cert_validity(&product_ca, "product CA").unwrap();
        verify_cert_validity(&root_ca, "equipment root").unwrap();
        verify_cert_by_cert(&root_ca, &product_ca, "product CA").unwrap();
        verify_cert_by_cert(&root_ca, &root_ca, "equipment root").unwrap();
    }
}
