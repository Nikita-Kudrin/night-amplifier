//! Logging configuration module
//!
//! Provides a flexible logging system with configurable log levels,
//! file rotation (5MB max), console output options, and optional
//! OpenTelemetry integration.
//!
//! # Example
//!
//! ```no_run
//! use night_amplifier::logging::{init_logging, LogConfig};
//!
//! // Default: INFO level, logs to ./logs directory
//! init_logging(LogConfig::default()).expect("Failed to initialize logging");
//!
//! // Or with custom configuration
//! let config = LogConfig::new()
//!     .with_level(tracing::Level::DEBUG)
//!     .with_log_dir("/var/log/night-amplifier")
//!     .with_console(true);
//! init_logging(config).expect("Failed to initialize logging");
//! ```

use std::path::PathBuf;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

#[cfg(feature = "telemetry")]
use crate::telemetry::{create_telemetry_layer, TelemetryConfig};
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Logging configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Minimum log level (default: INFO)
    pub level: Level,
    /// Directory for log files (default: "./logs")
    pub log_dir: PathBuf,
    /// Log file name prefix (default: "night-amplifier")
    pub file_prefix: String,
    /// Enable console output (default: true)
    pub console_output: bool,
    /// Enable file output (default: true)
    pub file_output: bool,
    /// Include source file and line in log output (default: false)
    pub include_location: bool,
    /// Include target (module path) in log output (default: true)
    pub include_target: bool,
    /// Include thread IDs in log output (default: false)
    pub include_thread_ids: bool,
    /// Include span events (default: false for production)
    pub include_span_events: bool,
    /// OpenTelemetry configuration (optional)
    #[cfg(feature = "telemetry")]
    pub telemetry: Option<TelemetryConfig>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: Level::INFO,
            log_dir: PathBuf::from("logs"),
            file_prefix: "night-amplifier".to_string(),
            console_output: true,
            file_output: true,
            include_location: false,
            include_target: true,
            include_thread_ids: false,
            include_span_events: false,
            #[cfg(feature = "telemetry")]
            telemetry: if TelemetryConfig::default_enabled() {
                Some(TelemetryConfig::default())
            } else {
                None
            },
        }
    }
}

impl LogConfig {
    /// Create a new LogConfig with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the minimum log level
    pub fn with_level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Set the log directory
    pub fn with_log_dir<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.log_dir = dir.into();
        self
    }

    /// Set the log file prefix
    pub fn with_file_prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.file_prefix = prefix.into();
        self
    }

    /// Enable or disable console output
    pub fn with_console(mut self, enabled: bool) -> Self {
        self.console_output = enabled;
        self
    }

    /// Enable or disable file output
    pub fn with_file(mut self, enabled: bool) -> Self {
        self.file_output = enabled;
        self
    }

    /// Include source file location in logs
    pub fn with_location(mut self, enabled: bool) -> Self {
        self.include_location = enabled;
        self
    }

    /// Include target (module path) in logs
    pub fn with_target(mut self, enabled: bool) -> Self {
        self.include_target = enabled;
        self
    }

    /// Include thread IDs in logs
    pub fn with_thread_ids(mut self, enabled: bool) -> Self {
        self.include_thread_ids = enabled;
        self
    }

    /// Include span events in logs
    pub fn with_span_events(mut self, enabled: bool) -> Self {
        self.include_span_events = enabled;
        self
    }

    /// Enable or disable OpenTelemetry telemetry
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry(mut self, config: Option<TelemetryConfig>) -> Self {
        self.telemetry = config;
        self
    }

    /// Enable OpenTelemetry with default configuration
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_enabled(mut self) -> Self {
        self.telemetry = Some(TelemetryConfig::default());
        self
    }

    /// Disable OpenTelemetry telemetry
    #[cfg(feature = "telemetry")]
    pub fn with_telemetry_disabled(mut self) -> Self {
        self.telemetry = None;
        self
    }

    /// Create a development configuration (DEBUG level, verbose output)
    pub fn development() -> Self {
        Self {
            level: Level::DEBUG,
            include_location: true,
            include_span_events: true,
            #[cfg(feature = "telemetry")]
            telemetry: Some(TelemetryConfig::default()),
            ..Default::default()
        }
    }

    /// Create a production configuration (INFO level, minimal output)
    pub fn production() -> Self {
        Self {
            level: Level::INFO,
            console_output: false,
            include_location: false,
            include_span_events: false,
            #[cfg(feature = "telemetry")]
            telemetry: None, // Disabled by default in production
            ..Default::default()
        }
    }
}

/// Guard that keeps the logging worker thread alive.
/// Must be held for the duration of the program.
pub struct LogGuard {
    _guards: Vec<WorkerGuard>,
    #[cfg(feature = "telemetry")]
    telemetry_provider: Option<SdkTracerProvider>,
}

#[cfg(feature = "telemetry")]
impl Drop for LogGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.telemetry_provider.take() {
            if let Err(e) = provider.shutdown() {
                eprintln!("Error shutting down telemetry provider: {:?}", e);
            }
        }
    }
}

/// Initialize the logging system with the given configuration.
///
/// Returns a guard that must be kept alive for the duration of the program.
/// When the guard is dropped, any pending log messages will be flushed.
///
/// # Example
///
/// ```no_run
/// use night_amplifier::logging::{init_logging, LogConfig};
///
/// fn main() {
///     let _guard = init_logging(LogConfig::default()).expect("logging init failed");
///     tracing::info!("Application started");
///     // ... rest of application
/// }
/// ```
pub fn init_logging(config: LogConfig) -> Result<LogGuard, LoggingError> {
    let mut guards = Vec::new();

    // Build the environment filter
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("night_amplifier={},{}", config.level, config.level))
    });

    // Determine span events
    let console_span_events = if config.include_span_events {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };
    let file_span_events = if config.include_span_events {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    // Build layers
    let registry = tracing_subscriber::registry();

    // Console layer
    let console_layer = if config.console_output {
        let layer = fmt::layer()
            .with_target(config.include_target)
            .with_file(config.include_location)
            .with_line_number(config.include_location)
            .with_thread_ids(config.include_thread_ids)
            .with_span_events(console_span_events)
            .with_ansi(true);
        Some(layer)
    } else {
        None
    };

    // File layer with rotation
    let file_layer = if config.file_output {
        // Create log directory
        std::fs::create_dir_all(&config.log_dir).map_err(|e| {
            LoggingError::InitFailed(format!(
                "Failed to create log directory {:?}: {}",
                config.log_dir, e
            ))
        })?;

        // Use tracing-appender's daily rotation as base, but we'll manage size ourselves
        let file_appender =
            RollingFileAppender::new(Rotation::DAILY, &config.log_dir, &config.file_prefix);

        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        guards.push(guard);

        let layer = fmt::layer()
            .with_writer(non_blocking)
            .with_target(config.include_target)
            .with_file(config.include_location)
            .with_line_number(config.include_location)
            .with_thread_ids(config.include_thread_ids)
            .with_span_events(file_span_events)
            .with_ansi(false); // No ANSI colors in files

        Some(layer)
    } else {
        None
    };

    // OpenTelemetry layer (when feature is enabled)
    #[cfg(feature = "telemetry")]
    let (telemetry_layer, telemetry_provider) = {
        if let Some(ref telemetry_config) = config.telemetry {
            match create_telemetry_layer(telemetry_config) {
                Ok(Some((layer, provider))) => (Some(layer), Some(provider)),
                Ok(None) => (None, None),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize telemetry: {}", e);
                    (None, None)
                }
            }
        } else {
            (None, None)
        }
    };

    // Initialize the subscriber with all layers
    #[cfg(feature = "telemetry")]
    {
        registry
            .with(env_filter)
            .with(console_layer)
            .with(file_layer)
            .with(telemetry_layer)
            .try_init()
            .map_err(|e| LoggingError::InitFailed(e.to_string()))?;
    }

    #[cfg(not(feature = "telemetry"))]
    {
        registry
            .with(env_filter)
            .with(console_layer)
            .with(file_layer)
            .try_init()
            .map_err(|e| LoggingError::InitFailed(e.to_string()))?;
    }

    Ok(LogGuard {
        _guards: guards,
        #[cfg(feature = "telemetry")]
        telemetry_provider,
    })
}

/// Initialize logging with default configuration.
///
/// This is a convenience function that uses `LogConfig::default()`.
pub fn init_default_logging() -> Result<LogGuard, LoggingError> {
    init_logging(LogConfig::default())
}

/// Error type for logging initialization
#[derive(Debug, Clone)]
pub enum LoggingError {
    /// Failed to initialize the logging system
    InitFailed(String),
}

impl std::fmt::Display for LoggingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoggingError::InitFailed(e) => write!(f, "Failed to initialize logging: {}", e),
        }
    }
}

impl std::error::Error for LoggingError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = LogConfig::new()
            .with_level(Level::DEBUG)
            .with_log_dir("/tmp/test-logs")
            .with_file_prefix("test")
            .with_console(false)
            .with_file(true)
            .with_location(true);

        assert_eq!(config.level, Level::DEBUG);
        assert_eq!(config.log_dir, PathBuf::from("/tmp/test-logs"));
        assert_eq!(config.file_prefix, "test");
        assert!(!config.console_output);
        assert!(config.file_output);
        assert!(config.include_location);
    }

    #[test]
    fn test_development_config() {
        let config = LogConfig::development();
        assert_eq!(config.level, Level::DEBUG);
        assert!(config.include_location);
        assert!(config.include_span_events);
    }

    #[test]
    fn test_production_config() {
        let config = LogConfig::production();
        assert_eq!(config.level, Level::INFO);
        assert!(!config.console_output);
        assert!(!config.include_location);
    }
}
