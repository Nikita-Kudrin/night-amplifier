use crate::error::Result;
use crate::frame::Frame;
use rayon::prelude::*;
use std::cmp::Ordering;
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
            crate::detection::luminance::mean_luminance(frame)
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
            let _span =
                tracing::info_span!("compute_centroids", candidates = candidates.len()).entered();
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

        // FWHM from the area above half maximum. `None` means "not measurable in
        // this window" — the star is left without a FWHM rather than carrying a
        // made-up one, because downstream weighting treats a wrong number far
        // worse than a missing one.
        //
        // The threshold is derived from this star's OWN pixel, not the `peak_value`
        // accumulated above: that value is the brightest raw pixel anywhere in the
        // whole `centroid_radius` window, which a brighter neighbour can dominate
        // even though it sits outside `search_radius` and never disqualifies this
        // star's own peak from `is_local_maximum`. Using the window-wide max here
        // would raise this star's half-maximum threshold above its own true half
        // level, poisoning the flood fill and making a perfectly measurable star
        // come back `None` — the same failure mode this fix exists to prevent, just
        // reached through the threshold instead of the area count.
        let own_peak_value = data[peak_y * width + peak_x];
        match self.compute_fwhm(data, width, height, peak_x, peak_y, own_peak_value) {
            Some(fwhm) => Some(Star::with_fwhm(
                centroid_x,
                centroid_y,
                sum_flux as f32,
                peak_value,
                snr,
                fwhm,
            )),
            None => Some(Star::new(
                centroid_x,
                centroid_y,
                sum_flux as f32,
                peak_value,
                snr,
            )),
        }
    }

    /// Computes FWHM from the area of the star above half its peak intensity.
    ///
    /// For a Gaussian the half-maximum contour is a circle of radius `σ·√(2ln2)`
    /// enclosing `A = 2π·ln2·σ²` pixels, so `FWHM = 2σ√(2ln2) = 2√(A/π)` — exact in
    /// the continuum, and needing only a pixel count to evaluate.
    ///
    /// This replaced a second moment taken over the whole centroid window. That
    /// estimate weights every pixel by `r²·I`, so background noise in the corners
    /// dominates for anything but a bright, tight star and the result saturates at
    /// `2.3548·√(2·Σd²/(2r+1))` — 10.53 px at `centroid_radius = 5`, independent of
    /// the actual star. Real frames pinned against that ceiling (p10 10.19 / p90
    /// 10.56 px on the 250 mm-dob fixture), which silently disabled FWHM-based frame
    /// weighting in `QualityLimits` (every frame scoring identically) and made the
    /// Pro solver's bloat detection fire on every frame.
    ///
    /// Half maximum is measured against a **local** background taken from the window's
    /// outermost ring, not the frame-wide median. Inside nebulosity the two differ
    /// enough that a frame-wide median puts the half level below the surrounding
    /// nebula, so the fill runs to the window edge and the star is discarded — that
    /// alone cost 92% of stars on the 250 mm-dob Orion fixture.
    ///
    /// Returns `None` when the star is not measurable in the available window:
    /// nothing clears half maximum, or the above-half region reaches the window
    /// border so its true extent is unknown. Callers must treat that as missing
    /// data — `compute_median_fwhm` already filters `None` out.
    fn compute_fwhm(
        &self,
        data: &[f32],
        width: usize,
        height: usize,
        peak_x: usize,
        peak_y: usize,
        peak_value: f32,
    ) -> Option<f32> {
        let radius = self.config.centroid_radius as isize;
        let side = (2 * radius + 1) as usize;
        let slot = |dx: isize, dy: isize| ((dy + radius) as usize) * side + (dx + radius) as usize;

        let background = self.local_background(data, width, height, peak_x, peak_y)?;
        let half_level = (peak_value - background) * 0.5;
        if half_level <= 0.0 {
            return None;
        }

        // Flood-fill outward from the peak rather than counting every pixel in the
        // window, so a blended neighbour sharing the window can't inflate the area.
        let mut visited = vec![false; side * side];
        let mut stack = vec![(0isize, 0isize)];
        visited[slot(0, 0)] = true;

        let mut area = 0usize;
        let mut reached_border = false;

        while let Some((dx, dy)) = stack.pop() {
            let nx = peak_x as isize + dx;
            let ny = peak_y as isize + dy;

            if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
                reached_border = true;
                continue;
            }
            if data[ny as usize * width + nx as usize] - background < half_level {
                continue;
            }

            area += 1;

            if dx.abs() == radius || dy.abs() == radius {
                // Still above half maximum at the edge of the window — the star is
                // wider than we can see, so any number we produced would be a floor,
                // not a measurement.
                reached_border = true;
                continue;
            }

            for (sx, sy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let next = slot(dx + sx, dy + sy);
                if !visited[next] {
                    visited[next] = true;
                    stack.push((dx + sx, dy + sy));
                }
            }
        }

        if area == 0 || reached_border {
            return None;
        }

        Some(2.0 * (area as f32 / std::f32::consts::PI).sqrt())
    }

    /// Median of a square annulus at twice the centroid radius — the sky level
    /// immediately around this star.
    ///
    /// Sampled at `2 × centroid_radius` rather than at the edge of the measurement
    /// window itself: a star's own wings are still ~25% of peak at `centroid_radius`
    /// for a FWHM-7 px star, which inflates the background, deflates the measured
    /// area and reads back ~15% narrow. Two radii out the same star contributes
    /// under 0.5%. The median keeps a neighbouring star that happens to land on the
    /// annulus from dragging it.
    ///
    /// Returns `None` only if the annulus lies entirely outside the frame.
    fn local_background(
        &self,
        data: &[f32],
        width: usize,
        height: usize,
        peak_x: usize,
        peak_y: usize,
    ) -> Option<f32> {
        let radius = self.config.centroid_radius as isize * 2;
        let mut ring = Vec::with_capacity((8 * radius) as usize);

        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let nx = peak_x as isize + dx;
                let ny = peak_y as isize + dy;
                if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
                    continue;
                }
                ring.push(data[ny as usize * width + nx as usize]);
            }
        }

        if ring.is_empty() {
            return None;
        }

        let mid = ring.len() / 2;
        ring.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        Some(ring[mid])
    }
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
    fn test_sub_pixel_centroid_shift() {
        let width = 50;
        let height = 50;
        let mut data = vec![0.05f32; width * height];

        // Generate a star centered exactly at 25.3, 25.0
        add_gaussian_star(&mut data, width, 25.3, 25.0, 0.8, 2.0);

        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();
        let config = DetectionConfig::default().with_sigma(3.0).unlimited_stars();

        let detector = StarDetector::new(config);
        let stars = detector.detect(&frame).unwrap();

        assert!(!stars.is_empty(), "Should detect the star");
        let star = &stars[0];

        // Ensure the centroid is detected at the sub-pixel coordinate correctly
        assert!(
            (star.x - 25.3).abs() < 0.1,
            "Centroid x should be ~25.3, got x={}",
            star.x
        );
        assert!(
            (star.y - 25.0).abs() < 0.1,
            "Centroid y should be ~25.0, got y={}",
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
                        let idx = c * (width * height) + (y as usize) * width + (x as usize);
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

    /// Renders one isolated Gaussian and returns the detected star.
    fn detect_single_gaussian(sigma: f32, peak: f32) -> Star {
        let (width, height) = (100usize, 100usize);
        let mut data = vec![0.05f32; width * height];
        add_gaussian_star(&mut data, width, 50.0, 50.0, peak, sigma);
        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();

        let stars = StarDetector::with_defaults().detect(&frame).unwrap();
        assert_eq!(
            stars.len(),
            1,
            "expected exactly one star for sigma={sigma}"
        );
        stars[0]
    }

    #[test]
    fn fwhm_recovers_gaussian_width() {
        // FWHM = 2·sqrt(2·ln2)·sigma. Pixel-counting the half-maximum area carries a
        // small discretisation bias that shrinks as the star grows; 10% covers it.
        for sigma in [1.0f32, 2.0, 3.0] {
            let expected = 2.354_82 * sigma;
            let measured = detect_single_gaussian(sigma, 0.6)
                .fwhm
                .unwrap_or_else(|| panic!("sigma={sigma} should be measurable"));
            let error = (measured - expected).abs() / expected;
            assert!(
                error < 0.10,
                "sigma={sigma}: expected FWHM ~{expected:.2}, got {measured:.2} ({:.1}% off)",
                error * 100.0
            );
        }
    }

    #[test]
    fn fwhm_is_none_when_star_exceeds_measurement_window() {
        // centroid_radius=5 gives an 11×11 window; a sigma=6 star is still above half
        // maximum at its border, so its width is unknowable here. Reporting a floor
        // value would be worse than reporting nothing.
        let star = detect_single_gaussian(6.0, 0.6);
        assert_eq!(star.fwhm, None);
    }

    #[test]
    fn fwhm_is_not_pinned_to_the_window_ceiling() {
        // Regression: the old second-moment estimate saturated at
        // 2.3548·sqrt(2·Σd²/(2r+1)) = 10.53 px for centroid_radius=5, so every star in
        // a real frame reported ~10.4 px regardless of its actual size. That killed
        // FWHM-based frame weighting (all frames scoring identically) and made the Pro
        // solver bin every frame. A field of genuinely different stars must spread.
        let (width, height) = (200usize, 200usize);
        let mut data = vec![0.05f32; width * height];
        for (i, sigma) in [1.0f32, 1.5, 2.0, 2.5, 3.0].iter().enumerate() {
            let x = 40.0 + (i as f32) * 30.0;
            add_gaussian_star(&mut data, width, x, 100.0, 0.6, *sigma);
        }
        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();

        let stars = StarDetector::with_defaults().detect(&frame).unwrap();
        let mut fwhms: Vec<f32> = stars.iter().filter_map(|s| s.fwhm).collect();
        assert_eq!(fwhms.len(), 5, "all five stars should be measurable");
        fwhms.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let spread = fwhms[fwhms.len() - 1] - fwhms[0];
        assert!(
            spread > 1.5,
            "FWHM must track star size, got spread {spread:.2} over {fwhms:?}"
        );
    }

    #[test]
    fn fwhm_excludes_a_blended_neighbour() {
        // A second star 4 px away sits inside the 11×11 window and clears the primary's
        // half-maximum level, but its above-half region is disconnected from the
        // primary's. Counting the whole window would add ~5 px of neighbour to ~5 px of
        // star and inflate FWHM by ~40%; the flood fill from the peak must ignore it.
        // (Blending does drag the *centroid* between the two peaks — that is the
        // centroid's own behaviour and not what this test is about.)
        let isolated = detect_single_gaussian(1.0, 0.60)
            .fwhm
            .expect("isolated control should be measurable");

        let (width, height) = (100usize, 100usize);
        let mut data = vec![0.05f32; width * height];
        add_gaussian_star(&mut data, width, 50.0, 50.0, 0.60, 1.0);
        add_gaussian_star(&mut data, width, 54.0, 50.0, 0.50, 1.0);
        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();

        let stars = StarDetector::with_defaults().detect(&frame).unwrap();
        assert_eq!(stars.len(), 2, "both peaks should be detected");

        for star in &stars {
            let fwhm = star.fwhm.expect("blended star should still be measurable");
            assert!(
                (fwhm - isolated).abs() < 0.5,
                "neighbour leaked in: got {fwhm:.2}, isolated control is {isolated:.2}"
            );
        }
    }

    #[test]
    fn fwhm_is_not_poisoned_by_a_brighter_neighbour_outside_search_radius() {
        // Regression: `compute_fwhm`'s half-maximum threshold used to be derived
        // from `peak_value`, the brightest raw pixel anywhere in the whole
        // `centroid_radius` window (5 px by default) — a strictly larger radius
        // than `search_radius` (3 px) used to decide local-maximum candidacy. A
        // brighter neighbour 5 px away never disqualifies this star's own peak from
        // `is_local_maximum` (it sits outside `search_radius`), but it used to leak
        // into this star's FWHM threshold anyway via the window-wide max, raising
        // the threshold above the dim star's own true half-maximum level and making
        // it come back `None` even though it is perfectly measurable on its own.
        //
        // The "neighbour" here is a single bare pixel, not a second Gaussian star:
        // that isolates the threshold-contamination bug from ordinary flux blending
        // (already covered by `fwhm_excludes_a_blended_neighbour` above) and from
        // `add_gaussian_star`'s additive overlap dragging the centroid. The pixel
        // still passes its own candidacy (a real second `Star`, dominated by its own
        // spike), so both are examined; the star under test is identified by total
        // flux, since the dim star's spread-out Gaussian body carries more of it
        // than the lone pixel's single-point spike ever can, regardless of which one
        // has the higher raw peak value.
        let isolated = detect_single_gaussian(1.5, 0.30)
            .fwhm
            .expect("isolated control should be measurable");

        let (width, height) = (100usize, 100usize);
        let mut data = vec![0.05f32; width * height];
        add_gaussian_star(&mut data, width, 50.0, 50.0, 0.30, 1.5); // dim star under test
        data[50 * width + 55] = 0.8; // bright lone pixel, exactly `centroid_radius` away

        let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();
        let stars = StarDetector::with_defaults().detect(&frame).unwrap();
        assert_eq!(
            stars.len(),
            2,
            "both the dim star and the lone pixel should register"
        );

        let dim = stars
            .iter()
            .max_by(|a, b| a.flux.partial_cmp(&b.flux).unwrap())
            .expect("two stars were detected");

        let fwhm = dim.fwhm.expect(
            "the dim star must remain measurable even though a brighter pixel \
             shares its centroid_radius window",
        );
        assert!(
            (fwhm - isolated).abs() < 0.5,
            "neighbour's brightness leaked into the threshold: got {fwhm:.2}, isolated control is {isolated:.2}"
        );
    }
}
