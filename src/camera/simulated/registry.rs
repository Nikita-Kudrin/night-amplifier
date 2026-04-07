//! Global registry for simulated camera directories

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing::{info, warn};

use crate::camera::error::{CameraError, CameraResult};
use crate::camera::traits::CameraProvider;
use crate::camera::types::CameraInfo;

use super::camera::{create_camera_info, SimulatedCamera};
use super::probe::probe_image_dimensions;

/// Supported image file extensions for TIFF
pub const TIFF_EXTENSIONS: &[&str] = &["tif", "tiff"];
/// Supported image file extensions for FITS
pub const FITS_EXTENSIONS: &[&str] = &["fit", "fits"];
/// Supported image file extensions for PNG
pub const PNG_EXTENSIONS: &[&str] = &["png"];

/// Global state for simulated camera directories
/// Supports multiple directories, each representing a separate simulated camera
static SIMULATED_DIRECTORIES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Add a directory as a new simulated camera
///
/// Returns `Ok(true)` if the camera was added, `Ok(false)` if a camera with the
/// same directory already exists (no duplicate added).
pub fn add_simulated_directory(path: PathBuf) -> CameraResult<bool> {
    if !path.exists() {
        return Err(CameraError::OpenFailed(format!(
            "Directory does not exist: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(CameraError::OpenFailed(format!(
            "Path is not a directory: {}",
            path.display()
        )));
    }

    // Canonicalize path for consistent comparison
    let canonical_path = path.canonicalize().map_err(|e| {
        CameraError::OpenFailed(format!("Failed to resolve path {}: {}", path.display(), e))
    })?;

    // Check if directory contains any image files
    let files = find_image_files(&canonical_path);
    if files.is_empty() {
        return Err(CameraError::OpenFailed(format!(
            "No image files found in directory: {}",
            path.display()
        )));
    }

    // Probe first file to ensure directory is usable
    probe_image_dimensions(&files[0]).map_err(|e| {
        CameraError::OpenFailed(format!(
            "Failed to probe image data in {}: {}",
            files[0].display(),
            e
        ))
    })?;

    let mut dirs = SIMULATED_DIRECTORIES.lock().unwrap();

    // Check if this directory is already added (compare canonical paths)
    for existing in dirs.iter() {
        if let Ok(existing_canonical) = existing.canonicalize() {
            if existing_canonical == canonical_path {
                info!(
                    path = %path.display(),
                    "Simulated camera directory already exists, skipping"
                );
                return Ok(false);
            }
        }
    }

    info!(
        path = %path.display(),
        file_count = files.len(),
        index = dirs.len(),
        "Simulated camera directory added"
    );

    dirs.push(canonical_path);
    Ok(true)
}

/// Set the directory path for the simulated camera (legacy API - replaces all directories)
pub fn set_simulated_directory(path: PathBuf) -> CameraResult<()> {
    // For backward compatibility: clear existing and add new
    clear_simulated_directories();
    add_simulated_directory(path)?;
    Ok(())
}

/// Get the first configured simulated camera directory (legacy API)
pub fn get_simulated_directory() -> Option<PathBuf> {
    let dirs = SIMULATED_DIRECTORIES.lock().unwrap();
    dirs.first().cloned()
}

/// Get all configured simulated camera directories
pub fn get_simulated_directories() -> Vec<PathBuf> {
    SIMULATED_DIRECTORIES.lock().unwrap().clone()
}

/// Clear all simulated camera directory configurations
pub fn clear_simulated_directories() {
    let mut dirs = SIMULATED_DIRECTORIES.lock().unwrap();
    dirs.clear();
}

/// Clear the simulated camera directory configuration (legacy API)
pub fn clear_simulated_directory() {
    clear_simulated_directories();
}

/// Remove a specific simulated camera directory by index
pub fn remove_simulated_directory(index: usize) -> CameraResult<PathBuf> {
    let mut dirs = SIMULATED_DIRECTORIES.lock().unwrap();
    if index >= dirs.len() {
        return Err(CameraError::InvalidCameraIndex {
            index,
            count: dirs.len(),
        });
    }
    Ok(dirs.remove(index))
}

/// Find all supported image files in a directory
pub fn find_image_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| is_supported_image_file(path))
        .collect();

    // Sort for deterministic ordering (alphabetical)
    files.sort();
    files
}

/// Check if a path has a supported image extension
pub fn is_supported_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext_lower = ext.to_lowercase();
            TIFF_EXTENSIONS.contains(&ext_lower.as_str())
                || FITS_EXTENSIONS.contains(&ext_lower.as_str())
                || PNG_EXTENSIONS.contains(&ext_lower.as_str())
        })
        .unwrap_or(false)
}

/// Provider for simulated cameras
pub struct SimulatedProvider;

impl SimulatedProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SimulatedProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for SimulatedProvider {
    fn name(&self) -> &'static str {
        "Simulator"
    }

    fn is_available(&self) -> bool {
        // Always available - no SDK needed
        true
    }

    fn camera_count(&self) -> CameraResult<usize> {
        let dirs = SIMULATED_DIRECTORIES.lock().unwrap();
        Ok(dirs.len())
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        let dirs = SIMULATED_DIRECTORIES.lock().unwrap();
        let mut cameras = Vec::new();

        for (index, dir) in dirs.iter().enumerate() {
            let files = find_image_files(dir);
            if files.is_empty() {
                continue;
            }

            // Probe first file for dimensions
            match probe_image_dimensions(&files[0]) {
                Ok((width, height, sensor_type, bayer_pattern)) => {
                    let dir_name = dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");

                    let mut info = create_camera_info(
                        dir_name,
                        files.len(),
                        width,
                        height,
                        sensor_type,
                        bayer_pattern,
                    );
                    info.id = index as i32;
                    cameras.push(info);
                }
                Err(e) => {
                    warn!(
                        directory = %dir.display(),
                        error = %e,
                        "Failed to probe simulated camera directory"
                    );
                }
            }
        }

        Ok(cameras)
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn crate::camera::traits::Camera>> {
        let dirs = SIMULATED_DIRECTORIES.lock().unwrap();
        let count = dirs.len();

        if index >= count {
            return Err(CameraError::InvalidCameraIndex { index, count });
        }

        let dir = dirs[index].clone();
        drop(dirs); // Release lock before creating camera

        let camera = SimulatedCamera::new(dir)?;
        Ok(Box::new(camera))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::sync::Mutex as StdMutex;
    use tempfile::tempdir;

    // Tests that modify global state need to be careful
    // We use a mutex to serialize tests that touch SIMULATED_DIRECTORIES
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    // Minimal valid 1x1 PNG data
    const MINIMAL_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0x44, 0x74, 0x8e, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn test_simulated_provider_no_directory() {
        let _lock = TEST_LOCK.lock().unwrap();
        clear_simulated_directories();

        let provider = SimulatedProvider::new();

        assert!(provider.is_available());
        assert_eq!(provider.camera_count().unwrap(), 0);
        assert!(provider.list_cameras().unwrap().is_empty());
    }

    #[test]
    fn test_find_image_files() {
        let _lock = TEST_LOCK.lock().unwrap();
        // This test doesn't touch global state but we lock anyway for safety
        let dir = tempdir().unwrap();

        // Create test files
        File::create(dir.path().join("test1.tif")).unwrap();
        File::create(dir.path().join("test2.fits")).unwrap();
        File::create(dir.path().join("test3.png")).unwrap();
        File::create(dir.path().join("readme.txt")).unwrap();

        let files = find_image_files(dir.path());
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_set_simulated_directory_nonexistent() {
        let _lock = TEST_LOCK.lock().unwrap();
        // This test should fail before modifying global state
        let result = add_simulated_directory(PathBuf::from("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_set_simulated_directory_empty() {
        let _lock = TEST_LOCK.lock().unwrap();
        // This test should fail before modifying global state (no image files)
        let dir = tempdir().unwrap();
        let result = add_simulated_directory(dir.path().to_path_buf());
        assert!(result.is_err());
    }

    #[test]
    fn test_add_multiple_simulated_directories() {
        let _lock = TEST_LOCK.lock().unwrap();
        clear_simulated_directories();

        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        // Create test files in both directories (valid PNGs)
        let mut f1 = File::create(dir1.path().join("test1.png")).unwrap();
        f1.write_all(MINIMAL_PNG).unwrap();
        let mut f2 = File::create(dir2.path().join("test2.png")).unwrap();
        f2.write_all(MINIMAL_PNG).unwrap();

        // Add first directory
        let result1 = add_simulated_directory(dir1.path().to_path_buf());
        assert!(result1.is_ok(), "Failed to add dir1: {:?}", result1.err());
        assert!(result1.unwrap()); // was_added = true

        // Add second directory
        let result2 = add_simulated_directory(dir2.path().to_path_buf());
        assert!(result2.is_ok(), "Failed to add dir2: {:?}", result2.err());
        assert!(result2.unwrap()); // was_added = true

        // Verify both are present
        let dirs = get_simulated_directories();
        assert_eq!(dirs.len(), 2);

        let provider = SimulatedProvider::new();
        assert_eq!(provider.camera_count().unwrap(), 2);

        clear_simulated_directories();
    }

    #[test]
    fn test_add_duplicate_directory_rejected() {
        let _lock = TEST_LOCK.lock().unwrap();
        clear_simulated_directories();

        let dir = tempdir().unwrap();
        let mut f = File::create(dir.path().join("test.png")).unwrap();
        f.write_all(MINIMAL_PNG).unwrap();

        // Add directory first time
        let result1 = add_simulated_directory(dir.path().to_path_buf());
        assert!(result1.is_ok());
        assert!(result1.unwrap()); // was_added = true

        // Try to add the same directory again
        let result2 = add_simulated_directory(dir.path().to_path_buf());
        assert!(result2.is_ok());
        assert!(!result2.unwrap()); // was_added = false (duplicate)

        // Verify only one directory is present
        let dirs = get_simulated_directories();
        assert_eq!(dirs.len(), 1);

        clear_simulated_directories();
    }

    #[test]
    fn test_remove_simulated_directory() {
        let _lock = TEST_LOCK.lock().unwrap();
        clear_simulated_directories();

        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        let mut f1 = File::create(dir1.path().join("test1.png")).unwrap();
        f1.write_all(MINIMAL_PNG).unwrap();
        let mut f2 = File::create(dir2.path().join("test2.png")).unwrap();
        f2.write_all(MINIMAL_PNG).unwrap();

        add_simulated_directory(dir1.path().to_path_buf()).unwrap();
        add_simulated_directory(dir2.path().to_path_buf()).unwrap();

        assert_eq!(get_simulated_directories().len(), 2);

        // Remove first directory
        let removed = remove_simulated_directory(0);
        assert!(removed.is_ok());

        assert_eq!(get_simulated_directories().len(), 1);

        clear_simulated_directories();
    }

    #[test]
    fn test_add_simulated_directory_invalid_probing() {
        let _lock = TEST_LOCK.lock().unwrap();
        clear_simulated_directories();

        let dir = tempdir().unwrap();
        // Create a file with .fits extension but invalid content
        let file_path = dir.path().join("broken.fits");
        File::create(file_path).unwrap(); // Empty file

        let result = add_simulated_directory(dir.path().to_path_buf());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to probe image data"));
        assert!(err_msg.contains("broken.fits"));

        clear_simulated_directories();
    }
}
