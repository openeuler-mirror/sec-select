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

//! Minimal RATS-TLS style logger without external dependencies.

use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

/// RATS-TLS compatible log levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    /// Most verbose diagnostic messages.
    Debug = 0,
    /// Informational messages.
    Info = 1,
    /// Warnings that do not immediately fail the operation.
    Warn = 2,
    /// Operation failures.
    Error = 3,
    /// Fatal failures.
    Fatal = 4,
    /// Disable all logging.
    None = 5,
}

impl LogLevel {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Debug,
            1 => Self::Info,
            2 => Self::Warn,
            3 => Self::Error,
            4 => Self::Fatal,
            _ => Self::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
            Self::None => "NONE",
        }
    }
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Error as u8);

/// Set the process-wide RATS-TLS log level.
pub fn set_log_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Return the process-wide RATS-TLS log level.
pub fn log_level() -> LogLevel {
    LogLevel::from_u8(LOG_LEVEL.load(Ordering::Relaxed))
}

/// Print one RATS-TLS style log line if `level` is enabled.
pub fn log(level: LogLevel, module: &'static str, line: u32, args: fmt::Arguments<'_>) {
    if LOG_LEVEL.load(Ordering::Relaxed) > level as u8 {
        return;
    }
    match level {
        LogLevel::Error | LogLevel::Fatal => {
            eprintln!("[{}] {module}@L{line}: {args}", level.label());
        }
        LogLevel::Debug | LogLevel::Info | LogLevel::Warn => {
            println!("[{}] {module}@L{line}: {args}", level.label());
        }
        LogLevel::None => {}
    }
}

/// Log with an explicit RATS-TLS log level.
#[macro_export]
macro_rules! rtls_log {
    ($level:expr, $($arg:tt)*) => {
        $crate::logger::log($level, module_path!(), line!(), format_args!($($arg)*))
    };
}

/// Log a DEBUG message.
#[macro_export]
macro_rules! rtls_debug {
    ($($arg:tt)*) => {
        $crate::rtls_log!($crate::logger::LogLevel::Debug, $($arg)*)
    };
}

/// Log an INFO message.
#[macro_export]
macro_rules! rtls_info {
    ($($arg:tt)*) => {
        $crate::rtls_log!($crate::logger::LogLevel::Info, $($arg)*)
    };
}

/// Log a WARN message.
#[macro_export]
macro_rules! rtls_warn {
    ($($arg:tt)*) => {
        $crate::rtls_log!($crate::logger::LogLevel::Warn, $($arg)*)
    };
}

/// Log an ERROR message.
#[macro_export]
macro_rules! rtls_err {
    ($($arg:tt)*) => {
        $crate::rtls_log!($crate::logger::LogLevel::Error, $($arg)*)
    };
}

/// Log a FATAL message.
#[macro_export]
macro_rules! rtls_fatal {
    ($($arg:tt)*) => {
        $crate::rtls_log!($crate::logger::LogLevel::Fatal, $($arg)*)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_every_log_level_and_exercises_enabled_and_disabled_output() {
        assert_eq!(LogLevel::Debug.label(), "DEBUG");
        assert_eq!(LogLevel::Info.label(), "INFO");
        assert_eq!(LogLevel::Warn.label(), "WARN");
        assert_eq!(LogLevel::Error.label(), "ERROR");
        assert_eq!(LogLevel::Fatal.label(), "FATAL");
        assert_eq!(LogLevel::None.label(), "NONE");

        for level in [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Fatal,
            LogLevel::None,
        ] {
            set_log_level(level);
            assert_eq!(log_level(), level);
        }
        set_log_level(LogLevel::None);
        log(LogLevel::Debug, "test", 1, format_args!("hidden"));
        set_log_level(LogLevel::Debug);
        for level in [
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
            LogLevel::Fatal,
            LogLevel::None,
        ] {
            log(level, "test", 2, format_args!("visible"));
        }
        set_log_level(LogLevel::Error);
    }
}
