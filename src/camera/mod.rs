//! Camera support for astronomical imaging: a unified interface over Player One, ZWO,
//! SVBony, Touptek, QHYCCD, and others, via a trait-based abstraction — [`Camera`]
//! (capture operations), [`CameraProvider`] (discovery/opening), [`CameraRegistry`]
//! (manages multiple providers).
//!
//! ```no_run
//! use night_amplifier::camera::{CameraRegistry, CaptureConfig};
//!
//! let mut registry = CameraRegistry::new();
//! registry.register_defaults();
//! let mut camera = registry.open_camera("PlayerOne", 0)?;
//! let config = CaptureConfig::default().with_exposure_us(1_000_000);
//! let frame = camera.capture(&config)?;
//! # Ok::<(), night_amplifier::camera::CameraError>(())
//! ```

mod device_lease;
mod device_lost;
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

#[cfg(feature = "indi")]
mod indi;
#[cfg(not(feature = "indi"))]
mod indi_stub;

#[cfg(feature = "qhy")]
mod qhy;
#[cfg(not(feature = "qhy"))]
mod qhy_stub;

#[cfg(feature = "touptek")]
mod touptek;
#[cfg(not(feature = "touptek"))]
mod touptek_stub;

// Re-export everything
pub use device_lease::DeviceLease;
pub use device_lost::{is_marked as is_device_lost_message, mark as mark_device_lost};
pub use error::{CameraError, CameraResult};
pub use registry::{CameraEntry, CameraRegistry};
pub use traits::{Camera, CameraProvider};
pub use types::{
    BufferPool, CameraInfo, CameraStatus, CaptureConfig, DualSamplingMode, GainPresets,
    ImageFormat, PooledBuffer, RawFrame, SensorMode, SensorType,
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
pub use zwo::{ZwoCamera, ZwoProvider};
#[cfg(not(feature = "zwo"))]
pub use zwo_stub::{ZwoCamera, ZwoProvider};

// INDI provider re-exports
#[cfg(feature = "indi")]
pub use indi::{IndiCamera, IndiProvider};
#[cfg(not(feature = "indi"))]
pub use indi_stub::{IndiCamera, IndiProvider};

// QHY provider re-exports
#[cfg(feature = "qhy")]
pub use qhy::{QhyCamera, QhyProvider};
#[cfg(not(feature = "qhy"))]
pub use qhy_stub::{QhyCamera, QhyProvider};

// ToupTek provider re-exports
#[cfg(feature = "touptek")]
pub use touptek::{TouptekCamera, TouptekProvider};
#[cfg(not(feature = "touptek"))]
pub use touptek_stub::{TouptekCamera, TouptekProvider};

// SVBony provider re-exports
#[cfg(feature = "svbony")]
mod svbony;
#[cfg(not(feature = "svbony"))]
mod svbony_stub;

#[cfg(feature = "svbony")]
pub use svbony::{SvbonyCamera, SvbonyProvider};
#[cfg(not(feature = "svbony"))]
pub use svbony_stub::{SvbonyCamera, SvbonyProvider};

// Simulated camera
pub use simulated::{
    add_simulated_directory, clear_simulated_directories, clear_simulated_directory,
    get_simulated_directories, get_simulated_directory, remove_simulated_directory,
    set_simulated_directory, SimulatedCamera, SimulatedProvider,
};
