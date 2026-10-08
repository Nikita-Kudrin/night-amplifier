//! Service layer for the server module
//!
//! Services encapsulate business logic and provide a clean interface
//! between API handlers and the underlying data/operations.

mod camera_service;
mod capture_service;
mod push_to_service;
mod settings_request;
mod settings_service;

pub use camera_service::CameraService;
pub use capture_service::CaptureService;
pub use push_to_service::{PushToService, PushToState, SolveLatch, WatchLatch};
pub use settings_request::UpdateSettingsRequest;
pub use settings_service::{optics_change, OpticsChange, SettingsService};
