use crate::error::Result;
use crate::frame::Frame;

use super::config::DetectionConfig;
use super::detector::StarDetector;
use super::star::Star;

/// Number of stars the first rung must find before the more permissive — and
/// slower — fallbacks are skipped.
const ENOUGH_STARS: usize = 10;

/// Star cap for every rung of the ladder.
///
/// Registration quality is limited by how much of the star field it can see. On a
/// 9 MP sub the old 30-star first rung registered 26 of 34 fixture frames with a
/// 2.6 px mean residual; lifting the cap takes that to 33 of 34 at 1.7 px for the
/// same 25 ms, because the cost is in the threshold pass, not in the stars kept.
const MAX_STARS: usize = 200;

/// Detects stars with adaptive parameters based on image characteristics
pub fn detect_stars_adaptive(frame: &Frame) -> Result<Vec<Star>> {
    // First rung: default sensitivity. `fast()` used to sit here, but its 30-star
    // cap starved registration and its 3 px centroid radius is too small to
    // measure a typical star — `compute_fwhm` hit the window border and returned
    // None on half the frames, which silently neutered quality weighting too.
    let detector = StarDetector::new(DetectionConfig::default().with_max_stars(MAX_STARS));
    if let Ok(stars) = detector.detect(frame) {
        if stars.len() >= ENOUGH_STARS {
            return Ok(stars);
        }
    }

    // Second attempt: sensitive config
    let detector = StarDetector::new(DetectionConfig::sensitive().with_max_stars(MAX_STARS));
    if let Ok(stars) = detector.detect(frame) {
        if stars.len() >= 3 {
            return Ok(stars);
        }
    }

    // Final fallback: aggressive
    StarDetector::new(DetectionConfig::aggressive().with_max_stars(MAX_STARS)).detect(frame)
}

/// Full adaptive detection that tries more configurations (slower)
pub fn detect_stars_adaptive_thorough(frame: &Frame) -> Result<Vec<Star>> {
    let stats = analyze_image(frame);
    let initial_config = choose_config(&stats);

    let configs = [
        initial_config,
        DetectionConfig::default().with_sigma(3.0).with_min_snr(3.0),
        DetectionConfig::sensitive(),
        DetectionConfig::aggressive(),
    ];

    for config in configs {
        if let Ok(stars) = StarDetector::new(config).detect(frame) {
            if stars.len() >= 3 {
                return Ok(stars);
            }
        }
    }

    StarDetector::new(DetectionConfig::aggressive()).detect(frame)
}

/// Only some fields are read; the rest are kept because the struct is the
/// analysis record a `Debug` log prints whole.
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ImageAnalysis {
    median: f32,
    sigma: f32,
    max_value: f32,
    dynamic_range: f32,
    is_underexposed: bool,
    has_bright_sources: bool,
}

fn analyze_image(frame: &Frame) -> ImageAnalysis {
    let luminance = crate::detection::luminance::mean_luminance(frame);

    let sample_size = 50_000.min(luminance.len());
    let step = luminance.len() / sample_size;
    let mut sample: Vec<f32> = luminance.iter().step_by(step.max(1)).copied().collect();
    sample.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let median = sample[sample.len() / 2];
    let max_value = luminance.iter().cloned().fold(f32::MIN, f32::max);

    let mut deviations: Vec<f32> = sample.iter().map(|&v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mad = deviations[deviations.len() / 2];
    let sigma = (mad * 1.4826).max(1e-6);

    let dynamic_range = max_value / median.max(1e-6);

    ImageAnalysis {
        median,
        sigma,
        max_value,
        dynamic_range,
        is_underexposed: median < 0.05,
        has_bright_sources: dynamic_range > 10.0,
    }
}

fn choose_config(analysis: &ImageAnalysis) -> DetectionConfig {
    if analysis.is_underexposed {
        return DetectionConfig::sensitive();
    }

    if analysis.has_bright_sources && analysis.dynamic_range > 50.0 {
        return DetectionConfig::default();
    }

    DetectionConfig::default().with_sigma(4.0).with_min_snr(3.0)
}
