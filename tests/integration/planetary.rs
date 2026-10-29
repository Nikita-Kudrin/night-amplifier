//! 2026-10-24's Moon (Ares-C PRO, 3008², 10 ms, gain 0) as a planetary fixture: 30 frames of
//! the 1467 it recorded in 80 s, what each is there for and where it truly lies, and the
//! measure that scores a stack's alignment. Shared with Pro's multi-point tests by `#[path]`.
//!
//! That night each frame took 134 s to align on the Pi, Stop waited for the queue, and no stack
//! was saved. The Moon also drifted 2.5 px/s: from frame 340 on, its shift lay past a ±50 px
//! search around no shift, and 77 % of the session matched on the search's edge at NCC
//! 0.66-0.96, each a wrong shift. The set keeps that drift, the sharpest and blurriest frames,
//! and the frame-to-frame jitter of the first seconds.

use std::path::{Path, PathBuf};

use night_amplifier::frame::Frame;
use night_amplifier::session::state::CaptureSettings;
use night_amplifier::stacking::StackingType;

use super::common::{ensure_fixtures_sync_named, missing_fixture_message, FIXTURES_DIR};

pub const MOON: &str = "moon-drifting-imx533-ser";

/// Each fixture frame: its index in the original capture, why it is in the set, and its shift
/// `(dx, dy)` onto frame 0 (reference - frame), measured by an exhaustive full-resolution NCC
/// search ±60 px around the centroids' estimate over the 1504 px window on the reference's
/// centroid (2026-10-29), in whole pixels. Every frame peaked at NCC 0.998-0.999.
pub const FRAMES: [(usize, &str, (i32, i32)); 30] = [
    (0, "reference: the first frame of the night", (0, 0)),
    (1, "aligned on the Pi in 134 s, logged (0.117, 1.053)", (0, 1)),
    (2, "aligned on the Pi in 134 s, logged (-0.057, 1.408)", (0, 1)),
    (17, "sharpest of the first second", (0, 8)),
    (31, "jitter: 4.4 px from its neighbour", (-1, 13)),
    (32, "jitter: 4.9 px from its neighbour", (0, 17)),
    (33, "jitter: 6.2 px from its neighbour", (-2, 12)),
    (60, "jitter: 5.2 px from its neighbour", (-5, 21)),
    (100, "ordinary", (-5, 26)),
    (168, "jitter: the night's largest jump, 6.4 px", (-6, 47)),
    (200, "ordinary", (-5, 36)),
    (258, "blur: the blurriest of the first 15 s by Laplacian", (-2, 44)),
    (300, "drift: just inside ±50", (-5, 47)),
    (318, "sharpest", (-5, 49)),
    (340, "drift: just past ±50", (-6, 51)),
    (400, "drift", (-6, 60)),
    (493, "sharpest", (-4, 73)),
    (600, "drift", (-6, 92)),
    (700, "ordinary", (-13, 109)),
    (800, "drift", (-15, 123)),
    (856, "blur: the night's blurriest by gradient energy", (-14, 129)),
    (900, "ordinary", (-13, 153)),
    (1000, "drift", (-18, 164)),
    (1107, "sharpest", (-23, 188)),
    (1164, "blur", (-27, 192)),
    (1200, "drift", (-27, 202)),
    (1300, "ordinary", (-30, 228)),
    (1340, "blur: the night's blurriest by Laplacian", (-34, 232)),
    (1400, "drift", (-35, 246)),
    (1466, "drift: the last frame, 255 px from the first", (-35, 255)),
];

/// The fixture's directory, downloaded if missing; panics rather than skips without it.
pub fn moon() -> PathBuf {
    ensure_fixtures_sync_named(&[MOON]);
    let dir = Path::new(FIXTURES_DIR).join(MOON);
    assert!(dir.join("capture.ser").is_file(), "{}", missing_fixture_message(MOON));
    dir
}

/// Planetary stacking at the shipped settings.
pub fn planetary_settings() -> CaptureSettings {
    CaptureSettings { stacking: true, stacking_type: StackingType::Planetary, ..Default::default() }
}

/// Gradient energy of the luminance binned 4x4: crater rims survive the binning, the
/// per-pixel noise a stack averages away does not, so a stack scores its alignment.
pub fn binned_sharpness(frame: &Frame) -> f64 {
    let lum = night_amplifier::planetary::frame_to_luminance(frame);
    let (w, h) = (frame.width() / 4, frame.height() / 4);
    let mut binned = vec![0.0f64; w * h];
    for y in 0..h * 4 {
        for x in 0..w * 4 {
            binned[(y / 4) * w + x / 4] += f64::from(lum[y * frame.width() + x]) / 16.0;
        }
    }
    let mut energy = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = binned[y * w + x + 1] - binned[y * w + x - 1];
            let gy = binned[(y + 1) * w + x] - binned[(y - 1) * w + x];
            energy += gx * gx + gy * gy;
        }
    }
    energy / ((w - 2) * (h - 2)) as f64
}
