//! Tests for server configuration

use std::net::SocketAddr;

use crate::server::{Server, ServerConfig, ServerError};

#[test]
fn test_server_config_with_bind_addr() {
    let addr: SocketAddr = "192.168.1.100:3000".parse().unwrap();
    let config = ServerConfig::new().with_bind_addr(addr);

    assert_eq!(config.bind_addr, addr);
}

#[test]
fn test_server_config_static_dir_none() {
    let config = ServerConfig::new().with_static_dir(None);
    assert!(config.static_dir.is_none());
}

#[test]
fn test_server_creation() {
    let config = ServerConfig::new().with_port(9999);
    let server = Server::new(config);

    assert_eq!(server.config.bind_addr.port(), 9999);
}

#[test]
fn test_server_with_defaults() {
    let server = Server::with_defaults();

    assert_eq!(server.config.bind_addr.port(), 9955);
    assert!(server.config.enable_cors);
}

#[test]
fn test_server_state_access() {
    let server = Server::with_defaults();
    let state = server.state();

    // Should be able to access state
    assert!(!state.is_cancelled());
}

#[test]
fn test_server_error_display() {
    let bind_err = ServerError::BindFailed("address in use".to_string());
    assert!(bind_err.to_string().contains("Failed to bind"));
    assert!(bind_err.to_string().contains("address in use"));

    let serve_err = ServerError::ServeFailed("connection refused".to_string());
    assert!(serve_err.to_string().contains("Server error"));
    assert!(serve_err.to_string().contains("connection refused"));
}

#[test]
fn test_server_error_debug() {
    let err = ServerError::BindFailed("test".to_string());
    let debug_str = format!("{:?}", err);
    assert!(debug_str.contains("BindFailed"));
}

/// Routing behaviour of the frontend fallback, which decides between the embedded
/// bundle and a `--static-dir` on disk.
mod frontend_serving {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::server::{Server, ServerConfig};

    async fn get(config: ServerConfig, uri: &str) -> (StatusCode, String, String) {
        let response = Server::new(config)
            .build_router()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, content_type, String::from_utf8_lossy(&body).into_owned())
    }

    /// The default has to be the embedded bundle. It used to be the string `web`, so a
    /// binary started anywhere near a checkout served the Vite source template and the
    /// frontend never booted.
    #[tokio::test]
    async fn the_default_serves_the_embedded_bundle() {
        let config = ServerConfig::new();
        assert!(config.static_dir.is_none());

        let (status, content_type, body) = get(config, "/eyepiece_quality").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"));
        assert!(
            body.contains("/assets/"),
            "the embedded bundle's index.html should name its hashed assets"
        );
    }

    #[tokio::test]
    async fn a_static_dir_is_served_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<p>from disk</p>").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/app.js"), "export const x = 1").unwrap();

        let config =
            ServerConfig::new().with_static_dir(Some(dir.path().to_string_lossy().into_owned()));

        let (status, _, body) = get(config.clone(), "/assets/app.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "export const x = 1");

        // A client-side route still reaches the SPA shell.
        let (status, content_type, body) = get(config, "/eyepiece_quality").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"));
        assert_eq!(body, "<p>from disk</p>");
    }

    /// The same silent-blank-page trap as the embedded path: a bundle hash that has
    /// moved on must 404, not come back as HTML that Chromium refuses to execute.
    #[tokio::test]
    async fn a_missing_asset_on_disk_is_a_404() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<p>from disk</p>").unwrap();

        let config =
            ServerConfig::new().with_static_dir(Some(dir.path().to_string_lossy().into_owned()));

        let (status, content_type, _) = get(config, "/assets/index-DEADBEEF.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!content_type.starts_with("text/html"));
    }

    /// A directory that holds no `index.html` cannot serve the app, so the embedded
    /// bundle takes over rather than the server answering nothing at all.
    #[tokio::test]
    async fn a_static_dir_without_index_html_falls_back_to_embedded() {
        let dir = tempfile::tempdir().unwrap();

        let config =
            ServerConfig::new().with_static_dir(Some(dir.path().to_string_lossy().into_owned()));

        let (status, content_type, body) = get(config, "/eyepiece_quality").await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"));
        assert!(body.contains("/assets/"));
    }

    /// API and WebSocket routes must keep their own handlers whichever branch is taken.
    #[tokio::test]
    async fn the_fallback_does_not_shadow_the_api() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<p>from disk</p>").unwrap();

        for config in [
            ServerConfig::new(),
            ServerConfig::new()
                .with_static_dir(Some(dir.path().to_string_lossy().into_owned())),
        ] {
            let (status, content_type, _) = get(config, "/api/capture/status").await;
            assert_eq!(status, StatusCode::OK);
            assert!(content_type.starts_with("application/json"));
        }
    }
}
