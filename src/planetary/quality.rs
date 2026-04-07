//! Quality metrics for frame selection in planetary stacking.

use crate::frame::Frame;

use super::QualityMetric;

/// Computes frame quality using the specified metric
pub fn compute_quality(frame: &Frame, metric: QualityMetric) -> f32 {
    let lum = frame_to_luminance(frame);
    let width = frame.width();
    let height = frame.height();

    match metric {
        QualityMetric::Laplacian => compute_laplacian_variance(&lum, width, height),
        QualityMetric::Sobel => compute_sobel_magnitude(&lum, width, height),
        QualityMetric::Tenengrad => compute_tenengrad(&lum, width, height),
        QualityMetric::StdDev => compute_std_dev(&lum),
    }
}

/// Converts frame to luminance values
pub(crate) fn frame_to_luminance(frame: &Frame) -> Vec<f32> {
    let data = frame.data();
    let channels = frame.channels();
    let pixels = frame.width() * frame.height();

    if channels == 1 {
        return data.to_vec();
    }

    let mut lum = Vec::with_capacity(pixels);
    for i in 0..pixels {
        let r = data[i * channels];
        let g = data[i * channels + 1];
        let b = data[i * channels + 2];
        lum.push(0.2126 * r + 0.7152 * g + 0.0722 * b);
    }
    lum
}

/// Computes Laplacian variance (sharpness metric)
fn compute_laplacian_variance(lum: &[f32], width: usize, height: usize) -> f32 {
    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut count = 0usize;

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let idx = y * width + x;
            let laplacian =
                lum[idx - width] + lum[idx + width] + lum[idx - 1] + lum[idx + 1] - 4.0 * lum[idx];

            sum += laplacian as f64;
            sum_sq += (laplacian * laplacian) as f64;
            count += 1;
        }
    }

    if count == 0 {
        return 0.0;
    }

    let mean = sum / count as f64;
    let variance = sum_sq / count as f64 - mean * mean;
    variance.max(0.0) as f32
}

/// Computes Sobel gradient magnitude (edge strength)
fn compute_sobel_magnitude(lum: &[f32], width: usize, height: usize) -> f32 {
    let mut sum = 0.0f64;
    let mut count = 0usize;

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let idx = y * width + x;

            // Sobel X: [-1, 0, 1; -2, 0, 2; -1, 0, 1]
            let gx = -lum[idx - width - 1] + lum[idx - width + 1] - 2.0 * lum[idx - 1]
                + 2.0 * lum[idx + 1]
                - lum[idx + width - 1]
                + lum[idx + width + 1];

            // Sobel Y: [-1, -2, -1; 0, 0, 0; 1, 2, 1]
            let gy = -lum[idx - width - 1] - 2.0 * lum[idx - width] - lum[idx - width + 1]
                + lum[idx + width - 1]
                + 2.0 * lum[idx + width]
                + lum[idx + width + 1];

            let magnitude = (gx * gx + gy * gy).sqrt();
            sum += magnitude as f64;
            count += 1;
        }
    }

    if count == 0 {
        return 0.0;
    }

    (sum / count as f64) as f32
}

/// Computes Tenengrad (squared Sobel gradient)
fn compute_tenengrad(lum: &[f32], width: usize, height: usize) -> f32 {
    let mut sum = 0.0f64;
    let mut count = 0usize;

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let idx = y * width + x;

            let gx = -lum[idx - width - 1] + lum[idx - width + 1] - 2.0 * lum[idx - 1]
                + 2.0 * lum[idx + 1]
                - lum[idx + width - 1]
                + lum[idx + width + 1];

            let gy = -lum[idx - width - 1] - 2.0 * lum[idx - width] - lum[idx - width + 1]
                + lum[idx + width - 1]
                + 2.0 * lum[idx + width]
                + lum[idx + width + 1];

            sum += (gx * gx + gy * gy) as f64;
            count += 1;
        }
    }

    if count == 0 {
        return 0.0;
    }

    (sum / count as f64) as f32
}

/// Computes standard deviation (contrast metric)
pub(crate) fn compute_std_dev(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }

    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|&v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
    variance.sqrt()
}
