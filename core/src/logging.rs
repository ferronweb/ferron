//! Logging module supporting Windows Event Log and stdout/stderr backends.
//!
//! This module provides a unified logging interface that automatically routes
//! logs to the Windows Event Log when running as a Windows service, or to
//! stdout/stderr when running as a regular console application.

use std::io::IsTerminal;
use std::sync::{atomic::AtomicUsize, atomic::Ordering};

#[cfg(windows)]
use std::sync::Mutex;

/// Log levels
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl LogLevel {
    #[inline]
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
        }
    }

    #[inline]
    fn color_code(&self) -> &'static str {
        match self {
            LogLevel::Error => "\x1b[31m", // Red
            LogLevel::Warn => "\x1b[33m",  // Yellow
            LogLevel::Info => "\x1b[36m",  // Cyan
            LogLevel::Debug => "\x1b[35m", // Magenta
        }
    }
}

/// Logger backend types
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LoggerBackend {
    /// Windows Event Log (Windows only)
    #[cfg(windows)]
    EventLog,
    /// Standard output/error streams
    Stdio,
}

#[cfg(windows)]
struct WindowsEventSource {
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WindowsEventSource {
    fn new(source_name: &str) -> anyhow::Result<Self> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::System::EventLog::RegisterEventSourceW;

        let source_name_wide: Vec<u16> = source_name.encode_utf16().chain(Some(0)).collect();
        let handle =
            unsafe { RegisterEventSourceW(PCWSTR::null(), PCWSTR(source_name_wide.as_ptr())) }?;

        if handle == INVALID_HANDLE_VALUE {
            anyhow::bail!("Failed to register event source");
        }

        Ok(Self { handle })
    }

    fn log(&self, level: LogLevel, message: &str) {
        use windows::core::PCWSTR;
        use windows::Win32::System::EventLog::{
            ReportEventW, EVENTLOG_AUDIT_SUCCESS, EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE,
            EVENTLOG_WARNING_TYPE,
        };

        let event_type = match level {
            LogLevel::Error => EVENTLOG_ERROR_TYPE,
            LogLevel::Warn => EVENTLOG_WARNING_TYPE,
            LogLevel::Info => EVENTLOG_INFORMATION_TYPE,
            LogLevel::Debug => EVENTLOG_AUDIT_SUCCESS,
        };

        let message_wide: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let strings = [PCWSTR(message_wide.as_ptr())];

        let _ = unsafe {
            ReportEventW(
                self.handle,
                event_type,
                0,
                1000, // This will cause Windows Event Viewer to display fallback string.
                None,
                0,
                Some(&strings),
                None,
            )
        };
    }
}

#[cfg(windows)]
impl Drop for WindowsEventSource {
    fn drop(&mut self) {
        use windows::Win32::System::EventLog::DeregisterEventSource;
        let _ = unsafe { DeregisterEventSource(self.handle) };
    }
}

/// Global logger instance
pub struct AppLogger {
    backend: LoggerBackend,
    max_level: AtomicUsize,
    is_tty: bool,
    #[cfg(windows)]
    event_source: Mutex<Option<WindowsEventSource>>,
}

// Safety: HANDLE is a raw pointer wrapper that is safe to send between threads
#[cfg(windows)]
unsafe impl Send for WindowsEventSource {}
#[cfg(windows)]
unsafe impl Sync for WindowsEventSource {}

#[cfg(windows)]
unsafe impl Send for AppLogger {}
#[cfg(windows)]
unsafe impl Sync for AppLogger {}

static mut GLOBAL_LOGGER: Option<&'static AppLogger> = None;

/// Get the global logger instance
fn get_logger() -> Option<&'static AppLogger> {
    unsafe { GLOBAL_LOGGER }
}

/// Check if a logger is initialized
#[inline]
pub fn is_init() -> bool {
    get_logger().is_some()
}

/// Initialize the logger with the specified backend
pub fn init(backend: LoggerBackend, level: LogLevel) -> anyhow::Result<()> {
    let is_tty = if !matches!(backend, LoggerBackend::Stdio) {
        false
    } else {
        std::io::stdout().is_terminal()
    };
    let logger = Box::new(AppLogger {
        backend,
        max_level: AtomicUsize::new(level as usize),
        is_tty,
        #[cfg(windows)]
        event_source: Mutex::new(None),
    });

    let static_logger: &'static AppLogger = Box::leak(logger);

    unsafe {
        GLOBAL_LOGGER = Some(static_logger);
    }

    Ok(())
}

/// Initialize logger for Windows service (uses Event Log)
#[cfg(windows)]
pub fn init_service_logger(service_name: &str, level: LogLevel) -> anyhow::Result<()> {
    let logger = Box::new(AppLogger {
        backend: LoggerBackend::EventLog,
        max_level: AtomicUsize::new(level as usize),
        is_tty: false, // Windows Event Log isn't a TTY
        event_source: Mutex::new(Some(WindowsEventSource::new(service_name)?)),
    });

    let static_logger: &'static AppLogger = Box::leak(logger);

    unsafe {
        GLOBAL_LOGGER = Some(static_logger);
    }

    Ok(())
}

/// Initialize logger for console application (uses stdout/stderr)
pub fn init_stdio_logger(level: LogLevel) -> anyhow::Result<()> {
    let is_tty = std::io::stdout().is_terminal();
    let logger = Box::new(AppLogger {
        backend: LoggerBackend::Stdio,
        max_level: AtomicUsize::new(level as usize),
        is_tty,
        #[cfg(windows)]
        event_source: Mutex::new(None),
    });

    let static_logger: &'static AppLogger = Box::leak(logger);

    unsafe {
        GLOBAL_LOGGER = Some(static_logger);
    }

    Ok(())
}

impl AppLogger {
    fn log(&self, level: LogLevel, message: &str) {
        if level as usize > self.max_level.load(Ordering::Relaxed) {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let formatted = if self.is_tty {
            let color = level.color_code();
            let reset = "\x1b[0m";
            format!(
                "{}[{} {}]{} {}",
                color,
                timestamp,
                level.as_str(),
                reset,
                message
            )
        } else {
            format!("[{} {}] {}", timestamp, level.as_str(), message)
        };

        match self.backend {
            LoggerBackend::Stdio => {
                if level == LogLevel::Error {
                    eprintln!("{}", formatted);
                } else {
                    println!("{}", formatted);
                }
            }
            #[cfg(windows)]
            LoggerBackend::EventLog => {
                if let Ok(source_guard) = self.event_source.lock() {
                    if let Some(source) = source_guard.as_ref() {
                        source.log(level, &formatted);
                    }
                }
            }
        }
    }

    fn flush(&self) {
        use std::io::{stderr, stdout, Write};
        let _ = stdout().flush();
        let _ = stderr().flush();
    }

    #[inline]
    fn set_max_level(&self, level: LogLevel) {
        self.max_level.store(level as usize, Ordering::Relaxed);
    }
}

/// Set the maximum log level
pub fn set_max_level(level: LogLevel) {
    if let Some(logger) = get_logger() {
        logger.set_max_level(level);
    }
}

/// Get the current maximum log level
#[inline]
pub fn max_level() -> LogLevel {
    get_logger()
        .and_then(|logger| match logger.max_level.load(Ordering::Relaxed) {
            0 => Some(LogLevel::Error),
            1 => Some(LogLevel::Warn),
            2 => Some(LogLevel::Info),
            3 => Some(LogLevel::Debug),
            _ => None,
        })
        .unwrap_or(LogLevel::Error)
}

/// Check if a log level is enabled
#[inline]
pub fn enabled(level: LogLevel) -> bool {
    level <= max_level()
}

/// Internal log function used by macros
#[inline]
pub fn log(level: LogLevel, message: &str) {
    if let Some(logger) = get_logger() {
        logger.log(level, message);
    }
}

/// Flush the logger
#[inline]
pub fn flush() {
    if let Some(logger) = get_logger() {
        logger.flush();
    }
}

/// Logging macro for info-level messages
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::logging::enabled($crate::logging::LogLevel::Info) {
            $crate::logging::log($crate::logging::LogLevel::Info, &format!($($arg)*));
        }
    };
}

/// Logging macro for error-level messages
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        if $crate::logging::enabled($crate::logging::LogLevel::Error) {
            $crate::logging::log($crate::logging::LogLevel::Error, &format!($($arg)*));
        }
    };
}

/// Logging macro for debug-level messages
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if $crate::logging::enabled($crate::logging::LogLevel::Debug) {
            $crate::logging::log($crate::logging::LogLevel::Debug, &format!($($arg)*));
        }
    };
}

/// Logging macro for warning-level messages
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if $crate::logging::enabled($crate::logging::LogLevel::Warn) {
            $crate::logging::log($crate::logging::LogLevel::Warn, &format!($($arg)*));
        }
    };
}
