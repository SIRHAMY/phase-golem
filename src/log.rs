use std::sync::atomic::{AtomicU8, Ordering};

/// Log levels for orchestrator output, ordered by verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

pub fn set_log_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn current_log_level() -> LogLevel {
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Error,
        1 => LogLevel::Warn,
        2 => LogLevel::Info,
        _ => LogLevel::Debug,
    }
}

/// Parse a log level string. Returns `Err` with a message for invalid input.
pub fn parse_log_level(s: &str) -> Result<LogLevel, String> {
    match s.to_lowercase().as_str() {
        "error" => Ok(LogLevel::Error),
        "warn" => Ok(LogLevel::Warn),
        "info" => Ok(LogLevel::Info),
        "debug" => Ok(LogLevel::Debug),
        _ => Err(format!(
            "Invalid log level '{}': expected error, warn, info, or debug",
            s
        )),
    }
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        eprintln!($($arg)*)
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if $crate::log::current_log_level() >= $crate::log::LogLevel::Warn {
            eprintln!($($arg)*)
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::log::current_log_level() >= $crate::log::LogLevel::Info {
            eprintln!($($arg)*)
        }
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if $crate::log::current_log_level() >= $crate::log::LogLevel::Debug {
            eprintln!($($arg)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_level() {
        assert_eq!(parse_log_level("error").unwrap(), LogLevel::Error);
        assert_eq!(parse_log_level("warn").unwrap(), LogLevel::Warn);
        assert_eq!(parse_log_level("info").unwrap(), LogLevel::Info);
        assert_eq!(parse_log_level("debug").unwrap(), LogLevel::Debug);
        assert_eq!(parse_log_level("INFO").unwrap(), LogLevel::Info);
        assert!(parse_log_level("invalid").is_err());
    }

    #[test]
    fn test_set_and_get_log_level() {
        // Note: tests share the global, so just verify round-trip
        set_log_level(LogLevel::Debug);
        assert_eq!(current_log_level(), LogLevel::Debug);
        set_log_level(LogLevel::Error);
        assert_eq!(current_log_level(), LogLevel::Error);
        // Restore default for other tests
        set_log_level(LogLevel::Info);
    }

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
    }
}
