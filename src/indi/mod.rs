//! INDI Client Library
//!
//! A device-agnostic, async client library for the Instrument Neutral
//! Distributed Interface (INDI) protocol.
//!
//! This module provides the core building blocks for connecting to an INDI
//! server, sending commands, and receiving property updates. It is designed
//! to be reusable across different device classes (cameras, mounts, focusers).

pub mod client;
pub mod connection;
pub mod device;
pub mod error;
pub mod fits_decoder;
pub mod xml;

pub use client::IndiClient;
pub use connection::IndiConnection;
pub use device::{IndiDevice, IndiProperty};
pub use error::{IndiError, Result};
pub use xml::PropertyState;
