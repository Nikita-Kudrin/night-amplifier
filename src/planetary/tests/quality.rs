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

#[test]
fn test_laplacian_variance_calculation() {
    let mut data = vec![0.0f32; 32 * 32 * 3];
    for y in 0..32 {
        for x in 0..32 {
            let idx = (y * 32 + x) * 3;
            let value = if x < 16 { 0.2 } else { 0.8 };
            data[idx] = value;
            data[idx + 1] = value;
            data[idx + 2] = value;
        }
    }
    let frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();
    let laplacian = compute_quality(&frame, QualityMetric::Laplacian);
    assert!(laplacian > 0.0, "Laplacian should detect edge");
}

#[test]
fn test_sobel_gradient_calculation() {
    let mut data = vec![0.0f32; 32 * 32 * 3];
    for y in 0..32 {
        for x in 0..32 {
            let idx = (y * 32 + x) * 3;
            let value = (x + y) as f32 / 64.0;
            data[idx] = value;
            data[idx + 1] = value;
            data[idx + 2] = value;
        }
    }
    let frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();
    let sobel = compute_quality(&frame, QualityMetric::Sobel);
    assert!(sobel > 0.0, "Sobel should detect gradient");
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
