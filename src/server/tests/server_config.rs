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

    assert_eq!(server.config.bind_addr.port(), 8080);
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
