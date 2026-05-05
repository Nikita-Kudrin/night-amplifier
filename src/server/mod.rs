//! Web server module for remote camera control and image streaming
//!
//! This module provides a REST API and WebSocket server for controlling
//! camera capture sessions and streaming live stacked images.
//!
//! # Capabilities
//!
//! - REST API for camera control (start/stop capture, settings)
//! - WebSocket streaming for live image preview
//! - WebSocket events for status updates
//!
//! # Example
//!
//! ```no_run
//! use night_amplifier::server::{Server, ServerConfig};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = ServerConfig::default();
//!     let server = Server::new(config);
//!     server.run().await.unwrap();
//! }
//! ```

mod api;
mod camera_session;
mod capture;
mod dto;
mod embedded_assets;
mod encoding;
pub mod error;
pub mod events;
pub mod services;
mod settings_persistence;
mod state;
mod util;
mod ws;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

pub use dto::*;
pub use encoding::{encode_rgb8_lz4, encode_rgb8_lz4_chunked, RGB8_CHUNKED_MAGIC, RGB8_MAGIC};
pub use error::{ApiError, ApiResult, ServerError};
pub use events::ServerEvent;
pub use services::{CameraService, CaptureService};
pub use settings_persistence::SettingsPersistence;
pub use state::*;
pub use ws::*;

use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind the server to
    pub bind_addr: SocketAddr,
    /// Path to static files directory (for frontend)
    pub static_dir: Option<String>,
    /// Enable CORS for cross-origin requests
    pub enable_cors: bool,
    /// Maximum WebSocket message size in bytes
    pub max_ws_message_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 8080)),
            static_dir: None,
            enable_cors: true,
            max_ws_message_size: 16 * 1024 * 1024, // 16MB
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the bind address
    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = addr;
        self
    }

    /// Set the port (keeps existing IP)
    pub fn with_port(mut self, port: u16) -> Self {
        self.bind_addr.set_port(port);
        self
    }

    /// Set the static files directory
    pub fn with_static_dir(mut self, dir: Option<String>) -> Self {
        self.static_dir = dir;
        self
    }

    /// Enable or disable CORS
    pub fn with_cors(mut self, enabled: bool) -> Self {
        self.enable_cors = enabled;
        self
    }
}

use crate::disk_writer::DiskWriter;

/// The main server struct
pub struct Server {
    config: ServerConfig,
    state: Arc<AppState>,
    disk_writer: Option<DiskWriter>,
}

impl Server {
    /// Create a new server with the given configuration
    pub fn new(config: ServerConfig) -> Self {
        let (state, disk_writer) = AppState::new();
        let state_arc = Arc::new(state);

        // Initialize Push-To plugin if available
        if let Some(plugin) = crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN) {
            plugin.init(state_arc.events.clone());
        }

        Self {
            config,
            state: state_arc,
            disk_writer: Some(disk_writer),
        }
    }

    /// Create a new server with default configuration
    pub fn with_defaults() -> Self {
        Self::new(ServerConfig::default())
    }

    /// Get a reference to the application state
    pub fn state(&self) -> Arc<AppState> {
        Arc::clone(&self.state)
    }

    /// Build the router with all routes
    fn build_router(&self) -> Router {
        let api_routes = api::create_router();

        let ws_routes = Router::new()
            .route("/stream", get(ws::stream_handler))
            .route("/events", get(ws::events_handler));

        let app = Router::new()
            .nest("/api", api_routes)
            .nest("/ws", ws_routes);

        // Add CORS if enabled
        let app = if self.config.enable_cors {
            let cors = CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any);
            app.layer(cors)
        } else {
            app
        };

        // Serve static files: prefer filesystem (for development), fall back
        // to embedded assets (for distribution).
        let app = if let Some(ref static_dir) = self.config.static_dir {
            let index_path = std::path::Path::new(static_dir).join("index.html");
            if index_path.exists() {
                let serve_dir = ServeDir::new(static_dir).fallback(ServeFile::new(index_path));
                app.fallback_service(serve_dir)
            } else {
                app.fallback(embedded_assets::serve_embedded)
            }
        } else {
            app.fallback(embedded_assets::serve_embedded)
        };

        app.with_state(Arc::clone(&self.state))
    }

    /// Run the server
    pub async fn run(mut self) -> Result<(), ServerError> {
        // Spawn the disk writer on a dedicated OS thread so file I/O
        // never competes with the tokio blocking-thread pool.
        if let Some(disk_writer) = self.disk_writer.take() {
            std::thread::Builder::new()
                .name("disk-writer".into())
                .spawn(move || disk_writer.run())
                .expect("failed to spawn disk writer thread");
        }

        let app = self.build_router();
        let listener = tokio::net::TcpListener::bind(self.config.bind_addr)
            .await
            .map_err(|e| ServerError::BindFailed(e.to_string()))?;

        info!("Server listening on {}", self.config.bind_addr);

        axum::serve(listener, app)
            .await
            .map_err(|e| ServerError::ServeFailed(e.to_string()))?;

        Ok(())
    }
}

// ServerError is now defined in error.rs module

// Note: Comprehensive tests are in the tests submodule (tests.rs)
