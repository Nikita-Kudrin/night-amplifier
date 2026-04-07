//! OpenTelemetry configuration module
//!
//! Provides optional OpenTelemetry integration for distributed tracing and metrics.

#[cfg(feature = "telemetry")]
use opentelemetry_sdk::metrics::SdkMeterProvider;
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::SdkTracerProvider;
#[cfg(feature = "telemetry")]
use std::sync::OnceLock;

pub mod metrics;
mod tracing;

pub use tracing::create_telemetry_layer;

#[cfg(feature = "telemetry")]
pub(crate) static METER_PROVIDER: OnceLock<SdkMeterProvider> = OnceLock::new();

/// Configuration for OpenTelemetry
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// OTLP endpoint URL
    pub endpoint: String,
    /// Service name for traces
    pub service_name: String,
    /// Whether telemetry is enabled
    pub enabled: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4317".to_string(),
            service_name: "night-amplifier".to_string(),
            enabled: Self::default_enabled(),
        }
    }
}

impl TelemetryConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn default_enabled() -> bool {
        cfg!(debug_assertions)
    }

    pub fn with_endpoint<S: Into<String>>(mut self, endpoint: S) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    pub fn with_service_name<S: Into<String>>(mut self, name: S) -> Self {
        self.service_name = name.into();
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
            config.endpoint = endpoint;
        }
        if let Ok(name) = std::env::var("OTEL_SERVICE_NAME") {
            config.service_name = name;
        }
        if let Ok(enabled) = std::env::var("OTEL_ENABLED") {
            config.enabled = enabled.to_lowercase() == "true" || enabled == "1";
        }
        config
    }
}

/// Guard that shuts down the OpenTelemetry tracer and meter when dropped
pub struct TelemetryGuard {
    #[cfg(feature = "telemetry")]
    _trace_provider: Option<SdkTracerProvider>,
    #[cfg(feature = "telemetry")]
    _meter_provider: Option<SdkMeterProvider>,
    #[cfg(not(feature = "telemetry"))]
    _phantom: std::marker::PhantomData<()>,
}

impl TelemetryGuard {
    #[cfg(feature = "telemetry")]
    fn new(
        trace_provider: Option<SdkTracerProvider>,
        meter_provider: Option<SdkMeterProvider>,
    ) -> Self {
        Self {
            _trace_provider: trace_provider,
            _meter_provider: meter_provider,
        }
    }

    #[cfg(not(feature = "telemetry"))]
    fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

#[cfg(feature = "telemetry")]
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        use opentelemetry::metrics::MeterProvider;
        use opentelemetry::trace::TracerProvider;
        if let Some(provider) = self._trace_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self._meter_provider.take() {
            let _ = provider.shutdown();
        }
    }
}

/// Error type for telemetry initialization
#[derive(Debug, Clone)]
pub enum TelemetryError {
    InitFailed(String),
    FeatureDisabled,
}

impl std::fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TelemetryError::InitFailed(e) => write!(f, "Failed to initialize telemetry: {}", e),
            TelemetryError::FeatureDisabled => {
                write!(f, "Telemetry feature not enabled at compile time")
            }
        }
    }
}

impl std::error::Error for TelemetryError {}

#[cfg(feature = "telemetry")]
pub fn init_telemetry(config: TelemetryConfig) -> Result<TelemetryGuard, TelemetryError> {
    if !config.enabled {
        return Ok(TelemetryGuard::new(None, None));
    }

    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::metrics::PeriodicReader;
    use opentelemetry_sdk::trace::Sampler;
    use opentelemetry_sdk::Resource;

    let resource = Resource::builder()
        .with_service_name(config.service_name.clone())
        .build();

    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()
        .map_err(|e| TelemetryError::InitFailed(e.to_string()))?;

    let trace_provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::AlwaysOn)
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter)
        .build();

    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()
        .map_err(|e| TelemetryError::InitFailed(e.to_string()))?;

    let metric_reader = PeriodicReader::builder(metric_exporter)
        .with_interval(std::time::Duration::from_secs(10))
        .build();

    let meter_provider = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(metric_reader)
        .build();

    let _ = METER_PROVIDER.set(meter_provider.clone());

    Ok(TelemetryGuard::new(
        Some(trace_provider),
        Some(meter_provider),
    ))
}

#[cfg(not(feature = "telemetry"))]
pub fn init_telemetry(config: TelemetryConfig) -> Result<TelemetryGuard, TelemetryError> {
    if config.enabled {
        return Err(TelemetryError::FeatureDisabled);
    }
    Ok(TelemetryGuard::new())
}

pub const fn is_telemetry_available() -> bool {
    cfg!(feature = "telemetry")
}

pub const fn is_telemetry_default_enabled() -> bool {
    cfg!(debug_assertions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = TelemetryConfig::new()
            .with_endpoint("http://localhost:9999")
            .with_service_name("test-service")
            .with_enabled(true);

        assert_eq!(config.endpoint, "http://localhost:9999");
        assert_eq!(config.service_name, "test-service");
        assert!(config.enabled);
    }
}
