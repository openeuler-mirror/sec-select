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

//! Shared validation for the sample command-line programs.

use std::fmt::Display;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{self, ExitCode};

use ratls_api::{RaTlsError, Result};

pub const MAX_LOG_SIZE: usize = 10 * 1024 * 1024;
pub const DEFAULT_MAX_KEY_SIZE: usize = 64 * 1024;
pub const MAX_KEY_SIZE: usize = 1024 * 1024;
pub const MAX_MESSAGE_SIZE: usize = 4096;
pub const MAX_JSON_POLICY_SIZE: usize = 1024 * 1024;
pub const MAX_DIGEST_POLICY_SIZE: usize = 10 * 1024 * 1024;

/// Print a clap diagnostic with the sample's standard severity prefix, then exit.
pub fn exit_with_clap_error(error: clap::Error) -> ! {
    if matches!(
        error.kind(),
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
    ) {
        error.exit();
    }
    let exit_code = error.exit_code();
    eprint!("{}", format_clap_error(&error));
    process::exit(exit_code);
}

/// Report a sample command's final result using the standard log prefix.
pub fn report_command_result(result: Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", format_command_error(&error));
            ExitCode::FAILURE
        }
    }
}

fn format_clap_error(error: &clap::Error) -> String {
    let rendered = error.to_string().replacen("error:", "[ERROR]", 1);
    if error.kind() != clap::error::ErrorKind::MissingRequiredArgument {
        return rendered;
    }
    let Some((diagnostic, remainder)) = rendered.split_once("\n\n") else {
        return rendered;
    };
    let diagnostic = diagnostic
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{diagnostic}\n\n{remainder}")
}

fn format_command_error(error: &impl Display) -> String {
    format!("[ERROR] {error}")
}

pub fn parse_port(value: &str) -> std::result::Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| "port must be an integer from 1 to 65535".to_string())?;
    if port == 0 {
        return Err("port must be an integer from 1 to 65535".into());
    }
    Ok(port)
}

pub fn parse_max_log(value: &str) -> std::result::Result<usize, String> {
    parse_bounded_size(value, "max-log", MAX_LOG_SIZE)
}

pub fn parse_max_key(value: &str) -> std::result::Result<usize, String> {
    parse_bounded_size(value, "max-key", MAX_KEY_SIZE)
}

pub fn parse_rim(value: &str) -> std::result::Result<String, String> {
    if value.len() < 2
        || value.len() > 128
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("RIM must contain 2 to 128 hexadecimal characters and have even length".into());
    }
    Ok(value.to_owned())
}

pub fn validate_input_file(path: &Path, name: &str, maximum: usize) -> Result<()> {
    let metadata = input_file_metadata(path, name)?;
    let length = usize::try_from(metadata.len()).map_err(|_| {
        RaTlsError::InvalidArgument(format!("{name} '{}' is too large", path.display()))
    })?;
    if length == 0 || length > maximum {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} '{}' must contain 1 to {maximum} bytes",
            path.display()
        )));
    }
    Ok(())
}

pub fn read_input_file(path: &Path, name: &str, maximum: usize) -> Result<Vec<u8>> {
    input_file_metadata(path, name)?;
    let file = fs::File::open(path).map_err(|error| {
        RaTlsError::InvalidArgument(format!(
            "failed to read {name} '{}': {error}",
            path.display()
        ))
    })?;
    let read_limit = u64::try_from(maximum)
        .ok()
        .and_then(|limit| limit.checked_add(1))
        .unwrap_or(u64::MAX);
    let mut data = Vec::new();
    file.take(read_limit)
        .read_to_end(&mut data)
        .map_err(|error| {
            RaTlsError::InvalidArgument(format!(
                "failed to read {name} '{}': {error}",
                path.display()
            ))
        })?;
    if data.is_empty() || data.len() > maximum {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} '{}' must contain 1 to {maximum} bytes",
            path.display()
        )));
    }
    Ok(data)
}

fn input_file_metadata(path: &Path, name: &str) -> Result<fs::Metadata> {
    let metadata = fs::metadata(path).map_err(|error| {
        RaTlsError::InvalidArgument(format!(
            "failed to inspect {name} '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} '{}' is not a regular file",
            path.display()
        )));
    }
    Ok(metadata)
}

pub fn validate_output_parent(path: &Path, name: &str) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} path must not be empty"
        )));
    }
    if fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(RaTlsError::InvalidArgument(format!(
            "{name} '{}' is a directory",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let Some(parent) = parent else {
        return Ok(());
    };
    let metadata = fs::metadata(parent).map_err(|error| {
        RaTlsError::InvalidArgument(format!(
            "failed to inspect parent directory for {name} '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Err(RaTlsError::InvalidArgument(format!(
            "parent path for {name} '{}' is not a directory",
            path.display()
        )));
    }
    Ok(())
}

fn parse_bounded_size(
    value: &str,
    name: &str,
    maximum: usize,
) -> std::result::Result<usize, String> {
    let size = value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be an integer from 1 to {maximum}"))?;
    if size == 0 || size > maximum {
        return Err(format!("{name} must be an integer from 1 to {maximum}"));
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ratls-cli-test-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_a_virtual_file_with_zero_metadata_size() {
        let path = Path::new("/proc/self/status");
        assert_eq!(fs::metadata(path).unwrap().len(), 0);

        let data = read_input_file(path, "virtual file", 64 * 1024).unwrap();

        assert!(!data.is_empty());
    }

    #[test]
    fn parses_ports_sizes_and_rim_boundaries() {
        assert_eq!(parse_port("1").unwrap(), 1);
        assert_eq!(parse_port("65535").unwrap(), 65535);
        assert!(parse_port("0").is_err());
        assert!(parse_port("65536").is_err());
        assert!(parse_port("text").is_err());

        assert_eq!(parse_max_log("1").unwrap(), 1);
        assert_eq!(
            parse_max_log(&MAX_LOG_SIZE.to_string()).unwrap(),
            MAX_LOG_SIZE
        );
        assert!(parse_max_log("0").is_err());
        assert!(parse_max_log(&(MAX_LOG_SIZE + 1).to_string()).is_err());
        assert_eq!(parse_max_key("1").unwrap(), 1);
        assert!(parse_max_key(&(MAX_KEY_SIZE + 1).to_string()).is_err());

        assert_eq!(parse_rim("00").unwrap(), "00");
        assert!(parse_rim("").is_err());
        assert!(parse_rim("0").is_err());
        assert!(parse_rim("xyz0").is_err());
        assert!(parse_rim(&"00".repeat(65)).is_err());
    }

    #[test]
    fn command_errors_use_the_standard_error_prefix() {
        let runtime_error = RaTlsError::InvalidData("bad firmware baseline".into());
        assert_eq!(
            format_command_error(&runtime_error),
            "[ERROR] invalid data: bad firmware baseline"
        );

        let clap_error =
            clap::Command::new("sample").error(clap::error::ErrorKind::InvalidValue, "bad option");
        let rendered = format_clap_error(&clap_error);
        assert!(rendered.starts_with("[ERROR] bad option"), "{rendered}");
        assert!(!rendered.contains("error:"), "{rendered}");

        let missing_arguments = clap::Command::new("sample")
            .arg(
                clap::Arg::new("bootlog")
                    .long("bootlog")
                    .action(clap::ArgAction::SetTrue)
                    .required(true),
            )
            .try_get_matches_from(["sample"])
            .unwrap_err();
        let rendered = format_clap_error(&missing_arguments);
        assert!(
            rendered.starts_with(
                "[ERROR] the following required arguments were not provided: --bootlog\n\nUsage:"
            ),
            "{rendered}"
        );
    }

    #[test]
    fn validates_input_and_output_paths_and_actual_read_length() {
        let directory = temporary_directory();
        let input = directory.join("input");
        fs::write(&input, b"abc").unwrap();
        assert!(validate_input_file(&input, "input", 3).is_ok());
        assert_eq!(read_input_file(&input, "input", 3).unwrap(), b"abc");
        assert!(validate_input_file(&input, "input", 2).is_err());
        assert!(read_input_file(&input, "input", 2).is_err());

        let empty = directory.join("empty");
        fs::write(&empty, []).unwrap();
        assert!(validate_input_file(&empty, "input", 3).is_err());
        assert!(read_input_file(&empty, "input", 3).is_err());
        assert!(validate_input_file(&directory, "input", 3).is_err());
        assert!(validate_input_file(&directory.join("missing"), "input", 3).is_err());

        assert!(validate_output_parent(&directory.join("output"), "output").is_ok());
        assert!(validate_output_parent(&directory, "output").is_err());
        assert!(
            validate_output_parent(&directory.join("missing-parent").join("output"), "output")
                .is_err()
        );
        assert!(validate_output_parent(Path::new(""), "output").is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
