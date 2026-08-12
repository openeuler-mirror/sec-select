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

//! Length-prefixed sample transport frames.
//!
//! RA-TLS exposes a byte stream after negotiation. The sample protocol adds a
//! four-byte big-endian payload length before each application blob so the
//! receiver can distinguish individual log and message payloads.

use ratls_api::api::{rats_tls_receive, rats_tls_transmit};
use ratls_api::tls_wrappers::TransportStream;
use ratls_api::{RaTlsError, Result};

/// Send one framed payload over an established RA-TLS handle.
///
/// The frame format is:
///
/// ```text
/// u32_be payload_len
/// u8[payload_len] payload
/// ```
///
/// Empty payloads are rejected because the sample uses frames only for
/// required data items.
pub fn send_frame(stream: &mut dyn TransportStream, payload: &[u8]) -> Result<()> {
    if payload.is_empty() || payload.len() > u32::MAX as usize {
        return Err(RaTlsError::InvalidArgument("invalid frame size".into()));
    }
    let size = (payload.len() as u32).to_be_bytes();
    transmit_all(stream, &size)?;
    transmit_all(stream, payload)?;
    Ok(())
}

/// Receive one framed payload, enforcing a caller supplied maximum size.
///
/// The size limit keeps a peer from forcing unbounded allocation before policy
/// verification has completed.
pub fn receive_frame(stream: &mut dyn TransportStream, max: usize) -> Result<Vec<u8>> {
    let mut size = [0u8; 4];
    receive_all(stream, &mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > max {
        return Err(RaTlsError::InvalidData(format!(
            "invalid frame size: {size}"
        )));
    }
    let mut payload = vec![0u8; size];
    receive_all(stream, &mut payload)?;
    Ok(payload)
}

fn transmit_all(stream: &mut dyn TransportStream, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let written = rats_tls_transmit(stream, data)?;
        if written == 0 || written > data.len() {
            return Err(RaTlsError::InvalidData("short/invalid transmit".into()));
        }
        data = &data[written..];
    }
    Ok(())
}

fn receive_all(stream: &mut dyn TransportStream, mut data: &mut [u8]) -> Result<()> {
    while !data.is_empty() {
        let read = rats_tls_receive(stream, data)?;
        if read == 0 || read > data.len() {
            return Err(RaTlsError::InvalidData("short/invalid receive".into()));
        }
        let tmp = data;
        data = &mut tmp[read..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn frame_round_trip_preserves_binary_payload() {
        let mut writer = Cursor::new(Vec::new());
        send_frame(&mut writer, b"\0hello\xff").unwrap();
        assert_eq!(&writer.get_ref()[..4], &7u32.to_be_bytes());

        let mut reader = Cursor::new(writer.into_inner());
        assert_eq!(receive_frame(&mut reader, 7).unwrap(), b"\0hello\xff");
    }

    #[test]
    fn rejects_empty_oversized_and_truncated_frames() {
        assert!(send_frame(&mut Cursor::new(Vec::new()), b"").is_err());

        let mut empty = Cursor::new(0u32.to_be_bytes().to_vec());
        assert!(receive_frame(&mut empty, 10).is_err());

        let mut oversized = Cursor::new(11u32.to_be_bytes().to_vec());
        assert!(receive_frame(&mut oversized, 10).is_err());

        let mut truncated = Cursor::new([3u32.to_be_bytes().as_slice(), b"ab"].concat());
        assert!(receive_frame(&mut truncated, 10).is_err());
    }
}
