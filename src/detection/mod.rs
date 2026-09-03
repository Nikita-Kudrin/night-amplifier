//! Star Detection Module with Sub-Pixel Centroiding. Stages: background estimation
//! (median, noise level) -> threshold detection -> local maxima -> hot-pixel rejection
//! -> centroiding (Center of Mass). Hot pixels are 1-pixel defects with no surrounding
//! gradient, unlike a star's PSF spread — rejected when the peak pixel holds >90% of
//! the aperture's total flux.
//!
//! Centroid: `x = Σ(x·I(x,y)) / Σ(I(x,y))`, same for y, with I the background-
//! subtracted intensity — typically ~0.1px accuracy for well-exposed stars.

mod adaptive;
mod background;
mod config;
mod detector;
pub mod luminance;
mod star;

pub use adaptive::{detect_stars_adaptive, detect_stars_adaptive_thorough};
pub use background::BackgroundStats;
pub use config::DetectionConfig;
pub use detector::StarDetector;
pub use luminance::{luminance_frame, mean_luminance};
pub use star::Star;

use crate::error::Result;
use crate::frame::Frame;

/// Convenience function to detect stars with default settings
pub fn detect_stars(frame: &Frame) -> Result<Vec<Star>> {
    StarDetector::with_defaults().detect(frame)
}

/// Convenience function to detect stars with custom sigma threshold
pub fn detect_stars_sigma(frame: &Frame, sigma: f32) -> Result<Vec<Star>> {
    StarDetector::new(DetectionConfig::default().with_sigma(sigma)).detect(frame)
}

/// Computes the median FWHM from a list of detected stars.
///
/// Returns None if there are no stars with FWHM data.
pub fn compute_median_fwhm(stars: &[Star]) -> Option<f32> {
    let mut fwhms: Vec<f32> = stars.iter().filter_map(|s| s.fwhm).collect();
    if fwhms.is_empty() {
        return None;
    }
    fwhms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = fwhms.len() / 2;
    if fwhms.len().is_multiple_of(2) {
        Some((fwhms[mid - 1] + fwhms[mid]) / 2.0)
    } else {
        Some(fwhms[mid])
    }
}

/// Computes the median SNR from a list of detected stars.
///
/// Returns None if there are no stars.
pub fn compute_median_snr(stars: &[Star]) -> Option<f32> {
    if stars.is_empty() {
        return None;
    }
    let mut snrs: Vec<f32> = stars.iter().map(|s| s.snr).collect();
    snrs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = snrs.len() / 2;
    if snrs.len().is_multiple_of(2) {
        Some((snrs[mid - 1] + snrs[mid]) / 2.0)
    } else {
        Some(snrs[mid])
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn create_test_frame_with_stars() -> Frame {
        let width = 100;
        let height = 100;
        let mut data = vec![0.1f32; width * height];

        add_gaussian_star(&mut data, width, 30.0, 30.0, 0.8, 3.0);
        add_gaussian_star(&mut data, width, 70.0, 50.0, 0.4, 2.5);
        data[50 * width + 50] = 0.9; // Hot pixel

        Frame::from_f32_vec(data, width, height, 1).unwrap()
    }

    pub fn add_gaussian_star(
        data: &mut [f32],
        width: usize,
        cx: f32,
        cy: f32,
        peak: f32,
        sigma: f32,
    ) {
        let radius = (sigma * 4.0) as isize;
        let cx_int = cx.round() as isize;
        let cy_int = cy.round() as isize;
        let height = data.len() / width;

        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let x = cx_int + dx;
                let y = cy_int + dy;

                if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                    let dist_x = x as f32 - cx;
                    let dist_y = y as f32 - cy;
                    let dist_sq = dist_x * dist_x + dist_y * dist_y;
                    let intensity = peak * (-dist_sq / (2.0 * sigma * sigma)).exp();
                    let idx = y as usize * width + x as usize;
                    data[idx] += intensity;
                }
            }
        }
    }
}
