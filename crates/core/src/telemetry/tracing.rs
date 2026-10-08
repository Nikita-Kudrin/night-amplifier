use super::{TelemetryConfig, TelemetryError};
use crate::error::Result;

#[cfg(feature = "telemetry")]
use opentelemetry::trace::TracerProvider as _;
#[cfg(feature = "telemetry")]
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Create an OpenTelemetry tracing layer for use with tracing-subscriber.
#[cfg(feature = "telemetry")]
pub fn create_telemetry_layer<S>(
    config: &TelemetryConfig,
) -> std::result::Result<
    Option<(
        tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>,
        SdkTracerProvider,
    )>,
    TelemetryError,
>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    if !config.enabled {
        return Ok(None);
    }

    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::trace::Sampler;
    use opentelemetry_sdk::Resource;

    // Create OTLP exporter
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()
        .map_err(|e| {
            TelemetryError::InitFailed(format!("Failed to create OTLP exporter: {}", e))
        })?;

    // Create tracer provider with resource attributes
    let resource = Resource::builder()
        .with_service_name(config.service_name.clone())
        .build();

    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::AlwaysOn)
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();

    // Get a tracer from the provider
    let tracer = provider.tracer(config.service_name.clone());

    // Create the tracing-opentelemetry layer
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    Ok(Some((layer, provider)))
}

#[cfg(not(feature = "telemetry"))]
pub fn create_telemetry_layer<S>(
    _config: &TelemetryConfig,
) -> std::result::Result<Option<()>, TelemetryError>
where
    S: tracing::Subscriber,
{
    Ok(None)
}
