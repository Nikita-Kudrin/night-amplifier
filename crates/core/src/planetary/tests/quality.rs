use super::common::{create_planetary_frame, create_test_frame};
use super::*;

#[test]
fn test_quality_metrics() {
    let frame = create_test_frame(64, 64, 0.0, 0.0);

    let laplacian = compute_quality(&frame, QualityMetric::Laplacian);
    let sobel = compute_quality(&frame, QualityMetric::Sobel);
    let tenengrad = compute_quality(&frame, QualityMetric::Tenengrad);
    let std_dev = compute_quality(&frame, QualityMetric::StdDev);

    assert!(laplacian > 0.0, "Laplacian should be positive");
    assert!(sobel > 0.0, "Sobel should be positive");
    assert!(tenengrad > 0.0, "Tenengrad should be positive");
    assert!(std_dev > 0.0, "StdDev should be positive");
}

#[test]
fn test_quality_metric_distinguishes_sharp_from_blurry() {
    let sharp_frame = create_planetary_frame(64, 64, 0.0, 0.0, 0.0);
    let blurry_frame = create_planetary_frame(64, 64, 0.0, 0.0, 5.0);

    for metric in [
        QualityMetric::Laplacian,
        QualityMetric::Sobel,
        QualityMetric::Tenengrad,
    ] {
        let sharp_quality = compute_quality(&sharp_frame, metric);
        let blurry_quality = compute_quality(&blurry_frame, metric);

        assert!(
            sharp_quality > blurry_quality,
            "{:?}: sharp ({}) should have higher quality than blurry ({})",
            metric,
            sharp_quality,
            blurry_quality
        );
    }
}

#[test]
fn test_quality_scores_ordering() {
    let frames: Vec<(Frame, f32)> = (0..5)
        .map(|i| {
            let blur = i as f32 * 2.0;
            (create_planetary_frame(64, 64, 0.0, 0.0, blur), blur)
        })
        .collect();

    let qualities: Vec<f32> = frames
        .iter()
        .map(|(f, _)| compute_quality(f, QualityMetric::Laplacian))
        .collect();

    for i in 1..qualities.len() {
        assert!(
            qualities[i] <= qualities[i - 1],
            "Quality should decrease with blur: {} vs {}",
            qualities[i - 1],
            qualities[i]
        );
    }
}

/// A single vertical step edge, built with `set_pixel` so the fixture cannot encode a
/// layout assumption.
///
/// The bound is pinned rather than left at `> 0.0`: the previous fixture wrote
/// `(y * 32 + x) * 3` into a planar frame, which reshuffles the edge into a different
/// image entirely, and `> 0.0` accepted that just as happily as the intended one.
#[test]
fn test_laplacian_variance_calculation() {
    let mut frame = Frame::zeros(32, 32, 3).unwrap();
    for y in 0..32 {
        for x in 0..32 {
            let value = if x < 16 { 0.2 } else { 0.8 };
            for c in 0..3 {
                frame.set_pixel(x, y, c, value);
            }
        }
    }
    let laplacian = compute_quality(&frame, QualityMetric::Laplacian);

    // One column of the 30 interior columns carries a +-0.6 Laplacian response either
    // side of the step; everything else is flat. Mean is 0, so the variance is
    // 2 * 0.6^2 * 30 / (30 * 30) = 0.024.
    assert!(
        (laplacian - 0.024).abs() < 1e-3,
        "Laplacian variance {laplacian} does not match the step edge (expected ~0.024)"
    );
}

/// A constant-slope diagonal ramp, again via `set_pixel`.
///
/// On a ramp of 1/64 per pixel in each axis the Sobel response is uniform, so the mean
/// magnitude is exactly `sqrt(2) * 8 / 64`. Pinning it means a fixture that quietly
/// stops being a ramp fails here instead of passing a `> 0.0` check.
#[test]
fn test_sobel_gradient_calculation() {
    let mut frame = Frame::zeros(32, 32, 3).unwrap();
    for y in 0..32 {
        for x in 0..32 {
            let value = (x + y) as f32 / 64.0;
            for c in 0..3 {
                frame.set_pixel(x, y, c, value);
            }
        }
    }
    let sobel = compute_quality(&frame, QualityMetric::Sobel);

    let expected = (2.0f32).sqrt() * 8.0 / 64.0;
    assert!(
        (sobel - expected).abs() < 1e-3,
        "Sobel magnitude {sobel} does not match the uniform ramp (expected ~{expected})"
    );
}

#[test]
fn test_uniform_frame_low_quality() {
    let uniform = Frame::filled(32, 32, 3, 0.5).unwrap();
    let laplacian = compute_quality(&uniform, QualityMetric::Laplacian);
    let sobel = compute_quality(&uniform, QualityMetric::Sobel);
    let tenengrad = compute_quality(&uniform, QualityMetric::Tenengrad);

    assert!(laplacian < 0.001);
    assert!(sobel < 0.001);
    assert!(tenengrad < 0.001);
}

#[test]
fn test_mono_frame_quality() {
    let data: Vec<f32> = (0..32 * 32)
        .map(|i| {
            let x = i % 32;
            let y = i / 32;
            ((x + y) as f32 / 64.0).sin().abs()
        })
        .collect();

    let frame = Frame::from_f32_vec(data, 32, 32, 1).unwrap();
    let quality = compute_quality(&frame, QualityMetric::Laplacian);
    assert!(quality > 0.0);
}
