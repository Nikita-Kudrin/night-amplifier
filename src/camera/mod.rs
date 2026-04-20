//! Camera support for astronomical imaging
//!
//! This module provides a unified interface for capturing images from various
//! astronomy camera manufacturers including Player One, ZWO, SVBony, Touptek,
//! QHYCCD, and others.
//!
//! # Architecture
//!
//! The camera system uses a trait-based abstraction:
//!
//! - [`Camera`] - Core trait for image capture operations
//! - [`CameraProvider`] - Factory trait for discovering and opening cameras
//! - [`CameraRegistry`] - Manages multiple camera providers
//!
//! # Example
//!
//! ```no_run
//! use night_amplifier::camera::{CameraRegistry, CaptureConfig};
//!
//! // Create registry and register available providers
//! let mut registry = CameraRegistry::new();
//! registry.register_defaults();
//!
//! // List all cameras from all providers
//! let cameras = registry.list_all_cameras()?;
//! for cam in &cameras {
//!     println!("{}: {} ({}x{})", cam.provider, cam.info.name,
//!              cam.info.max_width, cam.info.max_height);
//! }
//!
//! // Open a specific camera
//! let mut camera = registry.open_camera("PlayerOne", 0)?;
//!
//! // Capture an image
//! let config = CaptureConfig::default().with_exposure_us(1_000_000);
//! let frame = camera.capture(&config)?;
//! # Ok::<(), night_amplifier::camera::CameraError>(())
//! ```

mod error;
mod registry;
mod simulated;
mod traits;
mod types;

#[cfg(feature = "playerone")]
mod playerone;
#[cfg(not(feature = "playerone"))]
mod playerone_stub;

#[cfg(feature = "zwo")]
mod zwo;
#[cfg(not(feature = "zwo"))]
mod zwo_stub;

// Re-export everything
pub use error::{CameraError, CameraResult};
pub use registry::{CameraEntry, CameraRegistry};
pub use traits::{Camera, CameraProvider};
pub use types::{
    CameraInfo, CameraStatus, CaptureConfig, DualSamplingMode, GainPresets, ImageFormat,
    SensorMode, SensorType,
};

// Provider-specific re-exports
#[cfg(feature = "playerone")]
pub use playerone::PlayerOneCamera;
#[cfg(not(feature = "playerone"))]
pub use playerone_stub::PlayerOneCamera;

// Provider implementations
#[cfg(feature = "playerone")]
pub use playerone::PlayerOneProvider;
#[cfg(not(feature = "playerone"))]
pub use playerone_stub::PlayerOneProvider;

// ZWO provider re-exports
#[cfg(feature = "zwo")]
pub use zwo::ZwoCamera;
#[cfg(not(feature = "zwo"))]
pub use zwo_stub::ZwoCamera;

#[cfg(feature = "zwo")]
pub use zwo::ZwoProvider;
#[cfg(not(feature = "zwo"))]
pub use zwo_stub::ZwoProvider;

// Simulated camera
pub use simulated::{
    add_simulated_directory, clear_simulated_directories, clear_simulated_directory,
    get_simulated_directories, get_simulated_directory, remove_simulated_directory,
    set_simulated_directory, SimulatedCamera, SimulatedProvider,
};
