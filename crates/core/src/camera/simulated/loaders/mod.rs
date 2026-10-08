//! Image file loaders for various formats

mod fits;
mod png;
mod tiff;

use std::path::Path;

use crate::camera::error::{CameraError, CameraResult};
use crate::Frame;

use super::registry::{FITS_EXTENSIONS, PNG_EXTENSIONS, TIFF_EXTENSIONS};

pub use self::fits::load_fits;
pub use self::png::load_png;
pub use self::tiff::load_tiff;

/// Load an image file into a Frame
pub fn load_image(path: &Path) -> CameraResult<Frame> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if TIFF_EXTENSIONS.contains(&ext.as_str()) {
        load_tiff(path)
    } else if FITS_EXTENSIONS.contains(&ext.as_str()) {
        load_fits(path)
    } else if PNG_EXTENSIONS.contains(&ext.as_str()) {
        load_png(path)
    } else {
        Err(CameraError::ImageReadFailed(format!(
            "Unsupported file format: {}",
            ext
        )))
    }
}
