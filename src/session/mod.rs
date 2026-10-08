//! The application layer: capture sessions, camera lifecycles, settings and Push-To use
//! cases, and the state they share (`state::AppState`). Everything the server drives,
//! minus HTTP: `server` is the REST/WebSocket adapter over this, and nothing here depends
//! on it (`tests/layering_test.rs`).
//!
//! The events and the few wire types they carry live here too: they are what a session
//! reports, and `server` only serializes them.

pub mod camera;
pub mod capture;
pub mod encoding;
pub mod error;
pub mod events;
pub mod services;
pub mod settings_persistence;
pub mod state;
