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

/// Minimum fraction of frames the live-stacking path must get into the stack.
///
/// Higher than `MIN_STACKING_SUCCESS_RATE` because that constant covers the
/// batch path, which has no quality gate to lose frames to. The live path
/// deliberately drops badly fitted frames — but dropping a third of a clean
/// fixture set means detection, registration, or the gate's thresholds have
/// regressed, not that the fixtures went bad.
///
/// Measured against the stack rather than against admissions, so an early
/// re-base — which discards everything before it — shows up here as the lost
/// integration it is. The managed sets currently sit at 83–100 %.
pub const MIN_LIVE_STACKING_RETENTION: f64 = 0.7;

/// Re-bases a clean fixture set may make before the gate is chasing noise.
///
/// A re-base discards the integration built so far and drops the preview back to
/// a single sub, so it has to be rare and early. Star size is measured from an
/// integer count of pixels above half maximum, which quantises it in ~10 % steps
/// at the sharp end — a margin inside that resolution re-bases on the estimator
/// rather than on the sky.
pub const MAX_REBASES_PER_SESSION: usize = 1;

/// Share of a well-tracked session's frames that may be dropped for a high
/// registration residual.
///
/// "Well tracked" means the session aligns to well inside one star's width, so
/// there is nothing left for the gate to catch. Anything above this is the
/// residual limit having drifted back to scoring a session's precision against
/// itself.
pub const MAX_RESIDUAL_REJECTION_SHARE: f64 = 0.1;

/// Share of a real session Wanderer mode may restart the stack on.
///
/// A frame that genuinely will not register has always meant "the telescope
/// moved", and a rough night produces a few of those. What must not happen is
/// the frame gate's quality verdicts — soft stars, a loose fit — being read the
/// same way, which would restart the integration every time a cloud crossed.
pub const MAX_WANDERER_RESET_SHARE: f64 = 0.25;

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

/// Fixture sets this suite downloads and knows the shape of.
///
/// `tests/fixtures/` is gitignored, so on any given machine it may also hold
/// stray capture output or hand-dropped images. A regression test that asserts
/// on whatever happens to be on disk is not reproducible — assert on these.
pub const MANAGED_FIXTURE_SETS: &[&str] = &[
    "250mm-dob-imx533-dumbbell-fits",
    "250mm-dob-imx464-orion-png",
    "130mm-imx464-dumbell-nebulae-png",
    "130mm-imx464-ring-nebulae-png",
];

/// Supported image file extensions
pub const TIFF_EXTENSIONS: &[&str] = &["tif", "tiff"];
pub const FITS_EXTENSIONS: &[&str] = &["fit", "fits"];
pub const PNG_EXTENSIONS: &[&str] = &["png"];

/// Test output subdirectory names
pub const PROCESSED_OUTPUT_DIR: &str = "processed";
pub const STACKED_OUTPUT_DIR: &str = "stacked";
pub const DEBAYER_OUTPUT_DIR: &str = "debayer";

// ============================================================================
// Planar Layout Detectors
// ============================================================================
//
// `Frame` stores pixels planar (`channel * width * height + y * width + x`) while
// every 8-bit output format is interleaved. Reading a planar buffer as interleaved
// makes each output pixel three *adjacent samples of one channel*, so chroma
// collapses toward zero. That is cheap to assert on and needs no golden image,
// which makes it the detector for the whole class of layout bugs.
//
// Measured on the bundled fixtures: a correct *stretched* colour render scores ~32, a
// planar-read-as-interleaved one ~0.5.
//
// Calibration matters. This threshold only applies **after** background
// neutralisation and stretch. Raw linear data is legitimately near-neutral — a freshly
// debayered sub from the bundled FITS fixture scores 0.72 with nothing wrong with it —
// so asserting this on a linear frame produces a false failure. For raw data, assert a
// scene-independent structural property instead (see the CFA-site check in
// `debayer_tests`).

/// Minimum chroma spread a correct **stretched** colour render of the bundled fixtures
/// must exceed. Do not apply to raw linear frames; see the note above.
pub const MIN_CHROMA_SPREAD: f64 = 5.0;

/// Mean per-pixel `|R-G| + |G-B|` over an interleaved RGB8 buffer, in 0-255 units.
///
/// Samples on a grid rather than every pixel: this runs inside tests on
/// 2712x1538 frames and the statistic converges long before a full traversal.
pub fn mean_chroma_spread_rgb8(rgb8: &[u8], width: usize, height: usize) -> f64 {
    assert_eq!(
        rgb8.len(),
        width * height * 3,
        "mean_chroma_spread_rgb8 expects an interleaved RGB8 buffer"
    );

    let step_x = (width / 40).max(1);
    let step_y = (height / 40).max(1);
    let mut sum = 0.0f64;
    let mut count = 0usize;

    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let idx = (y * width + x) * 3;
            let r = rgb8[idx] as f64;
            let g = rgb8[idx + 1] as f64;
            let b = rgb8[idx + 2] as f64;
            sum += (r - g).abs() + (g - b).abs();
            count += 1;
            x += step_x;
        }
        y += step_y;
    }

    if count == 0 {
        return 0.0;
    }
    sum / count as f64
}

/// Mean per-pixel `|R-G| + |G-B|` over a `Frame`, scaled to 0-255 units.
///
/// Goes through `get_pixel` so the detector is layout-agnostic by construction and
/// cannot inherit the bug it is hunting.
pub fn mean_chroma_spread_frame(frame: &night_amplifier::Frame) -> f64 {
    if frame.channels() < 3 {
        return 0.0;
    }

    let (width, height) = (frame.width(), frame.height());
    let step_x = (width / 40).max(1);
    let step_y = (height / 40).max(1);
    let mut sum = 0.0f64;
    let mut count = 0usize;

    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let r = frame.get_pixel(x, y, 0) as f64 * 255.0;
            let g = frame.get_pixel(x, y, 1) as f64 * 255.0;
            let b = frame.get_pixel(x, y, 2) as f64 * 255.0;
            sum += (r - g).abs() + (g - b).abs();
            count += 1;
            x += step_x;
        }
        y += step_y;
    }

    if count == 0 {
        return 0.0;
    }
    sum / count as f64
}

/// Asserts a render kept its colour. `context` names the path under test.
pub fn assert_has_chroma(spread: f64, context: &str) {
    assert!(
        spread > MIN_CHROMA_SPREAD,
        "{context}: chroma spread {spread:.2} <= {MIN_CHROMA_SPREAD:.2}. \
         Channels have collapsed toward grey, which is the signature of reading \
         the planar Frame buffer as interleaved RGB."
    );
}

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
            // Skip this suite's own output. `stacked/` holds saved stacks and
            // their stretched PNG previews — since PNG became a recognised
            // fixture extension it looked like an eight-frame fixture set of
            // mismatched geometry, and `test_process_all_fixture_sets` was
            // happily "processing" it.
            let name = entry.file_name();
            let name = name.to_str();
            name != Some(PROCESSED_OUTPUT_DIR) && name != Some(STACKED_OUTPUT_DIR)
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
                        || PNG_EXTENSIONS.contains(&ext_lower.as_str())
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
    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("Failed to create output directory {:?}: {}", output_dir, e))?;

    Ok(output_dir)
}

/// Default list of fixture datasets to download.
pub const DEFAULT_FIXTURES: &[(&str, &str)] = &[
    (
        "250mm-dob-imx533-dumbbell-fits",
        "https://drive.usercontent.google.com/download?id=1Xl_Ip539vfWyvP-VWvDxZe4y90hzaAWD&export=download&confirm=t",
    ),
    (
        "250mm-dob-imx464-orion-png",
        "https://drive.usercontent.google.com/download?id=1vKjx5lCFoqhJOcgRLPd4Btcf6Y4j96ap&export=download&confirm=t",
    ),
    (
        "35mm-imx464-orion-tiff",
        "https://drive.usercontent.google.com/download?id=1Qgs51ATx7k5ECdTRwV8ThXE2Lgb2qRqP&export=download&confirm=t",
    ),
    (
        "130mm-imx464-dumbell-nebulae-png",
        "https://drive.usercontent.google.com/download?id=1GYc544x6EZpYmA0S3DUo3XqDo3NiyI7W&export=download&confirm=t",
    ),
    (
        "130mm-imx464-ring-nebulae-png",
        "https://drive.usercontent.google.com/download?id=1qeZJ71NxXdPIuUa3U6SNn_6ZMH6CftF3&export=download&confirm=t",
    ),
];

/// Downloads and extracts test fixture datasets from Google Drive.
///
/// Each fixture is only downloaded once — if the target directory already exists,
/// it is skipped. After downloading, the zip is extracted and removed.
pub async fn ensure_fixtures(names: Option<&[&str]>) {
    use std::fs;
    use std::io;

    let fixtures: Vec<(&str, &str)> = if let Some(names) = names {
        DEFAULT_FIXTURES
            .iter()
            .filter(|(name, _)| names.contains(name))
            .copied()
            .collect()
    } else {
        DEFAULT_FIXTURES.to_vec()
    };

    let fixtures_dir = Path::new(FIXTURES_DIR);
    if !fixtures_dir.exists() {
        tokio::fs::create_dir_all(fixtures_dir)
            .await
            .expect("Failed to create fixtures dir");
    }

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

        const MAX_RETRIES: usize = 3;
        let mut last_error = String::new();

        for attempt in 1..=MAX_RETRIES {
            if attempt > 1 {
                println!(
                    "Retrying fixture {} (attempt {}/{}), waiting 2s...",
                    name, attempt, MAX_RETRIES
                );
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let _ = fs::remove_file(&zip_path);
            }

            println!("Downloading fixture {} from {}", name, url);
            if let Err(e) = download_file(url, &zip_path, name).await {
                if dir_path.exists() {
                    break;
                }
                last_error = format!("Download failed: {}", e);
                continue;
            }

            println!("Extracting fixture {}", name);
            let file = match fs::File::open(&zip_path) {
                Ok(f) => f,
                Err(e) => {
                    last_error = format!("Failed to open downloaded file: {}", e);
                    continue;
                }
            };

            let mut archive = match zip::ZipArchive::new(file) {
                Ok(a) => a,
                Err(e) => {
                    if dir_path.exists() {
                        break;
                    }
                    // Log first bytes to help diagnose what Google Drive returned
                    let diagnostic = fs::read(&zip_path)
                        .ok()
                        .map(|bytes| {
                            let preview_len = bytes.len().min(200);
                            if bytes.starts_with(b"<") || bytes.starts_with(b"<!") {
                                format!(
                                    "file starts with HTML ({} bytes): {}",
                                    bytes.len(),
                                    String::from_utf8_lossy(&bytes[..preview_len])
                                )
                            } else {
                                format!(
                                    "file size: {} bytes, first 16 bytes: {:02x?}",
                                    bytes.len(),
                                    &bytes[..bytes.len().min(16)]
                                )
                            }
                        })
                        .unwrap_or_else(|| "could not read file".to_string());
                    last_error = format!("Invalid zip archive: {} ({})", e, diagnostic);
                    eprintln!("Attempt {}: {}", attempt, last_error);
                    continue;
                }
            };

            for i in 0..archive.len() {
                let mut file = archive.by_index(i).unwrap();
                let outpath = match file.enclosed_name() {
                    Some(path) => fixtures_dir.join(path),
                    None => continue,
                };

                if file.name().ends_with('/') {
                    let _ = std::fs::create_dir_all(&outpath);
                } else {
                    if let Some(p) = outpath.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    if let Ok(mut outfile) = fs::File::create(&outpath) {
                        let _ = io::copy(&mut file, &mut outfile);
                    }
                }
            }

            // Remove the zip after extraction (ignore if already removed by another test)
            let _ = fs::remove_file(&zip_path);
            last_error.clear();
            break;
        }

        if !last_error.is_empty() && !dir_path.exists() {
            panic!(
                "Failed to download/extract fixture {} after {} attempts: {}",
                name, MAX_RETRIES, last_error
            );
        }

        println!("Fixture {} ready", name);

        // Brief cooldown between fixtures to avoid Google Drive rate limiting
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Synchronous wrapper for ensure_fixtures for use in standard tests.
pub fn ensure_fixtures_sync() {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(ensure_fixtures(Some(MANAGED_FIXTURE_SETS)));
}

use regex::Regex;
use reqwest::Client;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{error, info};

type PushToResult<T> = Result<T, String>;

pub async fn download_file(url: &str, dest: &Path, component: &str) -> PushToResult<()> {
    if let Some(p) = dest.parent() {
        tokio::fs::create_dir_all(p)
            .await
            .map_err(|e| format!("Failed to create directories: {}", e))?;
    }

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let response = client.get(url).send().await.map_err(|e| {
        error!(error = %e, url = %url, "Download request failed");
        format!("Download request failed: {}", e)
    })?;

    let status = response.status();
    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/html"));

    // Google Drive can return the virus scan warning as either a non-success
    // status or as a 200 with Content-Type: text/html (for large files via
    // drive.usercontent.google.com).
    if !status.is_success() || is_html {
        let body = response.text().await.unwrap_or_default();
        if body.contains("Google Drive") || body.contains("virus scan") || body.contains("confirm=")
        {
            info!("Hit Google Drive virus scan warning page");
            if let Some(confirm_url) = extract_google_drive_confirm_url(&body, url) {
                let response = client.get(&confirm_url).send().await.map_err(|e| {
                    error!(error = %e, "Confirmation download request failed");
                    format!("Confirmation download request failed: {}", e)
                })?;

                if !response.status().is_success() {
                    return Err(format!(
                        "Confirmation download failed: HTTP {}",
                        response.status()
                    ));
                }
                return download_with_progress(response, dest, component).await;
            }
            return Err(
                "Google Drive virus scan page: could not extract download link".to_string(),
            );
        }

        if !status.is_success() {
            return Err(format!("Download failed: HTTP {}", status));
        }
        return Err("Download returned HTML instead of binary content".to_string());
    }

    download_with_progress(response, dest, component).await
}

async fn download_with_progress(
    mut response: reqwest::Response,
    dest: &Path,
    _component: &str,
) -> PushToResult<()> {
    let _total_size = response.content_length();
    let mut file = File::create(dest).await.map_err(|e| {
        error!(error = %e, path = %dest.display(), "Failed to create file");
        format!("Failed to create file {}: {}", dest.display(), e)
    })?;

    let mut _downloaded: u64 = 0;
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        error!(error = %e, "Download stream error");
        format!("Download stream error: {}", e)
    })? {
        file.write_all(&chunk).await.map_err(|e| {
            error!(error = %e, "Failed to write chunk to file");
            format!("Failed to write to file: {}", e)
        })?;
        _downloaded += chunk.len() as u64;
    }

    Ok(())
}

fn extract_google_drive_confirm_url(html: &str, original_url: &str) -> Option<String> {
    if let Some(id) = extract_google_drive_id(original_url) {
        let re = Regex::new(r#"confirm=([a-zA-Z0-9-_]+)"#).ok()?;
        if let Some(caps) = re.captures(html) {
            let confirm_token = caps.get(1)?.as_str();
            return Some(format!(
                "https://drive.usercontent.google.com/download?id={}&export=download&confirm={}",
                id, confirm_token
            ));
        }
    }

    let re = Regex::new(r#"href="(/uc\?export=download[^"]+)""#).ok()?;
    if let Some(caps) = re.captures(html) {
        let path = caps.get(1)?.as_str();
        return Some(format!("https://drive.google.com{}", path).replace("&amp;", "&"));
    }

    None
}

fn extract_google_drive_id(url: &str) -> Option<String> {
    if let Some(id_param) = url.split("id=").nth(1) {
        let id = id_param.split('&').next().unwrap_or(id_param);
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }

    let re = Regex::new(r"/file/d/([a-zA-Z0-9-_]+)").ok()?;
    if let Some(caps) = re.captures(url) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    None
}
