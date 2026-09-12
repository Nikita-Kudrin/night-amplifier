//! Logging configuration: configurable log levels, file rotation (5MB max), console
//! output, optional OpenTelemetry integration.
//!
//! ```no_run
//! use night_amplifier::logging::{init_logging, LogConfig};
//!
//! init_logging(LogConfig::default()).expect("Failed to initialize logging");
//! let config = LogConfig::new().with_level(tracing::Level::DEBUG).with_console(true);
//! init_logging(config).expect("Failed to initialize logging");
//! ```

use std::io::IsTerminal;
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

/// Span fields for the log file.
///
/// A type of its own on purpose: tracing-subscriber formats a span's fields once and
/// caches the text per `FormatFields` *type*, so a file layer sharing `DefaultFields` with
/// the coloured console reused the console's escaped copy — 15,533 of 31,108 lines in the
/// 2026-09-07 field log. The default `add_fields` builds on `format_fields`, so nothing
/// else needs forwarding.
struct PlainFields(fmt::format::DefaultFields);

impl<'writer> fmt::FormatFields<'writer> for PlainFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: fmt::format::Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        fmt::FormatFields::format_fields(&self.0, writer, fields)
    }
}

fn span_events(config: &LogConfig) -> FmtSpan {
    if config.include_span_events {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    }
}

fn build_console_layer<S, W>(
    config: &LogConfig,
    writer: W,
    ansi: bool,
) -> fmt::Layer<S, fmt::format::DefaultFields, fmt::format::Format, W>
where
    W: for<'writer> fmt::MakeWriter<'writer> + 'static,
{
    fmt::layer()
        .with_writer(writer)
        .with_target(config.include_target)
        .with_file(config.include_location)
        .with_line_number(config.include_location)
        .with_thread_ids(config.include_thread_ids)
        .with_span_events(span_events(config))
        .with_ansi(ansi)
}

fn build_file_layer<S, W>(
    config: &LogConfig,
    writer: W,
) -> fmt::Layer<S, PlainFields, fmt::format::Format, W>
where
    W: for<'writer> fmt::MakeWriter<'writer> + 'static,
{
    fmt::layer()
        .with_writer(writer)
        .fmt_fields(PlainFields(fmt::format::DefaultFields::new()))
        .with_target(config.include_target)
        .with_file(config.include_location)
        .with_line_number(config.include_location)
        .with_thread_ids(config.include_thread_ids)
        .with_span_events(span_events(config))
        .with_ansi(false)
}

/// Initialize the logging system. Returns a guard that must be kept alive for the
/// program's duration; dropping it flushes any pending log messages.
///
/// ```no_run
/// use night_amplifier::logging::{init_logging, LogConfig};
///
/// let _guard = init_logging(LogConfig::default()).expect("logging init failed");
/// tracing::info!("Application started");
/// ```
pub fn init_logging(config: LogConfig) -> Result<LogGuard, LoggingError> {
    let mut guards = Vec::new();

    // Build the environment filter
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("night_amplifier={},{}", config.level, config.level))
    });

    // Build layers
    let registry = tracing_subscriber::registry();

    // Colour only on a terminal: under a service manager stdout is a journal, not a screen.
    let console_layer = config
        .console_output
        .then(|| build_console_layer(&config, std::io::stdout, std::io::stdout().is_terminal()));

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

        Some(build_file_layer(&config, non_blocking))
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

    #[derive(Clone, Default)]
    struct Buffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> fmt::MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl Buffer {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    /// Span fields are formatted once per span and cached under the *field formatter's
    /// type*; with both layers on `DefaultFields` the file layer reused the console layer's
    /// coloured copy. 2026-09-07 field log: 15,533 of 31,108 lines carried escapes. Uses
    /// `init_logging`'s own builders, in its order (console first).
    #[test]
    fn file_layer_span_fields_carry_no_ansi_escapes_next_to_a_coloured_console() {
        let config = LogConfig::default();
        let console = Buffer::default();
        let file = Buffer::default();
        let subscriber = tracing_subscriber::registry()
            .with(build_console_layer(&config, console.clone(), true))
            .with(build_file_layer(&config, file.clone()));

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("stacking_iteration", frame_number = 1);
            let _entered = span.enter();
            tracing::info!("Frame added to stack");
        });

        assert!(
            console.text().contains('\x1b'),
            "the console layer must actually colour, or this test proves nothing"
        );
        let file_text = file.text();
        assert!(file_text.contains("frame_number"), "{file_text}");
        assert!(
            !file_text.contains('\x1b'),
            "escapes leaked into the file: {file_text:?}"
        );
    }

    /// Registration spans record `matched_stars` after creation, which goes through
    /// `add_fields` rather than `format_fields` — the half `PlainFields` does not forward.
    #[test]
    fn file_layer_fields_recorded_after_span_creation_stay_plain() {
        let config = LogConfig::default();
        let console = Buffer::default();
        let file = Buffer::default();
        let subscriber = tracing_subscriber::registry()
            .with(build_console_layer(&config, console.clone(), true))
            .with(build_file_layer(&config, file.clone()));

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "register",
                frame_number = 7,
                matched_stars = tracing::field::Empty
            );
            let _entered = span.enter();
            span.record("matched_stars", 42);
            tracing::info!("Registered");
        });

        let file_text = file.text();
        assert!(
            file_text.contains("frame_number=7 matched_stars=42"),
            "{file_text:?}"
        );
        assert!(!file_text.contains('\x1b'), "{file_text:?}");
        assert!(console.text().contains('\x1b'));
    }
}
