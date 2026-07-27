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

//! CCEL and CCA measured boot log support.
//!
//! The sample uses this module to parse the CCEL ACPI table metadata, parse the
//! associated event log, replay registries 1/2 into CCA REM[0]/REM[1], and extract firmware
//! component measurements that can be checked against a JSON baseline.

use openssl::hash::{Hasher, MessageDigest};

use ratls_api::{hex_decode, RaTlsError, Result};

/// TPM algorithm identifier for SHA-1 digests.
pub const TPM_ALG_SHA1: u16 = 0x0004;
/// TPM algorithm identifier for SHA-256 digests.
pub const TPM_ALG_SHA256: u16 = 0x000b;
/// TPM algorithm identifier for SHA-384 digests.
pub const TPM_ALG_SHA384: u16 = 0x000c;
/// TPM algorithm identifier for SHA-512 digests.
pub const TPM_ALG_SHA512: u16 = 0x000d;
/// TPM event type used for the Spec ID entry.
pub const EV_NO_ACTION: u32 = 0x00000003;
/// Event type used by firmware loaders for IPL-related measurements.
pub const EV_IPL: u32 = 0x0000000d;
/// Base value for EFI event types.
pub const EV_EFI_EVENT_BASE: u32 = 0x80000000;
/// EFI boot services application event type.
pub const EV_EFI_BOOT_SERVICES_APPLICATION: u32 = EV_EFI_EVENT_BASE + 0x3;

/// Parsed CCEL ACPI table fields needed by the sample.
#[derive(Debug, Clone)]
pub struct CcelTable {
    /// ACPI table signature, expected to be `CCEL`.
    pub signature: [u8; 4],
    /// ACPI table revision.
    pub revision: u8,
    /// ACPI checksum byte.
    pub checksum: u8,
    /// OEM identifier from the ACPI header.
    pub oem_id: [u8; 6],
    /// Confidential computing log type.
    pub cc_type: u8,
    /// Confidential computing log subtype.
    pub cc_subtype: u8,
    /// Physical length of the event log advertised by firmware.
    pub log_length: u64,
    /// Physical address of the event log advertised by firmware.
    pub log_address: u64,
}

/// One parsed CCEL event log entry.
#[derive(Debug, Clone)]
pub struct EventLogEntry {
    /// Measurement register index associated with the event.
    pub registry: u32,
    /// TPM/EFI event type.
    pub event_type: u32,
    /// Digest list as `(algorithm_id, digest_bytes)` pairs.
    pub digests: Vec<(u16, Vec<u8>)>,
    /// Raw event payload.
    pub event: Vec<u8>,
}

/// Firmware measurements extracted from the event log.
#[derive(Debug, Clone, Default)]
pub struct FirmwareState {
    /// SHA-256 digests of EFI boot service application images.
    pub efi_images: Vec<Vec<u8>>,
    /// SHA-256 digest of the GRUB configuration, when found.
    pub grub_config: Option<Vec<u8>>,
    /// SHA-256 digest of the selected kernel image, when found.
    pub kernel: Option<Vec<u8>>,
    /// SHA-256 digest of the selected initramfs image, when found.
    pub initramfs: Option<Vec<u8>>,
}

/// Parse a raw `/sys/firmware/acpi/tables/CCEL` table.
pub fn parse_ccel_table(data: &[u8]) -> Result<CcelTable> {
    if data.len() < 56 || &data[..4] != b"CCEL" {
        return Err(RaTlsError::InvalidData("invalid CCEL ACPI table".into()));
    }
    let mut signature = [0u8; 4];
    signature.copy_from_slice(&data[..4]);
    let mut oem_id = [0u8; 6];
    oem_id.copy_from_slice(&data[10..16]);
    Ok(CcelTable {
        signature,
        revision: data[8],
        checksum: data[9],
        oem_id,
        cc_type: data[36],
        cc_subtype: data[37],
        log_length: read_le_u64(data, 40)?,
        log_address: read_le_u64(data, 48)?,
    })
}

/// Parse a binary CCA event log.
///
/// The parser expects a leading `Spec ID Event03` record followed by TPM 2.0
/// style event records with algorithm-tagged digest lists.
pub fn parse_event_log(data: &[u8]) -> Result<Vec<EventLogEntry>> {
    let mut cursor = Cursor::new(data);
    let mut entries = Vec::new();
    while cursor.remaining() > 0 {
        let entry_start = cursor.offset;
        let registry = cursor.u32()?;
        let event_type = cursor.u32()?;
        if (registry == u32::MAX && event_type == u32::MAX) || (registry == 0 && event_type == 0) {
            break;
        }

        if entries.is_empty() && event_type == EV_NO_ACTION {
            let _legacy_digest = cursor.bytes(20)?;
            let event_size = cursor.u32()? as usize;
            let event = cursor.bytes(event_size)?;
            if event_size < 29 || !event.starts_with(b"Spec ID Event03") {
                return Err(RaTlsError::InvalidData(format!(
                    "invalid Spec ID Event03 entry at offset {entry_start}"
                )));
            }
            entries.push(EventLogEntry {
                registry,
                event_type,
                digests: Vec::new(),
                event: event.to_vec(),
            });
            continue;
        }

        let digest_count = cursor.u32()?;
        if digest_count == 0 || digest_count > 16 {
            return Err(RaTlsError::InvalidData(format!(
                "invalid digest count at offset {entry_start}"
            )));
        }
        let mut digests = Vec::with_capacity(digest_count as usize);
        for _ in 0..digest_count {
            let alg = cursor.u16()?;
            let size = digest_size(alg).ok_or_else(|| {
                RaTlsError::InvalidData(format!("unsupported TPM digest algorithm {alg:#x}"))
            })?;
            digests.push((alg, cursor.bytes(size)?.to_vec()));
        }
        let event_size = cursor.u32()? as usize;
        let event = cursor.bytes(event_size)?.to_vec();
        entries.push(EventLogEntry {
            registry,
            event_type,
            digests,
            event,
        });
    }
    if entries.is_empty() {
        return Err(RaTlsError::InvalidData("CCA event log is empty".into()));
    }
    Ok(entries)
}

/// Replay event-log registries 1 and 2 and compare CCA REM[0] and REM[1].
///
/// Only SHA-256 digests are replayed. Entries for other REM indices are ignored
/// by this sample verifier.
pub fn replay_boot_registries_1_2(
    entries: &[EventLogEntry],
    expected_rem: &[Vec<u8>],
) -> Result<()> {
    let mut replay = [vec![0u8; 32], vec![0u8; 32]];
    let mut used = [false, false];
    for entry in entries {
        if !(1..=2).contains(&entry.registry) {
            continue;
        }
        let Some(digest) = entry
            .digests
            .iter()
            .find(|(alg, _)| *alg == TPM_ALG_SHA256)
            .map(|(_, digest)| digest)
        else {
            continue;
        };
        let idx = (entry.registry - 1) as usize;
        replay[idx] = sha256_two(&replay[idx], digest)?;
        used[idx] = true;
    }
    for idx in 0..2 {
        if !used[idx] || expected_rem.get(idx).map(Vec::as_slice) != Some(replay[idx].as_slice()) {
            return Err(RaTlsError::InvalidData(format!(
                "boot log registry {} replay does not match CCA REM[{idx}]",
                idx + 1,
            )));
        }
    }
    Ok(())
}

/// Copy CCA REM[0] and REM[1] for event-log registries 1 and 2.
pub fn verified_boot_rems_from_claims(claims: &serde_json::Value) -> Result<Vec<Vec<u8>>> {
    (0..2)
        .map(|index| {
            let name = format!("cca_realm_rem{index}");
            let encoded = claims
                .get(&name)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    RaTlsError::InvalidData(format!("verified CCA evidence is missing {name}"))
                })?;
            let measurement = hex_decode(encoded)?;
            if measurement.len() != 32 {
                return Err(RaTlsError::InvalidData(format!(
                    "{name} must be a SHA-256 measurement, got {} bytes",
                    measurement.len()
                )));
            }
            Ok(measurement)
        })
        .collect()
}

/// Replay the measured boot log before exposing firmware measurements to policy checks.
pub fn verify_and_extract_firmware_state(
    entries: &[EventLogEntry],
    expected_rem: &[Vec<u8>],
) -> Result<FirmwareState> {
    replay_boot_registries_1_2(entries, expected_rem)?;
    Ok(extract_firmware_state(entries))
}

/// Extract firmware component digests used by the firmware baseline checker.
pub fn extract_firmware_state(entries: &[EventLogEntry]) -> FirmwareState {
    let mut state = FirmwareState::default();
    for entry in entries {
        let Some(digest) = entry
            .digests
            .iter()
            .find(|(alg, _)| *alg == TPM_ALG_SHA256)
            .map(|(_, digest)| digest.clone())
        else {
            continue;
        };
        if entry.event_type == EV_EFI_BOOT_SERVICES_APPLICATION {
            state.efi_images.push(digest);
        } else if entry.event_type == EV_IPL {
            if state.grub_config.is_none() && contains(&entry.event, b"grub.cfg") {
                state.grub_config = Some(digest);
            } else if state.kernel.is_none()
                && contains(&entry.event, b"/vmlinuz-")
                && !contains(&entry.event, b"grub_cmd:")
            {
                state.kernel = Some(digest);
            } else if state.initramfs.is_none()
                && contains(&entry.event, b"/initramfs-")
                && !contains(&entry.event, b"grub_cmd:")
            {
                state.initramfs = Some(digest);
            }
        }
    }
    state
}

fn sha256_two(first: &[u8], second: &[u8]) -> Result<Vec<u8>> {
    let mut hasher = Hasher::new(MessageDigest::sha256())?;
    hasher.update(first)?;
    hasher.update(second)?;
    Ok(hasher.finish()?.to_vec())
}

fn digest_size(algorithm: u16) -> Option<usize> {
    match algorithm {
        TPM_ALG_SHA1 => Some(20),
        TPM_ALG_SHA256 => Some(32),
        TPM_ALG_SHA384 => Some(48),
        TPM_ALG_SHA512 => Some(64),
        _ => None,
    }
}

fn contains(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|w| w == needle)
}

fn read_le_u64(data: &[u8], offset: usize) -> Result<u64> {
    let end = offset + 8;
    if end > data.len() {
        return Err(RaTlsError::InvalidData("short u64 field".into()));
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[offset..end]);
    Ok(u64::from_le_bytes(bytes))
}

struct Cursor<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }

    fn u16(&mut self) -> Result<u16> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| RaTlsError::InvalidData("event log cursor overflow".into()))?;
        if end > self.data.len() {
            return Err(RaTlsError::InvalidData("truncated event log".into()));
        }
        let out = &self.data[self.offset..end];
        self.offset = end;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event_log(entries: &[(u32, u32, u16, Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&EV_NO_ACTION.to_le_bytes());
        data.extend_from_slice(&[0; 20]);
        let mut spec = b"Spec ID Event03".to_vec();
        spec.resize(29, 0);
        data.extend_from_slice(&(spec.len() as u32).to_le_bytes());
        data.extend_from_slice(&spec);
        for (registry, event_type, algorithm, digest, event) in entries {
            data.extend_from_slice(&registry.to_le_bytes());
            data.extend_from_slice(&event_type.to_le_bytes());
            data.extend_from_slice(&1u32.to_le_bytes());
            data.extend_from_slice(&algorithm.to_le_bytes());
            data.extend_from_slice(digest);
            data.extend_from_slice(&(event.len() as u32).to_le_bytes());
            data.extend_from_slice(event);
        }
        data
    }

    #[test]
    fn verified_boot_rem_claims_map_registries_to_rem_array_indexes() {
        let claims = json!({
            "cca_realm_rem0": "00".repeat(32),
            "cca_realm_rem1": "11".repeat(32),
            "cca_realm_rem2": "22".repeat(32),
        });

        let rems = verified_boot_rems_from_claims(&claims).unwrap();
        assert_eq!(rems, vec![vec![0x00; 32], vec![0x11; 32]]);
    }

    #[test]
    fn verified_boot_rem_claims_require_both_sha256_measurements() {
        let missing = json!({"cca_realm_rem0": "00".repeat(32)});
        assert!(verified_boot_rems_from_claims(&missing).is_err());

        let wrong_size = json!({
            "cca_realm_rem0": "00".repeat(31),
            "cca_realm_rem1": "11".repeat(32),
        });
        assert!(verified_boot_rems_from_claims(&wrong_size).is_err());
    }

    #[test]
    fn firmware_state_extraction_requires_a_matching_boot_log_replay() {
        let digest_1 = vec![0x11; 32];
        let digest_2 = vec![0x22; 32];
        let entries = vec![
            EventLogEntry {
                registry: 1,
                event_type: EV_IPL,
                digests: vec![(TPM_ALG_SHA256, digest_1.clone())],
                event: b"/vmlinuz-test".to_vec(),
            },
            EventLogEntry {
                registry: 2,
                event_type: EV_IPL,
                digests: vec![(TPM_ALG_SHA256, digest_2.clone())],
                event: b"/initramfs-test".to_vec(),
            },
        ];
        let expected = vec![
            sha256_two(&[0; 32], &digest_1).unwrap(),
            sha256_two(&[0; 32], &digest_2).unwrap(),
        ];

        let state = verify_and_extract_firmware_state(&entries, &expected).unwrap();
        assert_eq!(state.kernel, Some(digest_1));
        assert_eq!(state.initramfs, Some(digest_2));

        let mut tampered = entries;
        tampered[0].digests[0].1[0] ^= 0xff;
        assert!(verify_and_extract_firmware_state(&tampered, &expected).is_err());
    }

    #[test]
    fn parses_ccel_table_and_rejects_bad_headers() {
        let mut table = vec![0; 56];
        table[..4].copy_from_slice(b"CCEL");
        table[8] = 1;
        table[9] = 2;
        table[10..16].copy_from_slice(b"HUAWEI");
        table[36] = 3;
        table[37] = 4;
        table[40..48].copy_from_slice(&123u64.to_le_bytes());
        table[48..56].copy_from_slice(&456u64.to_le_bytes());
        let parsed = parse_ccel_table(&table).unwrap();
        assert_eq!(parsed.signature, *b"CCEL");
        assert_eq!(parsed.revision, 1);
        assert_eq!(parsed.checksum, 2);
        assert_eq!(parsed.oem_id, *b"HUAWEI");
        assert_eq!(parsed.cc_type, 3);
        assert_eq!(parsed.cc_subtype, 4);
        assert_eq!(parsed.log_length, 123);
        assert_eq!(parsed.log_address, 456);
        assert!(parse_ccel_table(&table[..55]).is_err());
        table[..4].copy_from_slice(b"XXXX");
        assert!(parse_ccel_table(&table).is_err());
    }

    #[test]
    fn parses_supported_digest_banks_and_rejects_malformed_logs() {
        let data = event_log(&[
            (1, EV_IPL, TPM_ALG_SHA1, vec![1; 20], b"sha1".to_vec()),
            (1, EV_IPL, TPM_ALG_SHA256, vec![2; 32], b"sha256".to_vec()),
            (1, EV_IPL, TPM_ALG_SHA384, vec![3; 48], b"sha384".to_vec()),
            (2, EV_IPL, TPM_ALG_SHA512, vec![4; 64], b"sha512".to_vec()),
        ]);
        let parsed = parse_event_log(&data).unwrap();
        assert_eq!(parsed.len(), 5);
        assert!(parsed[0].digests.is_empty());
        assert_eq!(parsed[4].digests[0].1.len(), 64);

        assert!(parse_event_log(&[]).is_err());
        assert!(parse_event_log(&data[..data.len() - 1]).is_err());
        let unsupported = event_log(&[(1, EV_IPL, 0xffff, vec![], vec![])]);
        assert!(parse_event_log(&unsupported).is_err());
    }

    #[test]
    fn extracts_all_firmware_components_and_requires_both_replayed_registries() {
        let entries = vec![
            EventLogEntry {
                registry: 1,
                event_type: EV_EFI_BOOT_SERVICES_APPLICATION,
                digests: vec![(TPM_ALG_SHA256, vec![1; 32])],
                event: b"EFI image".to_vec(),
            },
            EventLogEntry {
                registry: 1,
                event_type: EV_IPL,
                digests: vec![(TPM_ALG_SHA256, vec![2; 32])],
                event: b"/boot/grub.cfg".to_vec(),
            },
            EventLogEntry {
                registry: 1,
                event_type: EV_IPL,
                digests: vec![(TPM_ALG_SHA256, vec![3; 32])],
                event: b"/vmlinuz-test".to_vec(),
            },
            EventLogEntry {
                registry: 2,
                event_type: EV_IPL,
                digests: vec![(TPM_ALG_SHA256, vec![4; 32])],
                event: b"/initramfs-test".to_vec(),
            },
        ];
        let state = extract_firmware_state(&entries);
        assert_eq!(state.efi_images, [vec![1; 32]]);
        assert_eq!(state.grub_config, Some(vec![2; 32]));
        assert_eq!(state.kernel, Some(vec![3; 32]));
        assert_eq!(state.initramfs, Some(vec![4; 32]));
        assert!(replay_boot_registries_1_2(&entries[..3], &vec![vec![0; 32]; 2]).is_err());
    }
}
