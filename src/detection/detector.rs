use crate::error::Result;
use crate::frame::Frame;
use rayon::prelude::*;
use tracing::{instrument, Span};

use super::background::BackgroundStats;
use super::config::DetectionConfig;
use super::star::Star;

/// Star detector for finding and measuring stars in frames
pub struct StarDetector {
    config: DetectionConfig,
}

impl StarDetector {
    pub fn new(config: DetectionConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(DetectionConfig::default())
    }

    /// Detects stars in a frame
    ///
    /// For multi-channel images, detection is performed on a luminance
    /// channel computed as the average of all channels.
    ///
    /// # Returns
    /// A vector of detected stars, sorted by flux (brightest first)
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        channels = frame.channels(),
        sigma_threshold = self.config.sigma_threshold,
        max_stars = ?self.config.max_stars
    ))]
    pub fn detect(&self, frame: &Frame) -> Result<Vec<Star>> {
        let luminance = {
            let _span = tracing::info_span!("compute_luminance").entered();
            compute_luminance(frame)
        };
        let width = frame.width();
        let height = frame.height();

        let stats = {
            let _span = tracing::info_span!("background_stats").entered();
            BackgroundStats::estimate(&luminance, self.config.sigma_threshold)
        };
        let candidates = {
            let _span = tracing::info_span!("find_local_maxima").entered();
            self.find_local_maxima(&luminance, width, height, &stats)
        };

        let mut stars: Vec<Star> = {
            let _span = tracing::info_span!("compute_centroids", candidates = candidates.len()).entered();
            candidates
                .into_iter()
                .filter_map(|(x, y)| self.compute_centroid(&luminance, width, height, x, y, &stats))
                .collect()
        };

        {
            let _span = tracing::info_span!("sort_stars").entered();
            stars.sort_by(|a, b| {
                b.flux
                    .partial_cmp(&a.flux)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        if let Some(max) = self.config.max_stars {
            stars.truncate(max);
        }

        Span::current().record("stars_detected", stars.len());

        Ok(stars)
    }

    pub fn config(&self) -> &DetectionConfig {
        &self.config
    }

    fn find_local_maxima(
        &self,
        data: &[f32],
        width: usize,
        height: usize,
        stats: &BackgroundStats,
    ) -> Vec<(usize, usize)> {
        let margin = self.config.border_margin;
        let radius = self.config.search_radius as isize;

        let y_range: Vec<usize> = (margin..height.saturating_sub(margin)).collect();

        y_range
            .par_iter()
            .flat_map(|&y| {
                (margin..width.saturating_sub(margin))
                    .filter_map(|x| {
                        let idx = y * width + x;
                        let value = data[idx];

                        if value < stats.threshold {
                            return None;
                        }

                        if self.is_local_maximum(data, width, height, x, y, value, radius) {
                            Some((x, y))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn is_local_maximum(
        &self,
        data: &[f32],
        width: usize,
        height: usize,
        x: usize,
        y: usize,
        value: f32,
        radius: isize,
    ) -> bool {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx == 0 && dy == 0 {
                    continue;
                }

                let nx = x as isize + dx;
                let ny = y as isize + dy;

                if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
                    continue;
                }

                let nidx = ny as usize * width + nx as usize;
                let neighbor = data[nidx];

                if neighbor > value {
                    return false;
                }

                // Handle ties: prefer earlier scan order
                if neighbor == value && (ny as usize > y || (ny as usize == y && nx as usize > x)) {
                    return false;
                }
            }
        }
        true
    }

    fn compute_centroid(
        &self,
        data: &[f32],
        width: usize,
        height: usize,
        peak_x: usize,
        peak_y: usize,
        stats: &BackgroundStats,
    ) -> Option<Star> {
        let radius = self.config.centroid_radius as isize;
        let background = stats.median;

        let mut sum_x = 0.0f64;
        let mut sum_y = 0.0f64;
        let mut sum_flux = 0.0f64;
        let mut peak_value = 0.0f32;
        let mut pixels_above_threshold = 0usize;

        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let nx = peak_x as isize + dx;
                let ny = peak_y as isize + dy;

                if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
                    continue;
                }

                let idx = ny as usize * width + nx as usize;
                let raw_value = data[idx];
                let value = (raw_value - background).max(0.0);

                if value > 0.0 {
                    sum_x += nx as f64 * value as f64;
                    sum_y += ny as f64 * value as f64;
                    sum_flux += value as f64;

                    if raw_value > peak_value {
                        peak_value = raw_value;
                    }

                    if raw_value > stats.threshold * 0.5 {
                        pixels_above_threshold += 1;
                    }
                }
            }
        }

        if sum_flux < 1e-6 {
            return None;
        }

        // Hot pixel rejection
        let peak_flux = (peak_value - background).max(0.0) as f64;
        if peak_flux / sum_flux > self.config.hot_pixel_threshold as f64 {
            return None;
        }

        if pixels_above_threshold < self.config.min_star_pixels {
            return None;
        }

        let centroid_x = (sum_x / sum_flux) as f32;
        let centroid_y = (sum_y / sum_flux) as f32;

        let dist_from_peak =
            ((centroid_x - peak_x as f32).powi(2) + (centroid_y - peak_y as f32).powi(2)).sqrt();

        if dist_from_peak > radius as f32 {
            return None;
        }

        let n_pixels = ((2 * radius + 1) * (2 * radius + 1)) as f64;
        let background_noise = stats.sigma as f64 * n_pixels.sqrt();
        let snr = if background_noise > 1e-10 {
            (sum_flux / background_noise) as f32
        } else {
            (sum_flux.sqrt()) as f32
        };

        if snr < self.config.min_snr {
            return None;
        }

        // Compute FWHM from second moment (variance of intensity distribution)
        // FWHM = 2 * sqrt(2 * ln(2)) * sigma ≈ 2.355 * sigma
        let fwhm = self.compute_fwhm(data, width, height, centroid_x, centroid_y, background);

        Some(Star::with_fwhm(
            centroid_x,
            centroid_y,
            sum_flux as f32,
            peak_value,
            snr,
            fwhm,
        ))
    }

    /// Computes FWHM from the second moment of the intensity distribution.
    ///
    /// Uses the relationship: FWHM = 2 * sqrt(2 * ln(2)) * sigma ≈ 2.355 * sigma
    /// where sigma is the standard deviation of the Gaussian profile.
    fn compute_fwhm(
        &self,
        data: &[f32],
        width: usize,
        height: usize,
        centroid_x: f32,
        centroid_y: f32,
        background: f32,
    ) -> f32 {
        let radius = self.config.centroid_radius as isize;
        let cx = centroid_x as f64;
        let cy = centroid_y as f64;

        let mut sum_r2 = 0.0f64; // Weighted sum of squared radii
        let mut sum_w = 0.0f64; // Sum of weights (intensities)

        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let nx = centroid_x.round() as isize + dx;
                let ny = centroid_y.round() as isize + dy;

                if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
                    continue;
                }

                let idx = ny as usize * width + nx as usize;
                let value = (data[idx] - background).max(0.0) as f64;

                if value > 0.0 {
                    let r2 = (nx as f64 - cx).powi(2) + (ny as f64 - cy).powi(2);
                    sum_r2 += r2 * value;
                    sum_w += value;
                }
            }
        }

        if sum_w < 1e-10 {
            return 2.0; // Default FWHM if no valid data
        }

        // Variance (second moment) = sum(r² * I) / sum(I)
        let variance = sum_r2 / sum_w;
        let sigma = variance.sqrt();

        // FWHM = 2 * sqrt(2 * ln(2)) * sigma ≈ 2.355 * sigma
        const FWHM_FACTOR: f64 = 2.354_820_045_030_949_4; // 2 * sqrt(2 * ln(2))
        let fwhm = (FWHM_FACTOR * sigma) as f32;

        // Clamp to reasonable range (1.0 to 20.0 pixels)
        fwhm.clamp(1.0, 20.0)
    }
}

fn compute_luminance(frame: &Frame) -> Vec<f32> {
    let channels = frame.channels();
    let pixel_count = frame.pixel_count();
    let data = frame.data();

    if channels == 1 {
        return data.to_vec();
    }

    let inv_channels = 1.0 / channels as f32;
    (0..pixel_count)
        .map(|i| {
            let base = i * channels;
            data[base..base + channels].iter().sum::<f32>() * inv_channels
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detection::tests::*;

    #[test]
    fn test_detect_synthetic_stars() {
        let frame = create_test_frame_with_stars();
        let config = DetectionConfig::default()
            .with_sigma(2.0)
            .with_max_stars(10)
            .with_min_snr(3.0);

        let detector = StarDetector::new(config);
        let stars = detector.detect(&frame).unwrap();

        assert!(
            stars.len() >= 2,
            "Expected at least 2 stars, got {}",
            stars.len()
        );

        let brightest = &stars[0];
        assert!(
            (brightest.x - 30.0).abs() < 1.0,
            "Expected x~30, got {}",
            brightest.x
        );
        assert!(
            (brightest.y - 30.0).abs() < 1.0,
            "Expected y~30, got {}",
            brightest.y
        );
    }

    #[test]
    fn test_hot_pixel_rejection() {
        let width = 50;
        let height = 50;
        let mut data = vec![0.1f32; width * height];
        data[25 * width + 25] = 0.95;

        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();
        let config = DetectionConfig::default().with_sigma(3.0).unlimited_stars();

        let detector = StarDetector::new(config);
        let stars = detector.detect(&frame).unwrap();

        let hot_pixel_detected = stars
            .iter()
            .any(|s| (s.x - 25.0).abs() < 2.0 && (s.y - 25.0).abs() < 2.0);

        assert!(!hot_pixel_detected, "Hot pixel should have been rejected");
    }

    #[test]
    fn test_centroid_accuracy() {
        let width = 50;
        let height = 50;
        let mut data = vec![0.05f32; width * height];
        add_gaussian_star(&mut data, width, 25.3, 25.7, 0.7, 2.5);

        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();
        let config = DetectionConfig::default().with_sigma(3.0).unlimited_stars();

        let detector = StarDetector::new(config);
        let stars = detector.detect(&frame).unwrap();

        assert!(!stars.is_empty(), "Should detect the star");

        let star = &stars[0];
        assert!(
            (star.x - 25.3).abs() < 0.3,
            "X centroid error: expected 25.3, got {}",
            star.x
        );
        assert!(
            (star.y - 25.7).abs() < 0.3,
            "Y centroid error: expected 25.7, got {}",
            star.y
        );
    }

    #[test]
    fn test_multichannel_detection() {
        let width = 80;
        let height = 80;
        let channels = 3;
        let mut data = vec![0.05f32; width * height * channels];

        let sigma = 3.0f32;
        let peak = 0.8f32;
        let radius = (sigma * 4.0) as isize;
        for c in 0..channels {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let x = 40 + dx;
                    let y = 40 + dy;
                    if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                        let dist_sq = (dx * dx + dy * dy) as f32;
                        let intensity = peak * (-dist_sq / (2.0 * sigma * sigma)).exp();
                        let idx = (y as usize * width + x as usize) * channels + c;
                        data[idx] += intensity;
                    }
                }
            }
        }

        let frame = Frame::from_f32_vec(data, width, height, channels).unwrap();
        let stars = StarDetector::with_defaults().detect(&frame).unwrap();

        assert!(!stars.is_empty(), "Should detect star in RGB image");
        assert!((stars[0].x - 40.0).abs() < 1.0);
        assert!((stars[0].y - 40.0).abs() < 1.0);
    }

    #[test]
    fn test_stars_sorted_by_brightness() {
        let frame = create_test_frame_with_stars();
        let stars = StarDetector::with_defaults().detect(&frame).unwrap();

        for i in 1..stars.len() {
            assert!(
                stars[i - 1].flux >= stars[i].flux,
                "Stars should be sorted by flux (descending)"
            );
        }
    }

    #[test]
    fn test_max_stars_limit() {
        let width = 100;
        let height = 100;
        let mut data = vec![0.05f32; width * height];

        for i in 0..20 {
            let x = 15.0 + (i % 5) as f32 * 15.0;
            let y = 15.0 + (i / 5) as f32 * 15.0;
            add_gaussian_star(&mut data, width, x, y, 0.3 + (i as f32 * 0.02), 2.0);
        }

        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();
        let config = DetectionConfig::default().with_sigma(2.0).with_max_stars(5);

        let detector = StarDetector::new(config);
        let stars = detector.detect(&frame).unwrap();

        assert!(stars.len() <= 5, "Should respect max_stars limit");
    }

    #[test]
    fn test_empty_frame() {
        let data = vec![0.1f32; 10000];
        let frame = Frame::from_f32_vec(data, 100, 100, 1).unwrap();

        let stars = StarDetector::with_defaults().detect(&frame).unwrap();
        assert!(stars.is_empty(), "Uniform frame should have no stars");
    }
}
