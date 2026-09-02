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

use std::sync::Mutex;

use rayon::prelude::*;

use crate::error::{Result, StackError};
use crate::statistics::fast_median;

use super::{CfaFrame, CfaPlanes, CfaStage};

/// Samples drawn from the centre crop to estimate one site's noise level.
const MAX_SIGMA_SAMPLES: usize = 32_768;

/// How many frames one set of per-site background and noise estimates is reused
/// for.
///
/// The estimate is two median passes over ~34 000 samples for each of the four
/// colour sites, and on a 9 MP frame it is a large share of what this filter
/// costs. What it measures — the sky level and its MAD — moves on the timescale
/// of the sky itself: twilight, a passing cloud, a gain change. Recomputing it
/// per sub buys nothing a 32-frame refresh does not, and a stale estimate shifts
/// the threshold only by however much the sky actually drifted underneath it.
///
/// The estimate is also dropped outright whenever the frame's shape changes, so
/// binning or an ROI change cannot be served from a stale one.
const SITE_STATS_TTL_FRAMES: u32 = 32;

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

/// Per-site background and noise, and how long it has been in use.
#[derive(Debug)]
struct CachedSites {
    /// Frame shape the estimate was taken on. A change to any of it — binning,
    /// an ROI, a mono/colour swap — invalidates the estimate outright.
    shape: (usize, usize, usize),
    /// `(background, sigma)` per colour site, in [`CfaPlanes::origins`] order.
    /// `None` for a site whose estimate was unusable.
    sites: Vec<Option<(f32, f32)>>,
    /// Frames served from this estimate so far.
    age: u32,
}

/// A registered [`CfaStage`] wrapper around [`reject_hot_pixels`].
///
/// Owns the per-site noise estimate across frames — the precomputed state
/// [`super::CfaPipeline`] is built per settings-change to hold. Rebuilding the
/// stage (which the stacking task does whenever the correction settings or the
/// stacking type move) drops it.
#[derive(Debug, Default)]
pub struct HotPixelFilter {
    config: HotPixelConfig,
    /// A `Mutex` rather than a `RefCell` because `CfaStage` is `Sync`; it is
    /// uncontended in practice, since one stacking task owns the pipeline.
    cached: Mutex<Option<CachedSites>>,
}

impl HotPixelFilter {
    /// Build the stage with explicit tuning.
    pub fn new(config: HotPixelConfig) -> Self {
        Self {
            config,
            cached: Mutex::new(None),
        }
    }

    /// Per-site estimates for this frame, reusing the cached set while it is
    /// still fresh and describes the same frame shape.
    fn site_stats(&self, cfa: &CfaFrame, planes: &CfaPlanes) -> Vec<Option<(f32, f32)>> {
        let shape = (planes.width, planes.height, planes.step);
        let mut guard = match self.cached.lock() {
            Ok(guard) => guard,
            // A poisoned lock means a previous estimate panicked. Recomputing is
            // always correct, so this must not cost the exposure.
            Err(poisoned) => poisoned.into_inner(),
        };

        if let Some(cached) = guard.as_mut() {
            if cached.shape == shape && cached.age < SITE_STATS_TTL_FRAMES {
                cached.age += 1;
                return cached.sites.clone();
            }
        }

        let data = cfa.frame().data();
        let sites: Vec<Option<(f32, f32)>> = planes
            .origins()
            .map(|(x0, y0)| {
                site_background(data, planes.width, planes.height, x0, y0, planes.step)
            })
            .collect();
        *guard = Some(CachedSites {
            shape,
            sites: sites.clone(),
            age: 0,
        });
        sites
    }
}

impl CfaStage for HotPixelFilter {
    fn name(&self) -> &'static str {
        "hot_pixels"
    }

    fn apply(&self, frame: &mut CfaFrame) -> Result<()> {
        let Some(planes) = frame.planes() else {
            return Err(StackError::ChannelMismatch {
                expected: 1,
                actual: frame.frame().channels(),
            });
        };
        let sites = self.site_stats(frame, &planes);
        let stats = reject_hot_pixels_with(frame, &self.config, &sites)?;
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
    let data = cfa.frame().data();
    let sites: Vec<Option<(f32, f32)>> = planes
        .origins()
        .map(|(x0, y0)| site_background(data, planes.width, planes.height, x0, y0, planes.step))
        .collect();
    reject_hot_pixels_with(cfa, config, &sites)
}

/// [`reject_hot_pixels`] against per-site estimates the caller already holds.
///
/// `sites` is `(background, sigma)` in [`CfaPlanes::origins`] order, `None` for a
/// site whose estimate was unusable. Splitting the estimate from the sweep is
/// what lets [`HotPixelFilter`] keep it across frames.
pub fn reject_hot_pixels_with(
    cfa: &mut CfaFrame,
    config: &HotPixelConfig,
    sites: &[Option<(f32, f32)>],
) -> Result<HotPixelStats> {
    let Some(planes) = cfa.planes() else {
        return Err(StackError::ChannelMismatch {
            expected: 1,
            actual: cfa.frame().channels(),
        });
    };

    let (width, height, step) = (planes.width, planes.height, planes.step);
    let mut stats = HotPixelStats::default();

    // Per-site thresholds in `CfaPlanes::origins` order, which is `y0 * step + x0`.
    // Resolved up front so the row sweep below is a lookup rather than a branch on
    // `Option` plus a NaN test per sample.
    let thresholds: Vec<Option<(f32, f32)>> = sites
        .iter()
        .map(|site| {
            let (background, sigma) = (*site)?;
            let tau = config.sigma * sigma;
            (!tau.is_nan() && tau > 0.0).then_some((background, tau))
        })
        .collect();
    stats.sites_skipped = thresholds.iter().filter(|t| t.is_none()).count();

    // # One sweep per row parity, not one per colour site
    //
    // The four Bayer sites are two pairs that share a row parity: `(0, 0)` and `(1, 0)`
    // read exactly the same three rows, as do `(0, 1)` and `(1, 1)`. Sweeping them
    // separately — which is what a loop over `origins()` does — walks the whole 36 MB
    // mosaic four times per frame instead of twice, and each of those passes reads every
    // cache line to use half of it, because the samples of one site sit `step` apart.
    //
    // Grouping by row parity makes each row triple one DRAM fetch serving both x
    // parities, taking the stage from four passes over the mosaic to two.
    //
    // **On x86 this is worth nothing, and that is expected.** `cfa_hot_pixels` measures
    // 112.8 ms against 112.3 ms at 3008x3008 — inside the noise. With 20 cores the stage
    // is compute-bound (eight `max` operations and three compares per sample), not
    // bandwidth-bound, so removing DRAM traffic removes nothing that was on the critical
    // path. The 34 ms this stage reported in production traces is largely rayon
    // contention with the render thread; uncontended it is ~7 ms.
    //
    // It is kept for the same reason `render::simd` keeps its NEON kernels on x86
    // evidence it does not trust: a Pi 5 has a fifth of the cores and a fifth of the
    // bandwidth, which moves this stage to the other side of that balance. The
    // benchmark is the case that would settle it there. The change is also not free of
    // benefit here — see the accumulator note below.
    //
    // Detection still reads the frame and the replacements are still applied afterwards,
    // so a corrected sample can never feed the test for one of its neighbours and the
    // result does not depend on how rayon split the rows.
    let corrections: Vec<(usize, f32)> = {
        let data = cfa.frame().data();
        let scan_rows: Vec<(usize, usize)> = (0..step)
            .flat_map(|y0| {
                (y0 + step..height.saturating_sub(step))
                    .step_by(step)
                    .map(move |y| (y, y0))
            })
            .collect();

        // `fold`/`reduce` rather than `flat_map_iter`: one accumulator per rayon task
        // instead of one `Vec` per row. At 3008x3008 that is thousands of allocations a
        // frame to hold a few hundred hits. Every index belongs to exactly one site and
        // is visited once, so the order tasks finish in cannot change the result.
        scan_rows
            .into_par_iter()
            .fold(Vec::new, |mut hits, (y, y0)| {
                scan_row_into(
                    &mut hits,
                    data,
                    width,
                    step,
                    y,
                    y0,
                    &thresholds,
                    config.isolation,
                );
                hits
            })
            .reduce(Vec::new, |mut a, mut b| {
                a.append(&mut b);
                a
            })
    };

    stats.corrected = corrections.len();
    let data = cfa.frame_mut().data_mut();
    for (idx, value) in corrections {
        data[idx] = value;
    }
    Ok(stats)
}

/// One row of the detection sweep, for every colour site that shares this row parity.
///
/// The three row slices are taken once and walked once per x parity. Both walks hit the
/// same cache lines, so the second is served from L1/L2 rather than from DRAM — which is
/// the whole point of grouping the sites this way. `thresholds` is indexed in
/// [`CfaPlanes::origins`] order, so this row's site `x0` is at `y0 * step + x0`.
#[allow(clippy::too_many_arguments)]
fn scan_row_into(
    hits: &mut Vec<(usize, f32)>,
    data: &[f32],
    width: usize,
    step: usize,
    y: usize,
    y0: usize,
    thresholds: &[Option<(f32, f32)>],
    isolation: f32,
) {
    let up = &data[(y - step) * width..][..width];
    let mid = &data[y * width..][..width];
    let down = &data[(y + step) * width..][..width];

    for x0 in 0..step {
        let Some((background, tau)) = thresholds[y0 * step + x0] else {
            continue;
        };

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
    }
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

    /// The estimate is reused across frames — that is the point of caching it —
    /// but a hot sample must still be corrected on every frame, not only on the
    /// one the estimate was taken from.
    #[test]
    fn a_reused_estimate_still_corrects_every_frame() {
        let filter = HotPixelFilter::new(HotPixelConfig::default());

        for round in 0..3 {
            let mut frame = sky(64, 64, 0.2, 0.02);
            frame.set_pixel(20, 20, 0, 0.95);
            let mut cfa = mosaic(frame);
            filter.apply(&mut cfa).unwrap();
            assert!(
                cfa.frame().get_pixel(20, 20, 0) < 0.4,
                "round {round}: hot sample survived at {}",
                cfa.frame().get_pixel(20, 20, 0)
            );
        }
    }

    /// A stale estimate must never be served to a differently-shaped frame: a
    /// binning or ROI change moves both the sample count and the level.
    #[test]
    fn a_shape_change_drops_the_cached_estimate() {
        let filter = HotPixelFilter::new(HotPixelConfig::default());

        let mut small = mosaic(sky(64, 64, 0.2, 0.02));
        filter.apply(&mut small).unwrap();
        let shape_after_first = filter.cached.lock().unwrap().as_ref().unwrap().shape;
        assert_eq!(shape_after_first, (64, 64, 2));

        // A brighter, larger frame: if the estimate were reused the threshold
        // would still be the small frame's.
        let mut large = mosaic(sky(96, 96, 0.5, 0.02));
        filter.apply(&mut large).unwrap();
        let cached = filter.cached.lock().unwrap();
        let cached = cached.as_ref().unwrap();
        assert_eq!(cached.shape, (96, 96, 2));
        assert_eq!(cached.age, 0, "a reshaped frame must re-estimate, not age");
    }

    /// The estimate ages out rather than being kept forever, so a sky that
    /// drifts — twilight, cloud, a gain change — is eventually re-measured.
    #[test]
    fn the_estimate_is_re_derived_once_it_ages_out() {
        let filter = HotPixelFilter::new(HotPixelConfig::default());
        // The first frame estimates and sets `age` to 0, the next TTL frames are
        // served from it, and the one after that re-estimates.
        for _ in 0..SITE_STATS_TTL_FRAMES + 2 {
            let mut cfa = mosaic(sky(64, 64, 0.2, 0.02));
            filter.apply(&mut cfa).unwrap();
        }
        assert_eq!(
            filter.cached.lock().unwrap().as_ref().unwrap().age,
            0,
            "estimate should have been refreshed on the frame after the TTL"
        );
    }

    #[test]
    fn an_rgb_frame_is_rejected_rather_than_filtered_as_if_it_were_a_mosaic() {
        let mut cfa = CfaFrame::direct(Frame::filled(16, 16, 3, 0.5).unwrap());
        assert!(reject_hot_pixels(&mut cfa, &HotPixelConfig::default()).is_err());
    }

    /// A sky whose four Bayer sites sit at deliberately different levels with
    /// deliberately different noise, so a threshold resolved for the wrong site gives a
    /// different answer. `sites` is indexed in [`CfaPlanes::origins`] order.
    fn sky_per_site(width: usize, height: usize, sites: [(f32, f32); 4]) -> Frame {
        let mut frame = Frame::zeros(width, height, 1).unwrap();
        let mut seed = 0x9E3779B9u32;
        for y in 0..height {
            for x in 0..width {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let unit = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                let (level, noise) = sites[(y % 2) * 2 + (x % 2)];
                frame.set_pixel(x, y, 0, level + unit * noise);
            }
        }
        frame
    }

    /// A plain four-sweep reference for the detector: one independent pass per colour
    /// site, with that site's own statistics resolved *inside* the loop.
    ///
    /// The oracle deliberately contains no index arithmetic. The production sweep groups
    /// the four sites into two passes by row parity and reads each site's threshold out
    /// of a flat table at `y0 * step + x0`; that index is the load-bearing part of the
    /// two-sweep rewrite, and nothing else in the suite pins it.
    fn reference_corrections(
        data: &[f32],
        width: usize,
        height: usize,
        step: usize,
        config: &HotPixelConfig,
    ) -> Vec<(usize, f32)> {
        let mut hits = Vec::new();
        for y0 in 0..step {
            for x0 in 0..step {
                let Some((background, sigma)) =
                    site_background(data, width, height, x0, y0, step)
                else {
                    continue;
                };
                let tau = config.sigma * sigma;
                if tau.is_nan() || tau <= 0.0 {
                    continue;
                }

                let mut y = y0 + step;
                while y + step < height {
                    let mut x = x0 + step;
                    while x + step < width {
                        let at = |xx: usize, yy: usize| data[yy * width + xx];
                        let centre = at(x, y);
                        let (nw, n, ne) = (at(x - step, y - step), at(x, y - step), at(x + step, y - step));
                        let (w, e) = (at(x - step, y), at(x + step, y));
                        let (sw, s, se) = (at(x - step, y + step), at(x, y + step), at(x + step, y + step));
                        let brightest = nw.max(n).max(ne).max(w).max(e).max(sw).max(s).max(se);
                        let above_background = centre - background;
                        if centre - brightest > tau
                            && above_background > 0.0
                            && brightest - background < config.isolation * above_background
                        {
                            hits.push((y * width + x, (nw + n + ne + w + e + sw + s + se) * 0.125));
                        }
                        x += step;
                    }
                    y += step;
                }
            }
        }
        hits
    }

    /// Every colour site must be swept against **its own** threshold.
    ///
    /// The two-sweep rewrite replaced a per-site loop with a flat table indexed
    /// `thresholds[y0 * step + x0]`. Transposing that to `thresholds[x0 * step + y0]`
    /// swaps the two green sites on any Bayer pattern, and on a normal frame the two
    /// greens have near-identical statistics — so the whole suite passed against the
    /// transposition. This fixture gives the four sites deliberately unequal backgrounds
    /// and sigmas and plants one sample on each, sized so the quiet green's clears its
    /// own threshold while the noisy green's does not.
    ///
    /// Asserting against the four-sweep reference pins the site ordering, the row-parity
    /// grouping and the sweep's index ranges at once.
    #[test]
    fn every_site_is_swept_against_its_own_threshold() {
        const W: usize = 128;
        const H: usize = 128;
        let config = HotPixelConfig::default();

        // (background, noise) per site, in `origins` order: R, G1, G2, B on RGGB.
        // G1 is quiet and G2 is noisy — that asymmetry is the only thing that can tell
        // the two apart, and it is exactly what the transposition destroys.
        let mut frame = sky_per_site(W, H, [(0.10, 0.010), (0.20, 0.002), (0.30, 0.020), (0.40, 0.006)]);
        let plants = [
            ((40usize, 40usize), 0.30f32), // R  — hot by a wide margin
            ((41, 40), 0.015),             // G1 — hot against 5 sigma of 0.002
            ((40, 41), 0.015),             // G2 — *not* hot against 5 sigma of 0.020
            ((41, 41), 0.30),              // B  — hot by a wide margin
        ];
        for ((x, y), excess) in plants {
            frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + excess);
        }

        let mut expected = frame.data().to_vec();
        let reference = reference_corrections(&expected.clone(), W, H, 2, &config);
        for (idx, value) in &reference {
            expected[*idx] = *value;
        }
        assert_eq!(
            reference.len(),
            3,
            "fixture must exercise a hit on three sites and a miss on the fourth"
        );

        let mut cfa = mosaic(frame);
        let stats = reject_hot_pixels(&mut cfa, &config).unwrap();

        assert_eq!(stats.corrected, reference.len());
        assert_eq!(
            cfa.frame().data(),
            expected.as_slice(),
            "the two-sweep detector disagreed with a plain four-sweep reference"
        );

        // Spelled out, because the whole-frame comparison would also pass if both sides
        // were wrong in the same direction: the sites the transposition swaps are the
        // two greens, and they must land on opposite answers here.
        assert!(
            cfa.frame().get_pixel(41, 40, 0) < 0.21,
            "the quiet green's planted sample should have been corrected"
        );
        assert!(
            cfa.frame().get_pixel(40, 41, 0) > 0.31,
            "the noisy green's planted sample is inside its own noise and must survive"
        );
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
