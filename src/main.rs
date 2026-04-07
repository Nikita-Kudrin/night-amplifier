//! Night Amplifier CLI
//!
//! Runs a web server for remote camera control and image streaming.

use night_amplifier::logging::{init_logging, LogConfig};
use night_amplifier::server::{Server, ServerConfig};
#[cfg(feature = "telemetry")]
use night_amplifier::telemetry::TelemetryConfig;
use std::net::SocketAddr;
use tracing::{error, info};

/// Command line arguments
struct Args {
    port: u16,
    #[cfg(feature = "telemetry")]
    telemetry: bool,
    #[cfg(feature = "telemetry")]
    otlp_endpoint: Option<String>,
}

impl Args {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1);
        let mut port = 8080u16;
        #[cfg(feature = "telemetry")]
        let mut telemetry = TelemetryConfig::default_enabled();
        #[cfg(feature = "telemetry")]
        let mut otlp_endpoint = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    Self::print_help();
                    std::process::exit(0);
                }
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
        }
    }

    fn print_help() {
        println!("Night Amplifier - EAA Live Stacking Server");
        println!();
        println!("Usage: night_amplifier [OPTIONS] [PORT]");
        println!();
        println!("Arguments:");
        println!("  [PORT]              Server port (default: 8080)");
        println!();
        println!("Options:");
        println!("  -h, --help          Show this help message");
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
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Build logging configuration
    #[cfg(feature = "telemetry")]
    let log_config = {
        let telemetry_config = if args.telemetry {
            let mut config = TelemetryConfig::default().with_enabled(true);
            if let Some(endpoint) = args.otlp_endpoint {
                config = config.with_endpoint(endpoint);
            }
            Some(config)
        } else {
            None
        };
        LogConfig::default().with_telemetry(telemetry_config)
    };

    #[cfg(not(feature = "telemetry"))]
    let log_config = LogConfig::default();

    // Initialize logging - keep the guard alive for the duration of main
    let _log_guard = init_logging(log_config).expect("Failed to initialize logging");

    info!("Night Amplifier - EAA Live Stacking Server");

    #[cfg(feature = "telemetry")]
    if args.telemetry {
        info!("OpenTelemetry tracing and metrics enabled");
        info!("View traces at http://localhost:16686 (Jaeger), metrics at http://localhost:9090 (Prometheus) or http://localhost:3000 (Grafana)");
        info!("Start the full stack: docker compose -f docker-compose.telemetry.yml up -d");
    }

    let port = args.port;

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let config = ServerConfig::new()
        .with_bind_addr(addr)
        .with_static_dir(Some("web".to_string()))
        .with_stream_fps(10)
        .with_jpeg_quality(85);

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

    if let Err(e) = server.run().await {
        error!("Server error: {}", e);
        std::process::exit(1);
    }
}
