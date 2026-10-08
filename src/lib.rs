//! Night Amplifier - Professional-grade EAA Live Stacking Engine
//!
//! A high-performance astronomy image stacking engine optimized for
//! Electronically Assisted Astronomy (EAA) on embedded platforms like Raspberry Pi 5.
//!
//! This crate is the HTTP/WebSocket server and the binary. It re-exports the domain
//! (`night_amplifier_core`) at the root and the application layer as [`session`], so
//! every `night_amplifier::…` path Pro and the tests use is unchanged.

// See the core crate's root for why `unused_imports` stays allowed.
#![allow(unused_imports)]

pub use night_amplifier_core::*;
pub use night_amplifier_session as session;

pub mod app;
pub mod server;
