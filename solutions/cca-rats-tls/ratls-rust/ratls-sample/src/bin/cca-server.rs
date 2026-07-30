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
use std::fs;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::Path;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, ValueEnum};
use ratls_api::api::{rats_tls_init, rats_tls_negotiate, rats_tls_set_verification_callback};
use ratls_api::core::{RaTlsConf, RaTlsHandle};
use ratls_api::crypto_wrappers::CertAlgorithm;
use ratls_api::logger::{set_log_level, LogLevel};
use ratls_api::rtls_debug;
use ratls_api::tls_wrappers::TransportStream;
use ratls_api::verifiers::VerifierPlugin;
use ratls_api::{hex_decode, RaTlsError, Result};
use ratls_sample::common::cli::{
    self, read_input_file, validate_input_file, validate_output_parent, MAX_JSON_POLICY_SIZE,
    MAX_LOG_SIZE,
};
use ratls_sample::common::frame::{receive_frame, send_frame};
use ratls_sample::common::platform_policy;
use ratls_sample::common::tls_config::{CertificateOptions, TlsVerifyOptions};

const REQUEST_IMA_LOG: &[u8] = b"REQUEST_IMA_LOG";
const REQUEST_CCEL_TABLE: &[u8] = b"REQUEST_CCEL_TABLE";
const REQUEST_EVENT_LOG: &[u8] = b"REQUEST_EVENT_LOG";
const ENABLE_FDE_TOKEN: &[u8] = b"ENABLE_FDE_TOKEN";
const ATTESTATION_PASS: &[u8] = b"ATTESTATION_PASS";

fn main() -> Result<()> {
    let args = Args::parse_validated();
    set_log_level(args.log_level.into());
    let certificate = args.certificate.load()?;
    let tls_verify = args.tls_verify.load()?;
    drop(rats_tls_init(session_conf(
        &args,
        &certificate,
        &tls_verify,
    ))?);
    rtls_debug!(
        "sample server log level set to {:?}, listen={}:{} mutual={}",
        args.log_level,
        args.ip,
        args.port,
        args.mutual
    );
    let listener = TcpListener::bind(SocketAddr::new(args.ip, args.port))?;
    eprintln!(
        "[INFO] CCA RA-TLS Rust server listening on {}:{}",
        args.ip, args.port
    );
    for stream in listener.incoming() {
        let stream = stream?;
        serve_one(stream, &args, &certificate, &tls_verify)?;
        if args.once {
            break;
        }
    }
    Ok(())
}

fn serve_one(
    stream: std::net::TcpStream,
    args: &Args,
    certificate: &ratls_api::core::CertificateConf,
    tls_verify: &ratls_api::core::TlsVerifyConf,
) -> Result<()> {
    let conf = session_conf(args, certificate, tls_verify);
    let mut handle = rats_tls_init(conf)?;
    configure_verification_callback(&mut handle, args)?;
    let mut stream = rats_tls_negotiate(&mut handle, Box::new(stream))?;
    loop {
        let request = receive_frame(stream.as_mut(), 4096)?;
        match request.as_slice() {
            REQUEST_IMA_LOG => send_file(stream.as_mut(), &args.ima_log)?,
            REQUEST_CCEL_TABLE => send_file(stream.as_mut(), &args.ccel_table)?,
            REQUEST_EVENT_LOG => send_file(stream.as_mut(), &args.event_log)?,
            ENABLE_FDE_TOKEN => receive_rootfs_key(stream.as_mut(), args)?,
            ATTESTATION_PASS => {
                send_frame(stream.as_mut(), ATTESTATION_PASS)?;
                break;
            }
            other => {
                eprintln!("[INFO] received client message {} bytes", other.len());
                send_frame(stream.as_mut(), other)?;
            }
        }
    }
    Ok(())
}

fn session_conf(
    args: &Args,
    certificate: &ratls_api::core::CertificateConf,
    tls_verify: &ratls_api::core::TlsVerifyConf,
) -> RaTlsConf {
    RaTlsConf {
        cert_algo: args.cert_algo.into(),
        mutual: args.mutual,
        server: true,
        verifier: args.mutual.then_some(VerifierPlugin::Cca),
        certificate: certificate.clone(),
        tls_verify: tls_verify.clone(),
        ..Default::default()
    }
}

fn receive_rootfs_key(stream: &mut dyn TransportStream, args: &Args) -> Result<()> {
    let key = receive_frame(stream, args.max_key)?;
    fs::write(&args.rootfs_key, &key)?;
    eprintln!(
        "[INFO] saved rootfs key {} bytes to {}",
        key.len(),
        args.rootfs_key
    );
    Ok(())
}

fn send_file(stream: &mut dyn TransportStream, path: &str) -> Result<()> {
    let data = read_input_file(Path::new(path), "measurement log", MAX_LOG_SIZE)?;
    send_frame(stream, &data)
}

#[derive(Parser, Debug)]
#[command(name = "cca-server", about = "CCA RA-TLS Rust sample server")]
struct Args {
    /// Local IPv4 or IPv6 listen address.
    #[arg(long, short = 'i', default_value = "127.0.0.1")]
    ip: IpAddr,

    /// Local TCP listen port, from 1 to 65535.
    #[arg(long, short = 'p', default_value_t = 1234, value_parser = cli::parse_port)]
    port: u16,

    /// Exit after serving one connection.
    #[arg(long, short = '1')]
    once: bool,

    /// Enable mutual RA-TLS and verify client CCA evidence.
    #[arg(long, short = 'm')]
    mutual: bool,

    /// IMA binary runtime measurement log returned on client request.
    #[arg(
        long,
        default_value = "/sys/kernel/security/ima/binary_runtime_measurements"
    )]
    ima_log: String,

    /// CCEL ACPI table returned on client request.
    #[arg(long, default_value = "/sys/firmware/acpi/tables/CCEL")]
    ccel_table: String,

    /// CCEL measured boot event log returned on client request.
    #[arg(
        long,
        alias = "boot-log",
        default_value = "/sys/firmware/acpi/tables/data/CCEL"
    )]
    event_log: String,

    /// Output path used when a client sends a rootfs/FDE key.
    #[arg(long, default_value = "/root/rootfs_key.bin")]
    rootfs_key: String,

    /// Require the verified client CCA RIM to equal this hexadecimal value.
    #[arg(long, requires = "mutual", value_parser = cli::parse_rim)]
    rim: Option<String>,

    /// Verify client CCA platform software components against a JSON policy.
    #[arg(long, short = 'P', requires = "mutual")]
    platform: Option<String>,

    /// Maximum accepted rootfs/FDE key frame size, up to 1 MiB.
    #[arg(
        long,
        default_value_t = cli::DEFAULT_MAX_KEY_SIZE,
        value_parser = cli::parse_max_key
    )]
    max_key: usize,

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
        self.tls_verify.validate_cli(true, self.mutual)?;
        if let Some(path) = &self.platform {
            validate_input_file(Path::new(path), "platform policy", MAX_JSON_POLICY_SIZE)?;
            platform_policy::validate_platform_policy(path)?;
        }
        validate_output_parent(Path::new(&self.rootfs_key), "rootfs key output")?;
        Ok(())
    }
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

fn configure_verification_callback(handle: &mut RaTlsHandle, args: &Args) -> Result<()> {
    let expected_rim = args.rim.as_deref().map(hex_decode).transpose()?;
    let platform_policy = args.platform.clone();
    if expected_rim.is_none() && platform_policy.is_none() {
        return Ok(());
    }

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

            Ok(())
        })),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_default_server_command() {
        let args = Args::try_parse_validated_from(["cca-server"]).unwrap();
        assert_eq!(args.ip, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(args.port, 1234);
        assert_eq!(args.max_key, cli::DEFAULT_MAX_KEY_SIZE);
    }

    #[test]
    fn rejects_invalid_network_and_size_values() {
        assert!(Args::try_parse_validated_from(["cca-server", "--ip", "localhost"]).is_err());
        assert!(Args::try_parse_validated_from(["cca-server", "--port", "0"]).is_err());
        assert!(Args::try_parse_validated_from(["cca-server", "--max-key", "1048577",]).is_err());
    }

    #[test]
    fn server_tls_verification_requires_mutual_and_a_trust_source() {
        assert!(Args::try_parse_validated_from([
            "cca-server",
            "--verify-peer-certificate",
            "--use-system-ca",
        ])
        .is_err());
        assert!(Args::try_parse_validated_from([
            "cca-server",
            "--mutual",
            "--verify-peer-certificate",
        ])
        .is_err());
        assert!(Args::try_parse_validated_from([
            "cca-server",
            "--mutual",
            "--verify-peer-certificate",
            "--use-system-ca",
        ])
        .is_ok());
    }
}
