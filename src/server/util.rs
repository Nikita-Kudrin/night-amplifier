//! Utility functions for the server module
//!
//! Contains shared helper functions to reduce code duplication.

use std::path::Path;

/// Supported image file extensions for the simulated camera
const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &["tif", "tiff", "fit", "fits", "png"];

/// Check if a file has a supported image extension
pub fn is_supported_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext_lower = ext.to_lowercase();
            SUPPORTED_IMAGE_EXTENSIONS.contains(&ext_lower.as_str())
        })
        .unwrap_or(false)
}

/// Count supported image files in a directory
pub fn count_image_files(dir: &Path) -> Option<usize> {
    std::fs::read_dir(dir).ok().map(|entries| {
        entries
            .filter_map(|e| e.ok())
            .filter(|e| is_supported_image_file(&e.path()))
            .count()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_supported_image_file() {
        assert!(is_supported_image_file(&PathBuf::from("image.tif")));
        assert!(is_supported_image_file(&PathBuf::from("image.TIFF")));
        assert!(is_supported_image_file(&PathBuf::from("image.fit")));
        assert!(is_supported_image_file(&PathBuf::from("image.fits")));
        assert!(is_supported_image_file(&PathBuf::from("image.png")));
        assert!(is_supported_image_file(&PathBuf::from("image.PNG")));

        assert!(!is_supported_image_file(&PathBuf::from("image.jpg")));
        assert!(!is_supported_image_file(&PathBuf::from("image.txt")));
        assert!(!is_supported_image_file(&PathBuf::from("noextension")));
    }
}
