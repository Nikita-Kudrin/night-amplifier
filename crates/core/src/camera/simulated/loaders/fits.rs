//! FITS loading for the simulated camera.
//!
//! The reader itself lives in [`crate::fits`] next to the writers — a FITS reader is not
//! a simulated-camera concern, and keeping one copy is what lets the integration harness
//! use the same code path the application does. This is only the error-type adapter.

use std::path::Path;

use crate::camera::error::{CameraError, CameraResult};
use crate::Frame;

pub fn load_fits(path: &Path) -> CameraResult<Frame> {
    crate::fits::read_frame(path).map_err(|e| CameraError::ImageReadFailed(e.to_string()))
}
