//! Tests for the raw-CFA stage: hot-pixel rejection, row/column FPN removal and
//! superpixel debayering, measured on the fixtures they were designed against.
//!
//! The two corrections here target defects **stacking cannot remove**: a hot
//! pixel and a readout offset land in the same place in every sub, so averaging
//! frames leaves them exactly where they were. Sky sigma therefore says almost
//! nothing about whether they worked, which is why these tests measure the
//! defects directly — line-median excess for FPN, and star count and FWHM from
//! the real detector for the hot-pixel filter.

use std::path::Path;

use night_amplifier::cfa::{reject_hot_pixels, remove_fpn, CfaFrame, HotPixelConfig};
use night_amplifier::{CfaPattern, DetectionConfig, Frame, StarDetector};
use serial_test::serial;

use crate::integration::common::{find_image_files_in_dir, FIXTURES_DIR};
use crate::integration::image_loading::load_image;

/// The IMX533 fixture: 3008x3008 mono CFA, 2 s at gain 300 on a 250 mm
/// Dobsonian. 5 189 pixels sit persistently above 20 sigma on it.
const FIXTURE: &str = "250mm-dob-imx533-dumbbell-fits";

/// 14-bit data shifted into a 16-bit container, so one ADU of the plan's
/// measurements is one normalized step of 1/65535.
const ADU: f64 = 65535.0;

/// Load the fixture's first frame as the sensor produced it — still mosaiced.
fn fixture_mosaic() -> Option<Frame> {
    let dir = Path::new(FIXTURES_DIR).join(FIXTURE);
    let first = find_image_files_in_dir(&dir).into_iter().next()?;
    let loaded = load_image(&first).ok()?;
    (loaded.frame.channels() == 1).then_some(loaded.frame)
}

fn sorted(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values
}

fn median_of(sorted_values: &[f64]) -> f64 {
    sorted_values[sorted_values.len() / 2]
}

fn mad_sigma(values: &[f64]) -> f64 {
    let centre = median_of(&sorted(values.to_vec()));
    let deviations = sorted(values.iter().map(|v| (v - centre).abs()).collect());
    median_of(&deviations) * 1.4826
}

/// Mean of the central 80 % of a line.
///
/// Not a median: this data is 14-bit shifted into 16 bits, so every sample — and
/// therefore every median — is a multiple of 4 ADU, and a MAD over quantized
/// line medians returns the quantum rather than the spread. A trimmed mean of
/// 1 504 samples is continuous, and trimming keeps stars and the nebula's edge
/// out of it.
fn trimmed_mean(line: Vec<f64>) -> f64 {
    let values = sorted(line);
    let cut = values.len() / 10;
    let core = &values[cut..values.len() - cut];
    core.iter().sum::<f64>() / core.len() as f64
}

/// Spread of one colour site's line levels, in ADU, split into the part a smooth
/// structure could explain and the part that must be line-to-line.
struct LineSpread {
    /// Excess of the whole line-to-line spread over pure noise — the quantity
    /// the plan's fixture table reports.
    total_excess: f64,
    /// Excess measured on *differences between adjacent lines*, so a sky
    /// gradient cannot contribute. This is fixed-pattern noise proper.
    high_frequency_excess: f64,
}

/// The Dumbbell's box in this fixture's full-resolution frame.
///
/// The same region `display_output_tests` measures on the 1440-tier encode,
/// scaled back up by 3008/1440.
const TARGET_BOX: (usize, usize, usize, usize) = (1170, 1170, 1838, 1838);

/// Integrated green-channel flux inside [`TARGET_BOX`].
fn target_flux(frame: &Frame) -> f64 {
    let (x0, y0, x1, y1) = TARGET_BOX;
    let mut flux = 0.0;
    for y in y0..y1.min(frame.height()) {
        for x in x0..x1.min(frame.width()) {
            flux += frame.get_pixel(x, y, 1) as f64;
        }
    }
    flux
}

fn line_spread(frame: &Frame, origin: (usize, usize), horizontal: bool) -> LineSpread {
    let (width, height) = (frame.width(), frame.height());
    let (x0, y0) = origin;

    let mut samples: Vec<f64> = Vec::new();
    for y in (y0..height).step_by(16) {
        for x in (x0..width).step_by(8) {
            samples.push(frame.get_pixel(x, y, 0) as f64 * ADU);
        }
    }
    let sigma = mad_sigma(&samples);

    let outer: Vec<usize> = if horizontal {
        (y0..height).step_by(2).collect()
    } else {
        (x0..width).step_by(2).collect()
    };
    let inner: Vec<usize> = if horizontal {
        (x0..width).step_by(2).collect()
    } else {
        (y0..height).step_by(2).collect()
    };

    let levels: Vec<f64> = outer
        .iter()
        .map(|&i| {
            trimmed_mean(
                inner
                    .iter()
                    .map(|&j| {
                        let (x, y) = if horizontal { (j, i) } else { (i, j) };
                        frame.get_pixel(x, y, 0) as f64 * ADU
                    })
                    .collect(),
            )
        })
        .collect();

    // A 10 % trimmed mean of n normal samples has a standard error of about
    // 1.05 * sigma / sqrt(n); anything above that in quadrature is structure.
    let predicted = 1.05 * sigma / (inner.len() as f64).sqrt();
    let total = mad_sigma(&levels);
    let deltas: Vec<f64> = levels.windows(2).map(|w| w[1] - w[0]).collect();
    let high_frequency = mad_sigma(&deltas) / std::f64::consts::SQRT_2;

    LineSpread {
        total_excess: (total * total - predicted * predicted).max(0.0).sqrt(),
        high_frequency_excess: (high_frequency * high_frequency - predicted * predicted)
            .max(0.0)
            .sqrt(),
    }
}

/// T1.3's acceptance test: the measured row and column excess must fall to the
/// pure-noise prediction.
///
/// Both figures are reported because they say different things. The *total*
/// excess is what the plan's fixture table measured (5.9 ADU per row, 6.7 per
/// column on this sensor) and this reproduces it — but on this fixture nearly
/// all of the column figure turns out to be smooth structure across the frame
/// rather than per-column readout offsets: the high-frequency part of it
/// measures at the noise floor, while a quarter of the row figure survives that
/// test. The correction removes both, which is why it is skipped for planetary
/// targets, where a bright disc fills enough of each line to move its level.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn row_and_column_fpn_falls_to_the_noise_floor() {
    let Some(frame) = fixture_mosaic() else {
        println!("Fixture {FIXTURE} not present. Skipping.");
        return;
    };

    let mut cfa = CfaFrame::mosaic(frame.clone(), CfaPattern::Rggb).unwrap();
    let stats = remove_fpn(&mut cfa).unwrap();
    let corrected = cfa.frame();

    println!("\n=== Row/column FPN, IMX533 fixture (ADU) ===");
    println!(
        "subtracted: row RMS {:.2}, column RMS {:.2}",
        stats.row_rms as f64 * ADU,
        stats.column_rms as f64 * ADU
    );
    println!(
        "{:<8} {:>5} {:>14} {:>13} {:>12} {:>11}",
        "site", "axis", "total before", "total after", "hf before", "hf after"
    );

    for origin in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        for (axis, horizontal) in [("row", true), ("col", false)] {
            let before = line_spread(&frame, origin, horizontal);
            let after = line_spread(corrected, origin, horizontal);

            println!(
                "{:<8} {:>5} {:>14.2} {:>13.2} {:>12.2} {:>11.2}",
                format!("{origin:?}"),
                axis,
                before.total_excess,
                after.total_excess,
                before.high_frequency_excess,
                after.high_frequency_excess
            );

            // The correction is a high-pass, so it is judged on the
            // high-frequency half. Asserting the *total* falls is asserting that
            // real gradients get flattened too, which is the behaviour that
            // drained 5 % of the target's flux.
            assert!(
                after.high_frequency_excess < 0.05,
                "site {origin:?} {axis}: line-to-line excess {:.2} -> {:.2} ADU, \
                 expected the noise floor",
                before.high_frequency_excess,
                after.high_frequency_excess
            );
            assert!(
                after.total_excess <= before.total_excess + 0.05,
                "site {origin:?} {axis}: correction *added* spread, {:.2} -> {:.2} ADU",
                before.total_excess,
                after.total_excess
            );
        }
    }

    // The other half of "tracked the readout, not the signal": the correction
    // subtracts each line's *raw* offset against the reference, so it removes the
    // whole low-frequency component of each axis, not only the line-to-line part.
    // If that were eating real structure it would show up as integrated flux
    // draining out of the target, which is the number nothing else here measures.
    let before_rgb = night_amplifier::debayer_with_pattern(&frame, CfaPattern::Rggb).unwrap();
    let after_rgb = night_amplifier::debayer_with_pattern(corrected, CfaPattern::Rggb).unwrap();
    let flux_before = target_flux(&before_rgb);
    let flux_after = target_flux(&after_rgb);
    let drift = flux_after / flux_before - 1.0;
    println!(
        "target flux: {flux_before:.4e} -> {flux_after:.4e} ({:+.3} %)",
        drift * 100.0
    );
    assert!(
        drift.abs() < 0.002,
        "flattening lines moved integrated target flux by {:.2} % — the correction \
         is removing real large-scale structure along with the readout offsets, and \
         its offsets need high-passing along the axis before they are subtracted",
        drift * 100.0
    );

    // Flattening lines must not cost stars: a correction that tracked the
    // signal instead of the readout would carve bands through the field.
    let detector = StarDetector::new(DetectionConfig::default().with_sigma(5.0));
    let before = detector
        .detect(&night_amplifier::debayer_with_pattern(&frame, CfaPattern::Rggb).unwrap())
        .unwrap();
    let after = detector
        .detect(&night_amplifier::debayer_with_pattern(corrected, CfaPattern::Rggb).unwrap())
        .unwrap();
    println!("stars: {} -> {}", before.len(), after.len());
    assert!(
        after.len() as f64 >= before.len() as f64 * 0.97,
        "star count fell from {} to {}",
        before.len(),
        after.len()
    );
}

/// T1.2's acceptance test. An over-eager filter shows up here and nowhere else:
/// eating star cores lowers the star count and inflates the surviving FWHM, and
/// neither of those moves sky sigma at all.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn hot_pixel_rejection_does_not_cost_stars_or_sharpness() {
    let Some(frame) = fixture_mosaic() else {
        println!("Fixture {FIXTURE} not present. Skipping.");
        return;
    };

    let detector = StarDetector::new(DetectionConfig::default().with_sigma(5.0));
    let before = detector
        .detect(&night_amplifier::debayer_with_pattern(&frame, CfaPattern::Rggb).unwrap())
        .unwrap();

    let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();
    let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();
    let filtered = cfa
        .debayer(night_amplifier::DebayerAlgorithm::Bilinear)
        .unwrap();
    let after = detector.detect(&filtered).unwrap();

    let fwhm_before = night_amplifier::detection::compute_median_fwhm(&before);
    let fwhm_after = night_amplifier::detection::compute_median_fwhm(&after);

    println!("\n=== Hot-pixel rejection, IMX533 fixture ===");
    println!("corrected samples: {}", stats.corrected);
    println!("stars: {} -> {}", before.len(), after.len());
    println!("median FWHM: {fwhm_before:?} -> {fwhm_after:?}");

    assert!(
        stats.corrected > 500,
        "expected the fixture's measured hot pixels to be found, got {}",
        stats.corrected
    );
    assert!(
        after.len() as f64 >= before.len() as f64 * 0.97,
        "star count fell from {} to {} — the filter is eating stars",
        before.len(),
        after.len()
    );
    if let (Some(b), Some(a)) = (fwhm_before, fwhm_after) {
        assert!(
            a <= b * 1.05,
            "median FWHM degraded from {b:.3} to {a:.3} — star cores are being clipped"
        );
    }
}

/// The synthetic half of T1.2's verification: known hot sites and known stars in
/// one frame, so "removed the defects" and "kept the signal" are both checked
/// against ground truth rather than against a previous measurement.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn known_hot_sites_go_and_known_stars_stay() {
    let (width, height) = (512usize, 512usize);
    let mut frame = Frame::zeros(width, height, 1).unwrap();

    let mut seed = 0xC0FFEEu32;
    let mut noise = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    };
    for y in 0..height {
        for x in 0..width {
            frame.set_pixel(x, y, 0, 0.02 + noise() * 0.004);
        }
    }

    // 40 stars on a grid, each a Gaussian a few samples across.
    let star_sites: Vec<(usize, usize)> = (0..40)
        .map(|i| (48 + (i % 8) * 56, 48 + (i / 8) * 88))
        .collect();
    for &(cx, cy) in &star_sites {
        let peak = 0.25 + (cx % 7) as f32 * 0.05;
        for dy in -8i32..=8 {
            for dx in -8i32..=8 {
                let r2 = (dx * dx + dy * dy) as f32;
                let (x, y) = ((cx as i32 + dx) as usize, (cy as i32 + dy) as usize);
                let v = frame.get_pixel(x, y, 0) + peak * (-r2 / 10.0).exp();
                frame.set_pixel(x, y, 0, v);
            }
        }
    }

    // 60 hot sites, deliberately away from the stars.
    let hot_sites: Vec<(usize, usize)> = (0..60)
        .map(|i| (20 + (i % 10) * 50, 20 + (i / 10) * 80))
        .filter(|&(x, y): &(usize, usize)| {
            !star_sites
                .iter()
                .any(|&(sx, sy)| x.abs_diff(sx) < 12 && y.abs_diff(sy) < 12)
        })
        .collect();
    for &(x, y) in &hot_sites {
        frame.set_pixel(x, y, 0, 0.85);
    }

    let detector = StarDetector::new(DetectionConfig::default().with_sigma(5.0));
    let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();
    let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();
    let filtered = cfa.frame();

    println!("\n=== Synthetic hot sites and stars ===");
    println!(
        "injected {} hot sites, corrected {}",
        hot_sites.len(),
        stats.corrected
    );

    for &(x, y) in &hot_sites {
        assert!(
            filtered.get_pixel(x, y, 0) < 0.1,
            "hot site at ({x}, {y}) survived at {}",
            filtered.get_pixel(x, y, 0)
        );
    }

    let stars = detector
        .detect(&night_amplifier::debayer_with_pattern(filtered, CfaPattern::Rggb).unwrap())
        .unwrap();
    println!(
        "stars detected after filtering: {} of {}",
        stars.len(),
        star_sites.len()
    );
    for &(cx, cy) in &star_sites {
        assert!(
            stars
                .iter()
                .any(|s| (s.x - cx as f32).abs() < 3.0 && (s.y - cy as f32).abs() < 3.0),
            "star at ({cx}, {cy}) was lost"
        );
    }
}

/// T1.4: superpixel binning must land the IMX533 above the eyepiece screen while
/// removing the interpolated chroma that bilinear invents from a mono source.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn superpixel_debayer_halves_the_frame_and_keeps_it_above_the_eyepiece_screen() {
    let Some(frame) = fixture_mosaic() else {
        println!("Fixture {FIXTURE} not present. Skipping.");
        return;
    };
    let (width, height) = (frame.width(), frame.height());

    let binned = CfaFrame::mosaic(frame, CfaPattern::Rggb)
        .unwrap()
        .debayer(night_amplifier::DebayerAlgorithm::Superpixel)
        .unwrap();

    println!(
        "\n=== Superpixel debayer ===\n{width}x{height} -> {}x{}",
        binned.width(),
        binned.height()
    );

    assert_eq!((binned.width(), binned.height()), (width / 2, height / 2));
    assert_eq!(binned.channels(), 3);
    assert!(
        binned.width() >= 1440 && binned.height() >= 1440,
        "IMX533 binned to {}x{} would now be below the 1440 eyepiece screen",
        binned.width(),
        binned.height()
    );
}
