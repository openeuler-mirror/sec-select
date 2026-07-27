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

use std::ffi::OsString;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, ValueEnum};
use ratls_api::api::{rats_tls_init, rats_tls_negotiate, rats_tls_set_verification_callback};
use ratls_api::attesters::AttesterPlugin;
use ratls_api::core::{RaTlsConf, RaTlsHandle};
use ratls_api::crypto_wrappers::CertAlgorithm;
use ratls_api::logger::{set_log_level, LogLevel};
use ratls_api::rtls_debug;
use ratls_api::tls_wrappers::TransportStream;
use ratls_api::RaTlsError::{InvalidArgument, InvalidData};
use ratls_api::{hex_decode, RaTlsError, Result};
use ratls_sample::common::cli::{
    self, read_input_file, validate_input_file, MAX_DIGEST_POLICY_SIZE, MAX_JSON_POLICY_SIZE,
    MAX_KEY_SIZE, MAX_MESSAGE_SIZE,
};
use ratls_sample::common::frame::{receive_frame, send_frame};
use ratls_sample::common::tls_config::{CertificateOptions, TlsVerifyOptions};
use ratls_sample::common::{event_log, firmware_policy, ima_log, platform_policy};

const REQUEST_IMA_LOG: &[u8] = b"REQUEST_IMA_LOG";
const REQUEST_CCEL_TABLE: &[u8] = b"REQUEST_CCEL_TABLE";
const REQUEST_EVENT_LOG: &[u8] = b"REQUEST_EVENT_LOG";
const ENABLE_FDE_TOKEN: &[u8] = b"ENABLE_FDE_TOKEN";
const ATTESTATION_PASS: &[u8] = b"ATTESTATION_PASS";

type SharedVerifiedRems = Arc<Mutex<Option<Vec<Vec<u8>>>>>;

fn main() -> Result<()> {
    let args = Args::parse_validated();
    set_log_level(args.log_level.into());
    rtls_debug!(
        "sample client log level set to {:?}, target={}:{} mutual={}",
        args.log_level,
        args.ip,
        args.port,
        args.mutual
    );
    let conf = RaTlsConf {
        attester: args.mutual.then_some(AttesterPlugin::Cca),
        cert_algo: args.cert_algo.into(),
        mutual: args.mutual,
        certificate: args.certificate.load()?,
        tls_verify: args.tls_verify.load()?,
        ..Default::default()
    };
    let mut handle = rats_tls_init(conf)?;
    let verified_boot_rems = configure_verification_callback(&mut handle, &args)?;
    let stream = TcpStream::connect(SocketAddr::new(args.ip, args.port))?;
    let mut stream = rats_tls_negotiate(&mut handle, Box::new(stream))?;

    if args.ima_log {
        send_frame(stream.as_mut(), REQUEST_IMA_LOG)?;
        let data = receive_frame(stream.as_mut(), args.max_log)?;
        let entries = ima_log::parse_binary_ima_log(&data)?;
        if let Some(path) = &args.digest {
            ima_log::ImaDigestBaseline::load(path)?.verify_entries(&entries)?;
        }
        eprintln!(
            "[INFO] received IMA log {} bytes, parsed {} entries",
            data.len(),
            entries.len()
        );
    }

    let mut firmware_state = None;
    if args.boot_log {
        send_frame(stream.as_mut(), REQUEST_CCEL_TABLE)?;
        let ccel = receive_frame(stream.as_mut(), args.max_log)?;
        let parsed = event_log::parse_ccel_table(&ccel)?;
        eprintln!(
            "[INFO] received CCEL table: log_length={}, log_address=0x{:x}",
            parsed.log_length, parsed.log_address
        );

        send_frame(stream.as_mut(), REQUEST_EVENT_LOG)?;
        let event_log_data = receive_frame(stream.as_mut(), args.max_log)?;
        let entries = event_log::parse_event_log(&event_log_data)?;
        let expected_rems = verified_boot_rems
            .as_ref()
            .ok_or_else(|| InvalidData("boot log verification state is unavailable".into()))?
            .lock()
            .map_err(|_| InvalidData("verified REM state is poisoned".into()))?
            .clone()
            .ok_or_else(|| {
                InvalidData("verified CCA evidence did not provide boot REM values".into())
            })?;
        firmware_state = Some(event_log::verify_and_extract_firmware_state(
            &entries,
            &expected_rems,
        )?);
        eprintln!("[INFO] boot event-log registries 1/2 match verified CCA REM[0]/REM[1]");
        eprintln!(
            "[INFO] received event log {} bytes, parsed {} entries",
            event_log_data.len(),
            entries.len()
        );
    }

    if let Some(path) = &args.firmware {
        let Some(state) = &firmware_state else {
            return Err(InvalidArgument(
                "--firmware requires --bootlog so event log can be verified".into(),
            ));
        };
        firmware_policy::verify_firmware_baseline(path, state)?;
        eprintln!("[INFO] firmware baseline JSON verified");
    }

    if let Some(path) = &args.fde_key {
        send_rootfs_key(stream.as_mut(), path)?;
    }

    let message = args.message_payload()?;
    send_frame(stream.as_mut(), &message)?;
    let response = receive_frame(stream.as_mut(), 4096)?;
    println!("CCA server echoed {} bytes", response.len());

    send_frame(stream.as_mut(), ATTESTATION_PASS)?;
    let ack = receive_frame(stream.as_mut(), 4096)?;
    if ack != ATTESTATION_PASS {
        return Err(InvalidData("server did not ack pass".into()));
    }
    Ok(())
}

fn send_rootfs_key(stream: &mut dyn TransportStream, path: &str) -> Result<()> {
    let key = read_input_file(Path::new(path), "FDE key", MAX_KEY_SIZE)?;
    send_frame(stream, ENABLE_FDE_TOKEN)?;
    send_frame(stream, &key)?;
    eprintln!("[INFO] sent rootfs key {} bytes from {}", key.len(), path);
    Ok(())
}

#[derive(Parser, Debug)]
#[command(name = "cca-client", about = "CCA RA-TLS Rust sample client")]
struct Args {
    /// Server IPv4 or IPv6 address. DNS names are not accepted.
    #[arg(long, short = 'i', default_value = "127.0.0.1")]
    ip: IpAddr,

    /// Server TCP port, from 1 to 65535.
    #[arg(long, short = 'p', default_value_t = 1234, value_parser = cli::parse_port)]
    port: u16,

    /// UTF-8 echo message, from 1 to 4096 bytes.
    #[arg(
        long,
        short = 'M',
        default_value = "hello CCA",
        conflicts_with = "message_file"
    )]
    message: String,

    /// Binary echo-message file, from 1 to 4096 bytes.
    #[arg(long)]
    message_file: Option<String>,

    /// Enable mutual RA-TLS so the client also sends CCA evidence.
    #[arg(long, short = 'm')]
    mutual: bool,

    /// Request and parse the server IMA measurement log.
    #[arg(long, short = 'I')]
    ima_log: bool,

    /// Request the CCEL table and measured boot event log.
    #[arg(long = "bootlog", short = 'g')]
    boot_log: bool,

    /// Verify boot measurements against a firmware baseline JSON file.
    #[arg(long, short = 'f', requires = "boot_log")]
    firmware: Option<String>,

    /// Verify the IMA log against a digest baseline file.
    #[arg(long, short = 'd', requires = "ima_log")]
    digest: Option<String>,

    /// Require the verified peer CCA RIM to equal this hexadecimal value.
    #[arg(long, value_parser = cli::parse_rim)]
    rim: Option<String>,

    /// Verify peer CCA platform software components against a JSON policy.
    #[arg(long, short = 'P')]
    platform: Option<String>,

    /// Send a rootfs/FDE key file after the RA-TLS handshake succeeds.
    #[arg(long = "fdekey", alias = "fde-key", short = 'k')]
    fde_key: Option<String>,

    /// Maximum accepted IMA or boot-log frame size, up to 10 MiB.
    #[arg(
        long,
        default_value_t = cli::MAX_LOG_SIZE,
        value_parser = cli::parse_max_log
    )]
    max_log: usize,

    /// Dynamic RA-TLS leaf-key algorithm.
    #[arg(long, value_enum, default_value_t = SampleCertAlgorithm::Ecc256)]
    cert_algo: SampleCertAlgorithm,

    #[command(flatten)]
    certificate: CertificateOptions,

    #[command(flatten)]
    tls_verify: TlsVerifyOptions,

    /// Process log level.
    #[arg(long, short = 'l', value_enum, default_value_t = SampleLogLevel::Error)]
    log_level: SampleLogLevel,
}

impl Args {
    fn parse_validated() -> Self {
        Self::try_parse_validated_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }

    fn try_parse_validated_from<I, T>(arguments: I) -> std::result::Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let args = Self::try_parse_from(arguments)?;
        args.validate().map_err(|error| {
            Self::command().error(ErrorKind::ValueValidation, error.to_string())
        })?;
        Ok(args)
    }

    fn validate(&self) -> Result<()> {
        self.tls_verify.validate_cli(false, self.mutual)?;
        if let Some(path) = &self.message_file {
            read_input_file(Path::new(path), "message file", MAX_MESSAGE_SIZE)?;
        } else if self.message.is_empty() || self.message.len() > MAX_MESSAGE_SIZE {
            return Err(InvalidArgument(format!(
                "--message must contain 1 to {MAX_MESSAGE_SIZE} UTF-8 bytes"
            )));
        }
        if let Some(path) = &self.firmware {
            validate_input_file(Path::new(path), "firmware baseline", MAX_JSON_POLICY_SIZE)?;
            firmware_policy::load_firmware_baseline(path)?;
        }
        if let Some(path) = &self.digest {
            validate_input_file(
                Path::new(path),
                "IMA digest baseline",
                MAX_DIGEST_POLICY_SIZE,
            )?;
            ima_log::ImaDigestBaseline::load(path)?;
        }
        if let Some(path) = &self.platform {
            validate_input_file(Path::new(path), "platform policy", MAX_JSON_POLICY_SIZE)?;
            platform_policy::validate_platform_policy(path)?;
        }
        if let Some(path) = &self.fde_key {
            read_input_file(Path::new(path), "FDE key", MAX_KEY_SIZE)?;
        }
        Ok(())
    }

    fn message_payload(&self) -> Result<Vec<u8>> {
        match &self.message_file {
            Some(path) => read_input_file(Path::new(path), "message file", MAX_MESSAGE_SIZE),
            None => Ok(self.message.as_bytes().to_vec()),
        }
    }
}

fn configure_verification_callback(
    handle: &mut RaTlsHandle,
    args: &Args,
) -> Result<Option<SharedVerifiedRems>> {
    let expected_rim = args.rim.as_deref().map(hex_decode).transpose()?;
    let platform_policy = args.platform.clone();
    let verified_boot_rems = args
        .boot_log
        .then(|| Arc::new(Mutex::new(None::<Vec<Vec<u8>>>)));
    if expected_rim.is_none() && platform_policy.is_none() && verified_boot_rems.is_none() {
        return Ok(None);
    }
    let callback_boot_rems = verified_boot_rems.clone();

    rats_tls_set_verification_callback(
        handle,
        Some(Box::new(move |evidence| {
            if let Some(expected) = expected_rim.as_ref() {
                let actual = evidence
                    .evidence_json
                    .get("cca_realm_rim")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        RaTlsError::InvalidData("verified evidence is missing CCA RIM".into())
                    })
                    .and_then(hex_decode)?;
                if &actual != expected {
                    return Err(RaTlsError::InvalidData(format!(
                        "RIM verification failed: expected {} bytes, got {} bytes",
                        expected.len(),
                        actual.len()
                    )));
                }
                eprintln!("[INFO] RIM verification passed");
            }

            if let Some(path) = platform_policy.as_deref() {
                platform_policy::verify_platform_policy(path, &evidence.evidence_json)?;
                eprintln!("[INFO] platform SW-components verification passed");
            }

            if let Some(boot_rems) = callback_boot_rems.as_ref() {
                let measurements =
                    event_log::verified_boot_rems_from_claims(&evidence.evidence_json)?;
                *boot_rems
                    .lock()
                    .map_err(|_| InvalidData("verified REM state is poisoned".into()))? =
                    Some(measurements);
            }

            Ok(())
        })),
    );
    Ok(verified_boot_rems)
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SampleLogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
    None,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SampleCertAlgorithm {
    Rsa3072,
    Ecc256,
}

impl From<SampleCertAlgorithm> for CertAlgorithm {
    fn from(algorithm: SampleCertAlgorithm) -> Self {
        match algorithm {
            SampleCertAlgorithm::Rsa3072 => Self::Rsa3072Sha256,
            SampleCertAlgorithm::Ecc256 => Self::Ecc256Sha256,
        }
    }
}

impl From<SampleLogLevel> for LogLevel {
    fn from(level: SampleLogLevel) -> Self {
        match level {
            SampleLogLevel::Debug => Self::Debug,
            SampleLogLevel::Info => Self::Info,
            SampleLogLevel::Warn => Self::Warn,
            SampleLogLevel::Error => Self::Error,
            SampleLogLevel::Fatal => Self::Fatal,
            SampleLogLevel::None => Self::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_default_client_command() {
        let args = Args::try_parse_validated_from(["cca-client"]).unwrap();
        assert_eq!(args.ip, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(args.port, 1234);
        assert_eq!(args.max_log, cli::MAX_LOG_SIZE);
    }

    #[test]
    fn rejects_invalid_network_values() {
        assert!(Args::try_parse_validated_from(["cca-client", "--ip", "localhost"]).is_err());
        assert!(Args::try_parse_validated_from(["cca-client", "--port", "0"]).is_err());
    }

    #[test]
    fn rejects_conflicting_messages_and_invalid_rim() {
        assert!(Args::try_parse_validated_from([
            "cca-client",
            "--message",
            "text",
            "--message-file",
            "message.bin",
        ])
        .is_err());
        assert!(Args::try_parse_validated_from(["cca-client", "--rim", "not-hex"]).is_err());
    }

    #[test]
    fn validates_tls_flag_relationships() {
        assert!(Args::try_parse_validated_from(["cca-client", "--use-system-ca"]).is_err());
        assert!(
            Args::try_parse_validated_from(["cca-client", "--verify-peer-certificate"]).is_err()
        );
        assert!(Args::try_parse_validated_from([
            "cca-client",
            "--verify-peer-certificate",
            "--use-system-ca",
        ])
        .is_ok());
    }
}
