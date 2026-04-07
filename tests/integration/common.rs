//! Common utilities, constants, and types shared across integration tests.

use std::fs;
use std::path::{Path, PathBuf};

/// Directory containing test fixture files
pub const FIXTURES_DIR: &str = "tests/fixtures";

/// Directory for processed output files
pub const PROCESSED_DIR: &str = "tests/fixtures/processed";

/// Minimum number of frames required for stacking tests
pub const MIN_FRAMES_FOR_STACKING: usize = 2;

// ============================================================================
// Validation Constants - Critical Thresholds for Pipeline Quality
// ============================================================================

/// Minimum number of stars that must be detected for reliable registration.
/// Triangle matching requires at least 3 stars, but we need more for robustness.
pub const MIN_STARS_FOR_REGISTRATION: usize = 10;

/// Minimum percentage of frames that must successfully stack (0.0 - 1.0).
/// If fewer frames stack, the registration algorithm may have issues.
pub const MIN_STACKING_SUCCESS_RATE: f64 = 0.5;

/// Minimum mean pixel value for output (ensures image is not all black)
pub const MIN_OUTPUT_MEAN_VALUE: f64 = 1.0;

/// Maximum mean pixel value for output (ensures image is not all white/saturated)
pub const MAX_OUTPUT_MEAN_VALUE: f64 = 254.0;

/// Minimum acceptable SNR for detected stars
pub const MIN_ACCEPTABLE_SNR: f32 = 5.0;

/// Minimum stretch factor that indicates successful auto-stretch
pub const MIN_STRETCH_FACTOR: f32 = 1.0;

/// Maximum stretch factor (beyond this suggests problematic data)
pub const MAX_STRETCH_FACTOR: f32 = 10000.0;

/// Supported image file extensions
pub const TIFF_EXTENSIONS: &[&str] = &["tif", "tiff"];
pub const FITS_EXTENSIONS: &[&str] = &["fit", "fits"];

/// Test output subdirectory names
pub const STACKED_OUTPUT_DIR: &str = "stacked";
pub const DEBAYER_OUTPUT_DIR: &str = "debayer";

// ============================================================================
// Common Types
// ============================================================================

/// Represents a loaded astronomical image with metadata
#[derive(Debug)]
pub struct LoadedImage {
    pub frame: night_amplifier::Frame,
    pub path: PathBuf,
    pub width: usize,
    pub height: usize,
    /// True if this was loaded as raw Bayer data (single channel, needs debayering)
    pub is_bayer: bool,
}

/// Represents a fixture subdirectory containing image files
#[derive(Debug)]
#[allow(dead_code)]
pub struct FixtureSet {
    pub name: String,
    pub path: PathBuf,
    pub files: Vec<PathBuf>,
}

// ============================================================================
// Fixture Discovery Functions
// ============================================================================

/// Finds all subdirectories in the fixtures directory that contain image files
pub fn find_fixture_sets() -> Vec<FixtureSet> {
    let fixtures_path = Path::new(FIXTURES_DIR);

    if !fixtures_path.exists() {
        return Vec::new();
    }

    let mut sets: Vec<FixtureSet> = fs::read_dir(fixtures_path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            // Skip the processed directory
            entry.file_name().to_str() != Some("processed")
        })
        .filter_map(|entry| {
            let dir_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let files = find_image_files_in_dir(&dir_path);
            if files.is_empty() {
                None
            } else {
                Some(FixtureSet {
                    name,
                    path: dir_path,
                    files,
                })
            }
        })
        .collect();

    // Sort for deterministic ordering
    sets.sort_by(|a, b| a.name.cmp(&b.name));
    sets
}

/// Finds all image files in a specific directory
pub fn find_image_files_in_dir(dir_path: &Path) -> Vec<PathBuf> {
    if !dir_path.exists() {
        return Vec::new();
    }

    let mut files: Vec<PathBuf> = fs::read_dir(dir_path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    let ext_lower = ext.to_lowercase();
                    TIFF_EXTENSIONS.contains(&ext_lower.as_str())
                        || FITS_EXTENSIONS.contains(&ext_lower.as_str())
                })
                .unwrap_or(false)
        })
        .collect();

    // Sort for deterministic ordering
    files.sort();
    files
}

// ============================================================================
// Test Output Directory Management
// ============================================================================

/// Gets the path to a test-specific output directory under PROCESSED_DIR.
/// The directory is NOT created by this function.
pub fn get_test_output_dir(test_name: &str) -> PathBuf {
    Path::new(PROCESSED_DIR).join(test_name)
}

/// Prepares a test-specific output directory by clearing it if it exists
/// and then creating it fresh.
pub fn prepare_test_output_dir(test_name: &str) -> Result<PathBuf, String> {
    let output_dir = get_test_output_dir(test_name);

    // Clear the directory if it exists
    if output_dir.exists() {
        fs::remove_dir_all(&output_dir)
            .map_err(|e| format!("Failed to clear output directory {:?}: {}", output_dir, e))?;
    }

    // Create the directory
    fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Failed to create output directory {:?}: {}", output_dir, e))?;

    Ok(output_dir)
}
