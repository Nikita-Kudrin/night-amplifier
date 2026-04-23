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

/// Default list of fixture datasets to download.
pub const DEFAULT_FIXTURES: &[(&str, &str)] = &[
    (
        "250mm-dob-imx464-orion-png",
        "https://drive.google.com/uc?id=1vKjx5lCFoqhJOcgRLPd4Btcf6Y4j96ap&export=download",
    ),
    (
        "35mm-imx464-orion-tiff",
        "https://drive.google.com/uc?id=1Qgs51ATx7k5ECdTRwV8ThXE2Lgb2qRqP&export=download",
    ),
    (
        "130mm-imx464-dumbell-nebulae-png",
        "https://drive.google.com/uc?id=1GYc544x6EZpYmA0S3DUo3XqDo3NiyI7W&export=download",
    ),
    (
        "130mm-imx464-ring-nebulae-png",
        "https://drive.google.com/uc?id=1qeZJ71NxXdPIuUa3U6SNn_6ZMH6CftF3&export=download",
    ),
];

/// Downloads and extracts test fixture datasets from Google Drive.
///
/// Each fixture is only downloaded once — if the target directory already exists,
/// it is skipped. After downloading, the zip is extracted and removed.
pub async fn ensure_fixtures(names: Option<&[&str]>) {
    use night_amplifier::push_to::download::download_file;
    use std::fs;
    use std::io;
    use std::path::Path;
    use tokio::sync::mpsc;

    let fixtures: Vec<(&str, &str)> = if let Some(names) = names {
        DEFAULT_FIXTURES
            .iter()
            .filter(|(name, _)| names.contains(name))
            .copied()
            .collect()
    } else {
        DEFAULT_FIXTURES.iter().copied().collect()
    };

    let fixtures_dir = Path::new(FIXTURES_DIR);
    if !fixtures_dir.exists() {
        fs::create_dir_all(fixtures_dir).expect("Failed to create fixtures dir");
    }

    let (tx, mut rx) = mpsc::channel(100);

    // Drain channel
    tokio::spawn(async move { while let Some(_) = rx.recv().await {} });

    for (name, url) in fixtures {
        let dir_path = fixtures_dir.join(name);
        if dir_path.exists() {
            continue;
        }

        let zip_path = fixtures_dir.join(format!("{}.zip", name));

        // Check again after potential race
        if dir_path.exists() {
            continue;
        }

        println!("Downloading fixture {} from {}", name, url);
        if let Err(e) = download_file(url, &zip_path, name, None, tx.clone()).await {
            if !dir_path.exists() {
                panic!("Failed to download fixture {}: {}", name, e);
            }
            continue;
        }

        println!("Extracting fixture {}", name);
        if let Ok(file) = fs::File::open(&zip_path) {
            let mut archive = match zip::ZipArchive::new(file) {
                Ok(a) => a,
                Err(_) => {
                    if dir_path.exists() {
                        continue;
                    }
                    panic!("Failed to open zip archive for {}", name);
                }
            };

            for i in 0..archive.len() {
                let mut file = archive.by_index(i).unwrap();
                let outpath = match file.enclosed_name() {
                    Some(path) => fixtures_dir.join(path),
                    None => continue,
                };

                if file.name().ends_with('/') {
                    let _ = fs::create_dir_all(&outpath);
                } else {
                    if let Some(p) = outpath.parent() {
                        let _ = fs::create_dir_all(&p);
                    }
                    if let Ok(mut outfile) = fs::File::create(&outpath) {
                        let _ = io::copy(&mut file, &mut outfile);
                    }
                }
            }

            // Remove the zip after extraction (ignore if already removed by another test)
            let _ = fs::remove_file(zip_path);
        }

        println!("Fixture {} ready", name);
    }
}

/// Synchronous wrapper for ensure_fixtures for use in standard tests.
pub fn ensure_fixtures_sync() {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(ensure_fixtures(Some(&[
            "250mm-dob-imx464-orion-png",
            "130mm-imx464-dumbell-nebulae-png",
            "130mm-imx464-ring-nebulae-png",
        ])));
}
