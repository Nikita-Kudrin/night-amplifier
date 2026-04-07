//! Comprehensive tests for the server API endpoints
//!
//! Tests cover:
//! - Success paths for all endpoints
//! - Error handling and edge cases
//! - Concurrent access scenarios
//! - WebSocket functionality

mod helpers;

mod api_response;
mod cameras;
mod capabilities;
mod capture_start_stop;
mod capture_status;
mod events;
mod server_config;
mod settings;
mod state_management;
