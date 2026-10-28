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

/// Minimum fraction of frames the live-stacking path must get into the stack.
///
/// The live path deliberately drops badly fitted frames, but losing a third
/// of a clean fixture set means detection, registration, or the gate's thresholds regressed,
/// not that the fixtures went bad. Measured against the stack, not admissions, so an early
/// re-base (which discards everything before it) shows up here as the lost integration it is.
/// Managed sets sit at 83–100 %.
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
    "cats-eye-nebula-imx533",
    "hercules-globular-cluster-imx533-fixture",
    "iris-nebula-imx533",
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
// `Frame` stores pixels planar (`channel*width*height + y*width + x`); every 8-bit format is
// interleaved, so reading planar as interleaved makes each pixel three adjacent samples of one
// channel, collapsing chroma toward zero — cheap to assert, no golden image needed (stretched
// fixture ~32, planar-as-interleaved ~0.5). Valid only after stretch: raw data is near-neutral
// (fresh sub scores 0.72 clean) — for raw data assert a structural property instead (`debayer_tests`).

/// Minimum chroma spread a correct **stretched** colour render of the bundled fixtures must
/// exceed. Do not apply to raw linear frames; see the note above.
///
/// A floor against channels collapsing (all-or-nothing: planar-as-interleaved scores ~0, not
/// a few percent low). Was 5.0, until raising `ContrastConfig`'s strength to 1.0 tripped it
/// at 4.97 — full intensity puts more of the frame in the curve's toe, costing a little
/// chroma. 4.0 keeps an order of magnitude over the defect while staying off that measurement.
pub const MIN_CHROMA_SPREAD: f64 = 4.0;

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
    println!("    chroma spread {spread:.2} ({context})");
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

/// A published fixture set: where it downloads from, and what its archive holds.
pub struct Fixture {
    pub name: &'static str,
    pub url: &'static str,
    pub contents: Contents,
}

/// A set's files and their bytes, counted as [`contents_of`] counts (dotfiles aside, at any
/// depth). A set on disk counts as present only if it holds what its archive does: a
/// hand-cut 1504² IC 59 under the published 3008² set's name passed unseen, and a bound
/// measured on it failed on every CI run. To register a set, put zeros here and copy the
/// counts the refusal prints for the set you zipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contents {
    pub files: usize,
    pub bytes: u64,
}

/// Default list of fixture datasets to download.
pub const DEFAULT_FIXTURES: &[Fixture] = &[
    Fixture {
        name: "250mm-dob-imx533-dumbbell-fits",
        url: "https://drive.usercontent.google.com/download?id=1Xl_Ip539vfWyvP-VWvDxZe4y90hzaAWD&export=download&confirm=t",
        contents: Contents { files: 35, bytes: 633_528_000 },
    },
    Fixture {
        name: "250mm-dob-imx464-orion-png",
        url: "https://drive.usercontent.google.com/download?id=1vKjx5lCFoqhJOcgRLPd4Btcf6Y4j96ap&export=download&confirm=t",
        contents: Contents { files: 30, bytes: 84_841_573 },
    },
    Fixture {
        name: "35mm-imx464-orion-tiff",
        url: "https://drive.usercontent.google.com/download?id=1Qgs51ATx7k5ECdTRwV8ThXE2Lgb2qRqP&export=download&confirm=t",
        contents: Contents { files: 17, bytes: 141_818_386 },
    },
    Fixture {
        name: "130mm-imx464-dumbell-nebulae-png",
        url: "https://drive.usercontent.google.com/download?id=1GYc544x6EZpYmA0S3DUo3XqDo3NiyI7W&export=download&confirm=t",
        contents: Contents { files: 20, bytes: 114_838_374 },
    },
    Fixture {
        name: "130mm-imx464-ring-nebulae-png",
        url: "https://drive.usercontent.google.com/download?id=1qeZJ71NxXdPIuUa3U6SNn_6ZMH6CftF3&export=download&confirm=t",
        contents: Contents { files: 20, bytes: 106_885_504 },
    },
    Fixture {
        name: "176mm-imx464-m101-fits",
        url: "https://drive.usercontent.google.com/download?id=1R4Tjcj5Ztkp4JSnzag2oXsq29ftt98VP&export=download&confirm=t",
        contents: Contents { files: 3, bytes: 16_693_801 },
    },
    Fixture {
        name: "176mm-imx464-delphinus-fits",
        url: "https://drive.usercontent.google.com/download?id=1fCEHKLGFsLNasuO4ebUqoIDcGwx882PU&export=download&confirm=t",
        contents: Contents { files: 2, bytes: 8_347_112 },
    },
    // Guide frames whose hot pixels, spread by bilinear demosaic, stop ASTAP solving.
    Fixture {
        name: "176mm-imx464-guide-hot-pixels-fits",
        url: "https://drive.usercontent.google.com/download?id=1wiLkVvM-yNS9LFXHTxEO-odscGq0Bz41&export=download&confirm=t",
        contents: Contents { files: 4, bytes: 25_039_942 },
    },
    // The same guide rig across a 5 s/g376 -> 0.5 s/g123 change: a noise estimate carried
    // over between them stopped the short sub plate-solving.
    Fixture {
        name: "176mm-imx464-guide-exposure-change-fits",
        url: "https://drive.usercontent.google.com/download?id=1ZeJEc_OC33oz_QuoNdkJOIgjxxE-Szc-&export=download&confirm=t",
        contents: Contents { files: 2, bytes: 16_692_480 },
    },
    // Guide frames from the 2026-09-14 Lagoon push: one still pointing that Push-To solved
    // twice (134-142), then the push onto M8 (183, 184, 187).
    Fixture {
        name: "176mm-imx464-guide-lagoon-fits",
        url: "https://drive.usercontent.google.com/download?id=1O4lUyZZtH8XJeb_Ef7MDuuo1LhVT9Cc4&export=download&confirm=t",
        contents: Contents { files: 13, bytes: 100_157_682 },
    },
    // Guide frames from the 2026-10-12 hunt for M101, then the Spindle Galaxy: frames ASTAP
    // solves under exit code 2, two full-sky false matches, one-off bad frames (a meteor,
    // two shaking after a push) beside neighbours that solve, and the OpenNGC rows of both
    // Spindles (`push_to_m101_spindle_replay_tests`, Pro).
    Fixture {
        name: "176mm-imx464-guide-m101-spindle-fits",
        url: "https://drive.usercontent.google.com/download?id=1W6vYiyFl0Bq6sgMsUgME13Slw2CicL4g&export=download&confirm=t",
        contents: Contents { files: 16, bytes: 108_507_528 },
    },
    // 16 subs of M27, cropped to 1024x1024 on an even origin so the RGGB phase survives.
    // Deliberately not the whole session: this is the shortest run that still shows the
    // render collapsing as the stack deepens, which is what `stack_depth_regression`
    // asserts against.
    Fixture {
        name: "stack-depth-dumbbell",
        url: "https://drive.usercontent.google.com/download?id=1MKQqxU3QSqWqg2wjGuLXmRhTW-iTaCVm&export=download&confirm=t",
        contents: Contents { files: 16, bytes: 33_638_400 },
    },
    // The whole 106-sub M27 session (IMX533, 5 s, 250 mm Dob), cropped to 1024x1024 on an
    // even origin around the nebula. The depth is the point and 16 subs cannot stand in
    // for it: the tone curve's depth gain only stops paying once a real session's sigma
    // stops falling as sqrt(N), which happens past ~32 frames (`stack_depth_grain_tests`).
    Fixture {
        name: "deep-stack-dumbbell-106",
        url: "https://drive.usercontent.google.com/download?id=1WSXWaRnIxQ6zX3faFdXV3wZsrHOFIEMe&export=download&confirm=t",
        contents: Contents { files: 106, bytes: 222_854_400 },
    },
    // 12 subs of a globular (IMX533, 6 s) from the 2026-09-14 session that rendered blocky
    // at the eyepiece, cropped to 2048x2048 on an even origin around the cluster.
    // `eyepiece_globular_regression` (Pro) asserts the downsample lattice, black-floor
    // grain and RBF halo against it.
    Fixture {
        name: "globular-cluster-eyepiece",
        url: "https://drive.usercontent.google.com/download?id=13TacCPmug7mM1F32WHGX3EYLGEm0RvVE&export=download&confirm=t",
        contents: Contents { files: 12, bytes: 100_707_840 },
    },
    Fixture {
        name: "ic-59-ghost-of-cassiopeia-nebula",
        url: "https://drive.usercontent.google.com/download?id=1LsFIKXtybn-bWGMmEtMMs6VR0qFaJ8Vi&export=download&confirm=t",
        contents: Contents { files: 15, bytes: 271_512_000 },
    },
    Fixture {
        name: "m33-triangulum-galaxy-imx533",
        url: "https://drive.usercontent.google.com/download?id=19Zi6f8ItzVpqdqQgFMp-k5ocxvEd5gBx&export=download&confirm=t",
        contents: Contents { files: 35, bytes: 633_528_000 },
    },
    Fixture {
        name: "m101-pinwheel-galaxy-imx533",
        url: "https://drive.usercontent.google.com/download?id=13QC7HvBGJGzSB3oDzmGeBWY-ModyV5Tn&export=download&confirm=t",
        contents: Contents { files: 39, bytes: 705_931_200 },
    },
    // 27 subs of NGC 6543 (IMX533, 0.7 s at gain 0, 250 mm Dob) from 2026-09-20: ~15 real
    // stars a sub against a nebula that shatters into ~45 brighter detections. Every sub
    // after the reference was refused (`thin_subs_of_a_bright_nebula_still_stack`).
    Fixture {
        name: "cats-eye-nebula-imx533",
        url: "https://drive.usercontent.google.com/download?id=1udM7sCdeknz_lhZihoDDLwqaGA4en8ZD&export=download&confirm=t",
        contents: Contents { files: 27, bytes: 488_721_600 },
    },
    // 64 subs of M33 (IMX533, 7 s), cropped to 1024x1024 on an even origin: the field drifts
    // across the crop, so only ~56 % of it holds every sub by the end — the hard case for
    // `auto_crop_tests` (crop area, change rate, the border skewing the stretch's statistics).
    Fixture {
        name: "m33-drifting-imx533-1024",
        url: "https://drive.usercontent.google.com/download?id=1EHrsxIVrU7A5-Qsj2K-IoakXdeq5Gb2A&export=download&confirm=t",
        contents: Contents { files: 64, bytes: 134_553_600 },
    },
    // M57 (two sessions) and NGC 6946 sessions (IMX533) whose stacks carry a strong one-sided
    // gradient (moon/light pollution) over a quarter to half of the frame
    // (`background_gradient_tests`).
    Fixture {
        name: "ring-nebula-imx533",
        url: "https://drive.usercontent.google.com/download?id=1h8-up54TbYq3QwQR2j3PNnJvNqzYTtlC&export=download&confirm=t",
        contents: Contents { files: 38, bytes: 687_830_400 },
    },
    Fixture {
        name: "ring-2-nebula-fixture-imx533",
        url: "https://drive.usercontent.google.com/download?id=191XhtUIbbIitqaUZJAXdpTrYjOl-BQuS&export=download&confirm=t",
        contents: Contents { files: 43, bytes: 778_334_400 },
    },
    Fixture {
        name: "fireworks-galaxy-imx533",
        url: "https://drive.usercontent.google.com/download?id=17Nm3Jooy2_X30dLdo_BS1odYpsNG6mzT&export=download&confirm=t",
        contents: Contents { files: 70, bytes: 1_267_056_000 },
    },
    // Four real sessions whose first subs are bad, cut from the sets above to 1536² on even
    // origins and reordered: M101 opening on a satellite trail, Fireworks on a bump and on
    // its own smeared sub 1, the 250 mm M27 on its bumped sub 1. Each `manifest.txt` names
    // the bad subs (`early_subs_tests`).
    Fixture {
        name: "early-bad-subs-imx533",
        url: "https://drive.usercontent.google.com/download?id=174IQ2R_pA8onDB_nR0PBvMd1gqPEL4p2&export=download&confirm=t",
        contents: Contents { files: 68, bytes: 302_286_166 },
    },
    // Display-size (bin 2) stacks of the ten sessions the background model is judged on,
    // with `depths.txt`: what the render task hands the background stage at 1440p. Cut
    // by Pro's `background_gradient_diagnostics::cut_stack_fixture`, so the regression
    // (`background_gradient_regression`) needs ~300 MB instead of every session's subs.
    // IC 59's stack is re-cut from the published set: the last upload's was a 752² crop of
    // a hand-cut copy, so IC 63's position held for it alone.
    Fixture {
        name: "background-gradient-stacks",
        url: "https://drive.usercontent.google.com/download?id=1bM7sDcimSbVvQtK4hL9-iYNFbYsn9Nlg&export=download&confirm=t",
        contents: Contents { files: 11, bytes: 242_332_133 },
    },
    // Four 2026-10-24 IMX533 sessions cut like `background-gradient-stacks` but auto-cropped
    // as production shows them (Pro's `cut_capture_stacks`): M57 and the Iris beside an
    // instrument glow peaking at 0.85-0.90 of the sensor width, NGC 40, and M13 filling the
    // frame. The glow is what both models left as a band on screen.
    Fixture {
        name: "background-capture-stacks",
        url: "https://drive.usercontent.google.com/download?id=1rEYLT7PrxGedIYcJPatNdcd6o4v3LoeS&export=download&confirm=t",
        contents: Contents { files: 5, bytes: 101_998_167 },
    },
    // Two of those sessions' raw subs, whole (Ares-C PRO, 3008², gain 126): M13 filling the
    // frame in 75 subs of 0.4 s, and the Iris (NGC 7023) in 63 of 5 s.
    Fixture {
        name: "hercules-globular-cluster-imx533-fixture",
        url: "https://drive.usercontent.google.com/download?id=1EXDudBM5ovlbTq10OId3sR7TjKt-Uux9&export=download&confirm=t",
        contents: Contents { files: 75, bytes: 1_357_560_000 },
    },
    Fixture {
        name: "iris-nebula-imx533",
        url: "https://drive.usercontent.google.com/download?id=1e91M7PAjp_VxjVDaST7BN05vHVfal6IB&export=download&confirm=t",
        contents: Contents { files: 63, bytes: 1_140_350_400 },
    },
];

/// Marks a `DEFAULT_FIXTURES` entry whose archive has been cut but not yet published.
///
/// The set is registered at the same time as the test that needs it, so the two cannot
/// drift apart; replacing this in the URL is the only step left. Its contents are
/// registered with it, so the author's local cut is held to them like any download. A test
/// reaching a pending set it does not have must fail — see [`missing_fixture_message`].
pub const PENDING_UPLOAD: &str = "PENDING_UPLOAD";

fn is_pending_upload(url: &str) -> bool {
    url.contains(PENDING_UPLOAD)
}

/// What the set at `dir` holds: every file under it, dotfiles aside at any depth (a Finder
/// `.DS_Store` is no frame, and no loader reads one).
pub fn contents_of(dir: &Path) -> std::io::Result<Contents> {
    let mut contents = Contents { files: 0, bytes: 0 };
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let meta = fs::metadata(entry.path())?;
        if meta.is_dir() {
            let inner = contents_of(&entry.path())?;
            contents.files += inner.files;
            contents.bytes += inner.bytes;
        } else {
            contents.files += 1;
            contents.bytes += meta.len();
        }
    }
    Ok(contents)
}

/// Whether the set at `dir` is the one `fixture` publishes. Read-only, so a download in
/// another process can never show it half-marked; refused, never replaced, since a copy
/// that differs may be someone's own work.
pub fn check_fixture_contents(dir: &Path, fixture: &Fixture) -> Result<(), String> {
    let found = contents_of(dir).map_err(|e| format!("fixture {} at {}: {e}", fixture.name, dir.display()))?;
    if found == fixture.contents {
        return Ok(());
    }
    Err(format!(
        "fixture {} at {} holds {} files / {} bytes, but DEFAULT_FIXTURES registers {} files / \
         {} bytes: it is not the set CI measures (cut or edited by hand, or from an older link). \
         Move it away and the tests download the published one",
        fixture.name,
        dir.display(),
        found.files,
        found.bytes,
        fixture.contents.files,
        fixture.contents.bytes
    ))
}

/// Why `name` is not on disk, said in a way that names the next action.
pub fn missing_fixture_message(name: &str) -> String {
    let pending = DEFAULT_FIXTURES
        .iter()
        .any(|fixture| fixture.name == name && is_pending_upload(fixture.url));
    if pending {
        format!(
            "fixture {name} is registered in DEFAULT_FIXTURES as {PENDING_UPLOAD}: the archive \
             exists but has not been uploaded, so replace the placeholder id with the real \
             download link"
        )
    } else {
        format!("fixture {name} is missing or too short to stack")
    }
}

/// Downloads and extracts test fixture datasets from Google Drive.
///
/// A fixture directory only ever appears complete: the archive is downloaded and
/// extracted under a per-process staging name and renamed into place. Extracting in place
/// let parallel tests that each found the directory missing read a FITS another test was
/// still writing — and nextest runs every test in its own process, so an in-process lock
/// alone cannot prevent that. The lock here only stops one binary downloading N copies.
pub async fn ensure_fixtures(names: Option<&[&str]>) {
    use std::fs;
    use std::sync::OnceLock;

    static SERIAL: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _serial = SERIAL
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;

    let fixtures: Vec<&Fixture> = DEFAULT_FIXTURES
        .iter()
        .filter(|fixture| names.is_none_or(|names| names.contains(&fixture.name)))
        .collect();

    let fixtures_dir = Path::new(FIXTURES_DIR);
    if !fixtures_dir.exists() {
        tokio::fs::create_dir_all(fixtures_dir)
            .await
            .expect("Failed to create fixtures dir");
    }

    for fixture in fixtures {
        let (name, url) = (fixture.name, fixture.url);
        let dir_path = fixtures_dir.join(name);
        if dir_path.exists() {
            check_fixture_contents(&dir_path, fixture).unwrap_or_else(|e| panic!("{e}"));
            continue;
        }
        if is_pending_upload(url) {
            // Registered, cut, not published yet. Nothing to fetch, and retrying three
            // times to save Google Drive's error page as a zip helps nobody. The tests
            // that want it fail on their own with a message naming this state — the one
            // thing that must not happen is their passing without the data.
            println!("Fixture {name} is registered but not published yet (PENDING_UPLOAD)");
            continue;
        }

        let pid = std::process::id();
        let zip_path = fixtures_dir.join(format!(".{name}.{pid}.zip"));
        let staging = fixtures_dir.join(format!(".{name}.{pid}.staging"));

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

            let _ = fs::remove_dir_all(&staging);
            let extracted = extract_archive(&mut archive, &staging);
            let _ = fs::remove_file(&zip_path);
            if extracted.is_ok() && staging.join(name).is_dir() {
                // The registry's mistake, not the download's: a retry fetches the same archive.
                let staged = check_fixture_contents(&staging.join(name), fixture);
                if let Err(e) = staged {
                    let _ = fs::remove_dir_all(&staging);
                    panic!("{url} no longer publishes the registered set — update its contents: {e}");
                }
            }
            if let Err(e) = extracted.and_then(|()| publish_staged(&staging, name, &dir_path)) {
                let _ = fs::remove_dir_all(&staging);
                if dir_path.exists() {
                    break;
                }
                last_error = e;
                continue;
            }
            let _ = fs::remove_dir_all(&staging);
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

fn extract_archive(
    archive: &mut zip::ZipArchive<std::fs::File>,
    into: &Path,
) -> Result<(), String> {
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Unreadable zip entry {i}: {e}"))?;
        let Some(outpath) = entry.enclosed_name().map(|p| into.join(p)) else {
            continue;
        };
        if entry.is_dir() {
            fs::create_dir_all(&outpath).map_err(|e| format!("{}: {e}", outpath.display()))?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut outfile =
            fs::File::create(&outpath).map_err(|e| format!("{}: {e}", outpath.display()))?;
        std::io::copy(&mut entry, &mut outfile)
            .map_err(|e| format!("Failed to extract {}: {e}", outpath.display()))?;
    }
    Ok(())
}

/// Move `staging/<name>` to `dir_path` in one rename. Losing that rename to another
/// process that published the same fixture first is success, not an error, and leaves the
/// winner's set untouched.
pub(crate) fn publish_staged(staging: &Path, name: &str, dir_path: &Path) -> Result<(), String> {
    let staged = staging.join(name);
    if !staged.is_dir() {
        return Err(format!("archive has no top-level {name}/ directory"));
    }
    match fs::rename(&staged, dir_path) {
        Ok(()) => Ok(()),
        Err(_) if dir_path.is_dir() => Ok(()),
        Err(e) => Err(format!("Failed to move {name} into place: {e}")),
    }
}

/// Synchronous wrapper for ensure_fixtures for use in standard tests.
pub fn ensure_fixtures_sync() {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(ensure_fixtures(Some(MANAGED_FIXTURE_SETS)));
}

/// [`ensure_fixtures_sync`] for a test that wants named sets rather than the managed
/// list — a single-dataset test should not pull the other four down to run.
pub fn ensure_fixtures_sync_named(names: &[&str]) {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(ensure_fixtures(Some(names)));
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

    // Google Drive can return the virus scan warning as either a non-success
    // status or as a 200 with Content-Type: text/html (for large files via
    // drive.usercontent.google.com).
    if !status.is_success() || is_html(&response) {
        let body = read_unexpected(response).await;
        let title = page_title(&body).unwrap_or("untitled");
        if body.contains("Google Drive") || body.contains("virus scan") || body.contains("confirm=")
        {
            info!("Hit Google Drive virus scan warning page");
            if let Some(confirm_url) = extract_google_drive_confirm_url(&body, url) {
                let response = client.get(&confirm_url).send().await.map_err(|e| {
                    error!(error = %e, "Confirmation download request failed");
                    format!("Confirmation download request failed: {}", e)
                })?;

                let status = response.status();
                if !status.is_success() || is_html(&response) {
                    let body = read_unexpected(response).await;
                    let title = page_title(&body).unwrap_or("untitled");
                    return Err(format!("Confirmation download failed: HTTP {status}, page \"{title}\""));
                }
                return download_with_progress(response, dest, component).await;
            }
            return Err(format!("Google Drive page with no download link: \"{title}\""));
        }

        if !status.is_success() {
            return Err(format!("Download failed: HTTP {status}, page \"{title}\""));
        }
        return Err(format!("Download returned HTML instead of binary content: \"{title}\""));
    }

    download_with_progress(response, dest, component).await
}

fn is_html(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/html"))
}

/// Reads a response that is not the archive and prints it: Drive refuses with a page, not
/// an error code, and a test's output is all CI keeps of why (2026-10-28 lost 71 tests to a
/// page nobody saw).
async fn read_unexpected(response: reqwest::Response) -> String {
    let status = response.status();
    let url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("no content type")
        .to_string();
    let body = response.text().await.unwrap_or_default();
    eprintln!("{}", unexpected_page_report(&url, status, &content_type, &body));
    body
}

/// A page longer than this is cut: the reason sits near the top, and every retry prints it.
pub(crate) const MAX_LOGGED_PAGE: usize = 64 * 1024;

pub(crate) fn unexpected_page_report(
    url: &str,
    status: reqwest::StatusCode,
    content_type: &str,
    body: &str,
) -> String {
    let shown = &body[..body.floor_char_boundary(MAX_LOGGED_PAGE)];
    let cut = if shown.len() < body.len() { "\n[truncated]" } else { "" };
    format!(
        "{url} answered HTTP {status} ({content_type}, {} bytes) instead of the archive:\n{shown}{cut}",
        body.len()
    )
}

pub(crate) fn page_title(html: &str) -> Option<&str> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").ok()?;
    let title = re.captures(html)?.get(1)?.as_str().trim();
    (!title.is_empty()).then_some(title)
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

    // `tokio::fs::File` hands writes to a background blocking task, and dropping it
    // does not wait for them. Without this the caller can open the file and read a
    // short prefix — which surfaces as "Could not find EOCD" on an archive that is
    // perfectly valid a moment later, and retrying cannot help because the next
    // attempt loses the same race. `shutdown` flushes and awaits the pending op.
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush {}: {}", dest.display(), e))?;
    file.shutdown()
        .await
        .map_err(|e| format!("Failed to close {}: {}", dest.display(), e))?;

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
