//! Hot-pixel rejection on the raw mosaic
//!
//! The IMX533 fixture carries 5 189 pixels persistently above 20 sigma and
//! 2 191 above 50. Stacking cannot touch them — they are in the same place in
//! every sub — and debayering spreads each one into a coloured 3x3 cross, which
//! is what makes them read as red and blue dots rather than as white specks. So
//! the filter has to run here, on the mosaic, before demosaic.
//!
//! # Why the obvious test eats stars
//!
//! `|centre - median(3x3)| > tau` fires on every star core: at 0.62 arcsec per
//! pixel a tight star legitimately sits far more than 5 sigma above its own
//! neighbours, and on one colour site the sampling is halved again. Two
//! corrections make the test safe:
//!
//! - **One-sided.** Only a sample *brighter* than its neighbours is a candidate.
//!   A dark defect is a different problem with a different fix (a master dark).
//! - **Isolation-gated, multiplicatively.** A hot pixel is a single-sample
//!   defect: its same-colour neighbours are undisturbed sky. A star is a PSF
//!   several samples wide, so its brightest neighbour carries a large *fraction*
//!   of its own amplitude above the background. Testing that fraction rather
//!   than an absolute difference is what makes the gate independent of how
//!   bright the star is — `centre - max(neighbours) > tau` alone still clips the
//!   core of a bright star, because 38 % of a 200-sigma peak is 76 sigma.
//!
//! # Why max-of-eight rather than a median
//!
//! A branchless median-of-9 is a 19-comparator sorting network. Eight
//! [`f32::max`] operations answer the same question here — the brightest
//! neighbour *is* the second-brightest sample of the 3x3 whenever the centre is
//! the brightest, which is the only case this filter acts on — and vectorize at
//! least as well on NEON for roughly a third of the work.
//!
//! The de-interleave into four planar buffers that usually accompanies CFA work
//! is skipped for the same reason: two full 36 MB copies per frame is real DRAM
//! traffic on a Pi 5, against a pipeline already measured at ~833 MB per frame.
//! Walking row triples `step` apart with stride-`step` reads inside each row
//! touches the same cache lines without the copies.

use rayon::prelude::*;

use crate::error::{Result, StackError};
use crate::statistics::fast_median;

use super::{CfaFrame, CfaStage};

/// Samples drawn from the centre crop to estimate one site's noise level.
const MAX_SIGMA_SAMPLES: usize = 32_768;

/// Scales a MAD into a Gaussian sigma.
const MAD_TO_SIGMA: f32 = 1.4826;

/// Tuning for [`reject_hot_pixels`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HotPixelConfig {
    /// How far above its brightest same-colour neighbour a sample must sit,
    /// in sigmas of that colour site's own noise.
    pub sigma: f32,
    /// Largest share of the centre's amplitude above background that the
    /// brightest neighbour may carry and still count as isolated.
    ///
    /// A Gaussian PSF sampled at one colour site keeps 60 % or more of its peak
    /// one sample out even when it is critically sampled; sky noise beside a hot
    /// pixel keeps a few per cent. 0.35 sits in the gap, and being a ratio it
    /// does not move with star brightness.
    pub isolation: f32,
}

impl Default for HotPixelConfig {
    fn default() -> Self {
        Self {
            sigma: 5.0,
            isolation: 0.35,
        }
    }
}

/// What [`reject_hot_pixels`] did, for logging and for tests to assert on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HotPixelStats {
    /// Samples replaced by their neighbourhood mean.
    pub corrected: usize,
    /// Colour sites whose noise estimate was unusable, so they were left alone.
    pub sites_skipped: usize,
}

/// A registered [`CfaStage`] wrapper around [`reject_hot_pixels`].
#[derive(Debug, Clone, Copy, Default)]
pub struct HotPixelFilter {
    config: HotPixelConfig,
}

impl HotPixelFilter {
    /// Build the stage with explicit tuning.
    pub fn new(config: HotPixelConfig) -> Self {
        Self { config }
    }
}

impl CfaStage for HotPixelFilter {
    fn name(&self) -> &'static str {
        "hot_pixels"
    }

    fn apply(&self, frame: &mut CfaFrame) -> Result<()> {
        let stats = reject_hot_pixels(frame, &self.config)?;
        tracing::debug!(
            corrected = stats.corrected,
            sites_skipped = stats.sites_skipped,
            "Hot pixels rejected"
        );
        Ok(())
    }
}

/// Replace isolated hot samples with the mean of their same-colour neighbours.
///
/// Operates on each colour site independently: an R sample and the B sample
/// beside it sit at different levels, so treating them as neighbours would read
/// the mosaic itself as a defect.
pub fn reject_hot_pixels(cfa: &mut CfaFrame, config: &HotPixelConfig) -> Result<HotPixelStats> {
    let Some(planes) = cfa.planes() else {
        return Err(StackError::ChannelMismatch {
            expected: 1,
            actual: cfa.frame().channels(),
        });
    };

    let (width, height, step) = (planes.width, planes.height, planes.step);
    let mut stats = HotPixelStats::default();

    // Detection reads the frame; the replacements are applied afterwards, so a
    // corrected sample can never feed the test for one of its neighbours and
    // the result does not depend on how rayon split the rows.
    let mut corrections: Vec<(usize, f32)> = Vec::new();
    {
        let data = cfa.frame().data();
        for (x0, y0) in planes.origins() {
            let Some((background, sigma)) = site_background(data, width, height, x0, y0, step)
            else {
                stats.sites_skipped += 1;
                continue;
            };
            let tau = config.sigma * sigma;
            if tau.is_nan() || tau <= 0.0 {
                stats.sites_skipped += 1;
                continue;
            }

            let rows: Vec<usize> = (y0 + step..height.saturating_sub(step))
                .step_by(step)
                .collect();
            let mut hits: Vec<(usize, f32)> = rows
                .into_par_iter()
                .flat_map_iter(|y| {
                    scan_row(data, width, x0, step, y, tau, background, config.isolation)
                })
                .collect();
            corrections.append(&mut hits);
        }
    }

    stats.corrected = corrections.len();
    let data = cfa.frame_mut().data_mut();
    for (idx, value) in corrections {
        data[idx] = value;
    }
    Ok(stats)
}

/// One row of the detection sweep: three rows `step` apart, stride-`step` reads.
#[allow(clippy::too_many_arguments)]
fn scan_row(
    data: &[f32],
    width: usize,
    x0: usize,
    step: usize,
    y: usize,
    tau: f32,
    background: f32,
    isolation: f32,
) -> Vec<(usize, f32)> {
    let up = &data[(y - step) * width..][..width];
    let mid = &data[y * width..][..width];
    let down = &data[(y + step) * width..][..width];

    let mut hits = Vec::new();
    let mut x = x0 + step;
    while x + step < width {
        let centre = mid[x];
        let (nw, n, ne) = (up[x - step], up[x], up[x + step]);
        let (w, e) = (mid[x - step], mid[x + step]);
        let (sw, s, se) = (down[x - step], down[x], down[x + step]);

        let brightest = nw.max(n).max(ne).max(w).max(e).max(sw).max(s).max(se);
        let above_background = centre - background;
        if centre - brightest > tau
            && above_background > 0.0
            && brightest - background < isolation * above_background
        {
            let mean = (nw + n + ne + w + e + sw + s + se) * 0.125;
            hits.push((y * width + x, mean));
        }
        x += step;
    }
    hits
}

/// Robust background and noise level of one colour site, from a centre crop.
///
/// The crop keeps the estimate away from vignetted corners and from the amp
/// glow that lives at a sensor's edge; sub-sampling whole rows keeps it cheap on
/// a 9 MP frame. Returns `None` when the crop holds too few samples to estimate
/// from.
fn site_background(
    data: &[f32],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    step: usize,
) -> Option<(f32, f32)> {
    let first_x = align_to_site(width / 4, x0, step);
    let first_y = align_to_site(height / 4, y0, step);
    let last_x = (width * 3 / 4).min(width);
    let last_y = (height * 3 / 4).min(height);
    if first_x >= last_x || first_y >= last_y {
        return None;
    }

    let cols = (last_x - first_x).div_ceil(step);
    let rows = (last_y - first_y).div_ceil(step);
    let row_stride = (rows * cols / MAX_SIGMA_SAMPLES).max(1);

    let mut samples: Vec<f32> = Vec::with_capacity(rows.div_ceil(row_stride) * cols);
    for y in (first_y..last_y).step_by(step * row_stride) {
        let row = &data[y * width..][..width];
        samples.extend(row[first_x..last_x].iter().step_by(step).copied());
    }
    if samples.len() < 64 {
        return None;
    }

    let median = fast_median(&mut samples);
    for v in samples.iter_mut() {
        *v = (*v - median).abs();
    }
    let mad = fast_median(&mut samples);
    let sigma = mad * MAD_TO_SIGMA;
    if !sigma.is_finite() || sigma <= 0.0 {
        return None;
    }
    Some((median, sigma))
}

/// First index at or after `from` that belongs to the site with origin parity `origin`.
#[inline]
fn align_to_site(from: usize, origin: usize, step: usize) -> usize {
    from + (origin + step - from % step) % step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debayer::CfaPattern;
    use crate::frame::Frame;

    /// A noisy but deterministic sky, so MAD has something to measure.
    fn sky(width: usize, height: usize, level: f32, noise: f32) -> Frame {
        let mut frame = Frame::zeros(width, height, 1).unwrap();
        let mut seed = 0x9E3779B9u32;
        for y in 0..height {
            for x in 0..width {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let unit = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                frame.set_pixel(x, y, 0, level + unit * noise);
            }
        }
        frame
    }

    fn mosaic(frame: Frame) -> CfaFrame {
        CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap()
    }

    #[test]
    fn aligns_a_crop_start_onto_each_site() {
        assert_eq!(align_to_site(10, 0, 2), 10);
        assert_eq!(align_to_site(10, 1, 2), 11);
        assert_eq!(align_to_site(11, 0, 2), 12);
        assert_eq!(align_to_site(11, 1, 2), 11);
        assert_eq!(align_to_site(11, 0, 1), 11);
    }

    #[test]
    fn replaces_an_isolated_hot_sample_with_its_neighbourhood() {
        let mut frame = sky(128, 128, 0.10, 0.004);
        frame.set_pixel(64, 64, 0, 0.9);
        let mut cfa = mosaic(frame);

        let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();

        assert_eq!(stats.corrected, 1);
        assert!(
            (cfa.frame().get_pixel(64, 64, 0) - 0.10).abs() < 0.01,
            "hot sample should land back on the local sky level"
        );
    }

    #[test]
    fn leaves_a_star_core_alone_however_bright_it_is() {
        // A Gaussian a couple of samples wide on each colour site — the case a
        // plain `centre - max(neighbours) > tau` test clips.
        for peak in [0.05f32, 0.4, 0.95] {
            let mut frame = sky(128, 128, 0.10, 0.004);
            let (cx, cy) = (64i32, 64i32);
            for dy in -6i32..=6 {
                for dx in -6i32..=6 {
                    let r2 = (dx * dx + dy * dy) as f32;
                    let v = peak * (-r2 / 8.0).exp();
                    let (x, y) = ((cx + dx) as usize, (cy + dy) as usize);
                    frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + v);
                }
            }
            let before = frame.get_pixel(64, 64, 0);
            let mut cfa = mosaic(frame);

            reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();

            assert_eq!(
                cfa.frame().get_pixel(64, 64, 0),
                before,
                "star peak {peak} was clipped"
            );
        }
    }

    #[test]
    fn a_bright_neighbouring_site_is_not_a_neighbour() {
        // Every R sample bright, every other site dark: the mosaic itself must
        // not read as a field of hot pixels.
        let mut frame = sky(128, 128, 0.05, 0.002);
        for y in (0..128).step_by(2) {
            for x in (0..128).step_by(2) {
                frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + 0.5);
            }
        }
        let mut cfa = mosaic(frame);

        let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();

        assert_eq!(stats.corrected, 0);
    }

    #[test]
    fn a_cold_sample_is_left_for_the_dark_frame_to_deal_with() {
        let mut frame = sky(128, 128, 0.10, 0.004);
        frame.set_pixel(64, 64, 0, 0.0);
        let mut cfa = mosaic(frame);

        let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();

        assert_eq!(stats.corrected, 0);
        assert_eq!(cfa.frame().get_pixel(64, 64, 0), 0.0);
    }

    #[test]
    fn a_flat_frame_has_no_noise_estimate_and_is_left_untouched() {
        let mut cfa = mosaic(Frame::filled(64, 64, 1, 0.2).unwrap());

        let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();

        assert_eq!(stats.corrected, 0);
        assert_eq!(stats.sites_skipped, 4);
        assert!(cfa.frame().data().iter().all(|&v| v == 0.2));
    }

    #[test]
    fn a_mono_frame_uses_its_immediate_neighbours() {
        let mut frame = sky(128, 128, 0.10, 0.004);
        frame.set_pixel(64, 64, 0, 0.9);
        let mut cfa = CfaFrame::direct(frame);

        let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();

        assert_eq!(stats.corrected, 1);
        assert!((cfa.frame().get_pixel(64, 64, 0) - 0.10).abs() < 0.01);
    }

    #[test]
    fn an_rgb_frame_is_rejected_rather_than_filtered_as_if_it_were_a_mosaic() {
        let mut cfa = CfaFrame::direct(Frame::filled(16, 16, 3, 0.5).unwrap());
        assert!(reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).is_err());
    }

    #[test]
    fn the_frame_border_is_left_uncorrected_rather_than_read_out_of_bounds() {
        let mut frame = sky(64, 64, 0.10, 0.004);
        for (x, y) in [(0usize, 0usize), (63, 0), (0, 63), (63, 63), (1, 30)] {
            frame.set_pixel(x, y, 0, 0.9);
        }
        let mut cfa = mosaic(frame);

        let stats = reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).unwrap();

        assert_eq!(stats.corrected, 0);
    }
}
