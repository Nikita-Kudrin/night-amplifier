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
    /// Serve the frontend from this directory instead of the bundle embedded at build
    /// time. Opt-in: see `Args::parse` for why it is not inferred.
    static_dir: Option<String>,
}

impl Args {
    fn parse() -> Self {
        let default_port = if std::path::Path::new("Cargo.toml").exists()
            || std::path::Path::new("web/index.html").exists()
        {
            9955u16
        } else {
            8844u16
        };
        Self::parse_from(
            std::env::args().skip(1),
            default_port,
            std::env::var("NIGHT_AMPLIFIER_STATIC_DIR").ok(),
        )
    }

    /// The argument logic, with the process environment passed in rather than read, so
    /// it can be tested without mutating globals shared by every other test.
    fn parse_from(
        args: impl Iterator<Item = String>,
        default_port: u16,
        env_static_dir: Option<String>,
    ) -> Self {
        let mut args = args;
        let mut port = default_port;
        #[cfg(feature = "telemetry")]
        let mut telemetry = TelemetryConfig::default_enabled();
        #[cfg(feature = "telemetry")]
        let mut otlp_endpoint = None;
        let mut indi_host = None;
        let mut indi_port = None;
        let mut span_timings = false;
        let mut static_dir = None;

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
                "--static-dir" => {
                    static_dir = args.next();
                    if static_dir.is_none() {
                        eprintln!("Error: --static-dir requires a value");
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

        // Opt-in, and never inferred from the working directory.
        //
        // This used to be hardcoded to `web`, which meant that running the binary
        // anywhere near a checkout served the Vite *source* template — the one that
        // loads `/src/main.js` and untransformed `.vue` files — in place of the built
        // bundle, for a frontend that could not start. Nothing depends on it: `npm run
        // dev` serves the UI from Vite and proxies `/api` and `/ws` here, so the
        // embedded bundle is the right default for every way the server is actually run.
        let static_dir = static_dir.or(env_static_dir);

        Self {
            port,
            #[cfg(feature = "telemetry")]
            telemetry,
            #[cfg(feature = "telemetry")]
            otlp_endpoint,
            indi_host,
            indi_port,
            span_timings,
            static_dir,
        }
    }

    fn print_help() {
        println!("Night Amplifier - EAA Live Stacking Server");
        println!();
        println!("Usage: night_amplifier [OPTIONS] [PORT]");
        println!();
        println!("Arguments:");
        println!(
            "  [PORT]              Server port (default: 9955, or 8844 in distribution binary)"
        );
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
        println!(
            "  --static-dir DIR    Serve the frontend from DIR (must contain index.html)"
        );
        println!(
            "                      instead of the bundle embedded at build time."
        );
        println!(
            "                      Also settable via NIGHT_AMPLIFIER_STATIC_DIR."
        );
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

/// Run the Night Amplifier server. Call `register_plugins` before logging is
/// initialized, to register Pro plugin implementations into the global OnceLock
/// registries — pass a no-op closure (or nothing) for the Community edition.
///
/// ```ignore
/// night_amplifier::app::run(|| { BACKGROUND_PLUGIN.set(Box::new(RbfPlugin)).ok(); }).await;
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
        .with_static_dir(args.static_dir.clone());

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

#[cfg(test)]
mod tests {
    use super::Args;

    fn parse(argv: &[&str], env_static_dir: Option<&str>) -> Args {
        Args::parse_from(
            argv.iter().map(|s| s.to_string()),
            9955,
            env_static_dir.map(str::to_string),
        )
    }

    /// The regression this flag exists for. `static_dir` was hardcoded to `web`, so a
    /// binary run from anywhere near a checkout served the Vite *source* template —
    /// which loads `/src/main.js` and untransformed `.vue` files — instead of the
    /// embedded bundle, and the frontend never started. Nothing may infer it.
    #[test]
    fn the_frontend_is_embedded_unless_asked_otherwise() {
        assert!(parse(&[], None).static_dir.is_none());
        assert!(parse(&["8080"], None).static_dir.is_none());
    }

    #[test]
    fn static_dir_comes_from_the_flag() {
        assert_eq!(
            parse(&["--static-dir", "web/dist"], None).static_dir.as_deref(),
            Some("web/dist")
        );
    }

    #[test]
    fn static_dir_falls_back_to_the_environment() {
        assert_eq!(
            parse(&[], Some("/srv/ui")).static_dir.as_deref(),
            Some("/srv/ui")
        );
    }

    /// An explicit flag outranks an environment variable someone's shell profile set.
    #[test]
    fn the_flag_outranks_the_environment() {
        assert_eq!(
            parse(&["--static-dir", "web/dist"], Some("/srv/ui"))
                .static_dir
                .as_deref(),
            Some("web/dist")
        );
    }

    /// The flag must not swallow a following argument's meaning.
    #[test]
    fn static_dir_does_not_disturb_other_arguments() {
        let args = parse(&["--static-dir", "web/dist", "8080"], None);
        assert_eq!(args.static_dir.as_deref(), Some("web/dist"));
        assert_eq!(args.port, 8080);
    }
}
