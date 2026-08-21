//! Application entry point shared between Community and Pro binaries.
//!
//! Provides `run()` which handles argument parsing, logging setup, and server startup.
//! Pro callers pass a plugin registration closure; Community passes nothing.

use crate::logging::{init_logging, LogConfig};
use crate::server::{Server, ServerConfig};
#[cfg(feature = "telemetry")]
use crate::telemetry::TelemetryConfig;
use std::net::SocketAddr;
use tracing::{error, info, warn};

pub static APP_VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Command line arguments
struct Args {
    port: u16,
    #[cfg(feature = "telemetry")]
    telemetry: bool,
    #[cfg(feature = "telemetry")]
    otlp_endpoint: Option<String>,
    indi_host: Option<String>,
    indi_port: Option<u16>,
    span_timings: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1);
        let mut port = if std::path::Path::new("Cargo.toml").exists() || std::path::Path::new("web/index.html").exists() {
            9955u16
        } else {
            8844u16
        };
        #[cfg(feature = "telemetry")]
        let mut telemetry = TelemetryConfig::default_enabled();
        #[cfg(feature = "telemetry")]
        let mut otlp_endpoint = None;
        let mut indi_host = None;
        let mut indi_port = None;
        let mut span_timings = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    Self::print_help();
                    std::process::exit(0);
                }
                "--span-timings" => span_timings = true,
                #[cfg(feature = "telemetry")]
                "--telemetry" => telemetry = true,
                #[cfg(feature = "telemetry")]
                "--no-telemetry" => telemetry = false,
                #[cfg(feature = "telemetry")]
                "--otlp-endpoint" => {
                    otlp_endpoint = args.next();
                    if otlp_endpoint.is_none() {
                        eprintln!("Error: --otlp-endpoint requires a value");
                        std::process::exit(1);
                    }
                }
                "--indi-host" => {
                    indi_host = args.next();
                    if indi_host.is_none() {
                        eprintln!("Error: --indi-host requires a value");
                        std::process::exit(1);
                    }
                }
                "--indi-port" => {
                    if let Some(port_str) = args.next() {
                        if let Ok(p) = port_str.parse::<u16>() {
                            indi_port = Some(p);
                        } else {
                            eprintln!("Error: Invalid port for --indi-port: {}", port_str);
                            std::process::exit(1);
                        }
                    } else {
                        eprintln!("Error: --indi-port requires a value");
                        std::process::exit(1);
                    }
                }
                _ => {
                    if let Ok(p) = arg.parse::<u16>() {
                        port = p;
                    } else if !arg.starts_with('-') {
                        eprintln!("Warning: Unknown argument: {}", arg);
                    } else {
                        eprintln!("Error: Unknown option: {}", arg);
                        Self::print_help();
                        std::process::exit(1);
                    }
                }
            }
        }

        Self {
            port,
            #[cfg(feature = "telemetry")]
            telemetry,
            #[cfg(feature = "telemetry")]
            otlp_endpoint,
            indi_host,
            indi_port,
            span_timings,
        }
    }

    fn print_help() {
        println!("Night Amplifier - EAA Live Stacking Server");
        println!();
        println!("Usage: night_amplifier [OPTIONS] [PORT]");
        println!();
        println!("Arguments:");
        println!("  [PORT]              Server port (default: 9955, or 8844 in distribution binary)");
        println!();
        println!("Options:");
        println!("  -h, --help          Show this help message");
        println!(
            "  --span-timings      Log per-stage durations (capture, debayer, stack, render, encode)"
        );
        #[cfg(feature = "telemetry")]
        {
            println!(
                "  --telemetry         Enable OpenTelemetry tracing (default in debug builds)"
            );
            println!("  --no-telemetry      Disable OpenTelemetry tracing");
            println!("  --otlp-endpoint URL OTLP endpoint URL (default: http://localhost:4317)");
        }
        #[cfg(not(feature = "telemetry"))]
        {
            println!();
            println!("Note: OpenTelemetry support not compiled in. Build with --features telemetry to enable.");
        }
        println!();
        println!("  --indi-host HOST    INDI server host (overrides settings)");
        println!("  --indi-port PORT    INDI server port (overrides settings)");
    }
}

/// Best-effort startup check: warn immediately if the configured OTLP
/// endpoint isn't reachable, rather than letting the user discover it later
/// from an empty Jaeger UI and a stream of repeated background export
/// failures (`BatchSpanProcessor.ExportError`, once per batch, forever).
/// Telemetry stays enabled either way — the collector may come up shortly
/// after this check, and the OTLP client already retries on its own; this
/// only makes the "nothing is arriving" case obvious right away instead of
/// having to infer it from silence.
#[cfg(feature = "telemetry")]
async fn check_otlp_reachable(endpoint: &str) {
    let Some((host, port)) = parse_host_port(endpoint) else {
        return;
    };
    let connect = tokio::net::TcpStream::connect((host.as_str(), port));
    match tokio::time::timeout(std::time::Duration::from_millis(500), connect).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => warn!(
            endpoint,
            error = %e,
            "OTLP collector unreachable — traces/metrics will not export until it is. \
             Set --otlp-endpoint or OTEL_EXPORTER_OTLP_ENDPOINT if it runs on a different host \
             than this app (the default only works when both are on the same machine)."
        ),
        Err(_) => warn!(
            endpoint,
            "OTLP collector did not respond within 500ms — traces/metrics may not export. \
             Set --otlp-endpoint or OTEL_EXPORTER_OTLP_ENDPOINT if it runs on a different host \
             than this app (the default only works when both are on the same machine)."
        ),
    }
}

/// Extract `(host, port)` from an OTLP endpoint URL like `http://host:4317`.
/// Deliberately simple string parsing rather than a full URL parser — good
/// enough for the plain `scheme://host:port` shape this endpoint always has
/// in practice. Does not handle bracketed IPv6 literals.
#[cfg(feature = "telemetry")]
fn parse_host_port(endpoint: &str) -> Option<(String, u16)> {
    let without_scheme = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    let host_port = without_scheme.split('/').next()?;
    let (host, port_str) = host_port.rsplit_once(':')?;
    let port = port_str.parse().ok()?;
    Some((host.to_string(), port))
}

/// Run the Night Amplifier server.
///
/// Call `register_plugins` before logging is initialized to register Pro plugin
/// implementations into the global OnceLock registries. Pass a no-op closure
/// (or nothing) for the Community edition.
///
/// # Example (Pro)
/// ```ignore
/// night_amplifier::app::run(|| {
///     BACKGROUND_PLUGIN.set(Box::new(RbfPlugin)).ok();
///     // ...
/// }).await;
/// ```
pub async fn run(register_plugins: impl FnOnce()) {
    let args = Args::parse();

    // Register plugins before anything else
    register_plugins();

    // Build logging configuration
    #[cfg(feature = "telemetry")]
    let (log_config, telemetry_endpoint) = {
        // `from_env()` picks up `OTEL_EXPORTER_OTLP_ENDPOINT` /
        // `OTEL_SERVICE_NAME` / `OTEL_ENABLED`; `--otlp-endpoint` overrides on
        // top when given — CLI > env > default.
        let telemetry_config = if args.telemetry {
            let mut config = TelemetryConfig::from_env().with_enabled(true);
            if let Some(endpoint) = args.otlp_endpoint {
                config = config.with_endpoint(endpoint);
            }
            Some(config)
        } else {
            None
        };

        // Captured here (before `with_telemetry` consumes `telemetry_config`)
        // so `check_otlp_reachable` can be called *after* `init_logging()`
        // below — calling it here, before any subscriber exists, would mean
        // its `warn!` has nothing to record it and is silently dropped.
        let endpoint = telemetry_config.as_ref().map(|c| c.endpoint.clone());

        let log_config = LogConfig::default()
            .with_telemetry(telemetry_config)
            .with_span_events(args.span_timings);
        (log_config, endpoint)
    };

    #[cfg(not(feature = "telemetry"))]
    let log_config = LogConfig::default().with_span_events(args.span_timings);

    // Initialize logging - keep the guard alive for the duration of main
    let _log_guard = init_logging(log_config).expect("Failed to initialize logging");

    info!("Night Amplifier - EAA Live Stacking Server");

    if args.span_timings {
        info!("Span timings enabled - per-stage durations are logged on span close");
    }

    #[cfg(feature = "telemetry")]
    if args.telemetry {
        info!("OpenTelemetry tracing and metrics enabled");
        info!("View traces at http://localhost:16686 (Jaeger), metrics at http://localhost:9090 (Prometheus) or http://localhost:3000 (Grafana)");
        info!("Start the full stack: docker compose -f docker-compose.telemetry.yml up -d");
        if let Some(endpoint) = telemetry_endpoint {
            check_otlp_reachable(&endpoint).await;
        }
    }

    let port = args.port;

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let config = ServerConfig::new()
        .with_bind_addr(addr)
        .with_static_dir(Some("web".to_string()));

    info!("Starting server on http://{}", addr);
    info!("API endpoints:");
    info!("  GET  /api/cameras          - List available cameras");
    info!("  POST /api/cameras/:id/connect    - Connect to camera");
    info!("  POST /api/cameras/:id/disconnect - Disconnect camera");
    info!("  POST /api/capture/start    - Start capture session");
    info!("  POST /api/capture/stop     - Stop capture session");
    info!("  GET  /api/capture/status   - Get capture status");
    info!("  GET  /api/settings         - Get current settings");
    info!("  POST /api/settings         - Update settings");
    info!("WebSocket endpoints:");
    info!("  WS   /ws/stream            - Live image stream");
    info!("  WS   /ws/events            - Server events");

    let server = Server::new(config);

    // Apply CLI overrides for INDI if present
    if args.indi_host.is_some() || args.indi_port.is_some() {
        let state = server.state();
        let mut settings = state.settings.write().await;
        if let Some(host) = args.indi_host {
            settings.indi_server_host = host;
        }
        if let Some(port) = args.indi_port {
            settings.indi_server_port = port;
        }
    }

    if let Err(e) = server.run().await {
        error!("Server error: {}", e);
        std::process::exit(1);
    }
}
