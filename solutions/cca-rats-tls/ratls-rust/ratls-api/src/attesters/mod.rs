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

//! Attester adapter trait and built-in implementations.

/// CCA attester implementation.
pub mod cca;

use crate::attesters::cca::CcaAttester;
use crate::core::evidence::AttestationEvidence;
use crate::RaTlsError;
use std::fmt;

/// Trait implemented by evidence collection backends.
pub trait Attester: Send {
    fn name(&self) -> &'static str;
    fn evidence_tag(&self) -> u64;
    fn collect_evidence(&self, challenge: &[u8]) -> Result<AttestationEvidence, RaTlsError>;
}

pub struct AttesterRegistry;

impl AttesterRegistry {
    /// Load an attester implementation.
    pub fn load(plugin: AttesterPlugin) -> Result<Box<dyn Attester>, RaTlsError> {
        match plugin {
            AttesterPlugin::Cca => Ok(Box::new(CcaAttester::default())),
        }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum AttesterPlugin {
    Cca = 1,
}

impl fmt::Display for AttesterPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            AttesterPlugin::Cca => "Cca",
        };
        f.write_str(name)
    }
}

impl TryFrom<u32> for AttesterPlugin {
    type Error = RaTlsError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Cca),
            other => Err(RaTlsError::InvalidArgument(format!(
                "invalid Attester plugin value: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_c_selector_expose_the_cca_attester() {
        let attester = AttesterRegistry::load(AttesterPlugin::Cca).unwrap();
        assert_eq!(attester.name(), "cca");
        assert_eq!(AttesterPlugin::Cca.to_string(), "Cca");
        assert!(matches!(
            AttesterPlugin::try_from(1).unwrap(),
            AttesterPlugin::Cca
        ));
        assert!(AttesterPlugin::try_from(0).is_err());
    }
}
