//! Cross-frame reuse of three *stack-level* estimates (white balance, background
//! model, median/MAD) that move only 1/N per render; recomputing them costs 6.4ms of
//! 11.2ms render time on an IMX464 frame with bilinear background (Pro's RBF costs
//! more; pixel work like neutralisation/subtraction/black-point stays uncached).
//! Invalidated by live view, a settings change ([`AnalysisKey`], NaN-safe bit
//! fingerprint), a shape change, or stack growth (MAD falls as `1/sqrt(N)`: 1->2
//! subs halves noise, 140->141 nothing) — bounded by [`DEPTH_GROWTH`]/[`MAX_AGE_FRAMES`].
//! A reused MAD is carried to frame depth ([`NoiseTrend`]) rather than measured fresh, which once pulsed the tone curve each refresh (M27 depth 24-30, grain 2.54→2.32→2.45).

use std::sync::Arc;

use night_amplifier_core::background::{BackgroundConfig, BackgroundExtractionAlgorithm, BackgroundModel, FieldProtection};
use night_amplifier_core::error::Result;
use night_amplifier_core::statistics::ImageStats;
use night_amplifier_core::stacking::CropRect;

/// Frames one analysis is reused for before it is recomputed regardless of stack growth.
///
/// The sky moves on its own — twilight, cloud, a passing gradient — and a stack whose
/// frames are all being rejected does not grow at all, so proportional growth cannot be
/// the only refresh trigger. Eight frames is one to two seconds at the rates this
/// pipeline runs at.
const MAX_AGE_FRAMES: u32 = 8;

/// Relative growth in stack depth that forces a refresh.
///
/// 1.25 means the estimates are recomputed once the stack is a quarter deeper than when
/// they were taken, which is up to a ~12 % change in MAD — carried in between by
/// [`NoiseTrend`], so a refresh only corrects what the trend got wrong.
const DEPTH_GROWTH: f32 = 1.25;

/// Steepest fall of the stack's noise with depth a trend may extrapolate.
///
/// `1/sqrt(N)` is pure averaging of independent subs, and nothing a stack does falls
/// faster; real stacks fall slower (the 106-sub IMX533 set: `N^-0.41` to 32 subs, then
/// `N^-0.19`) because sky structure under the MAD does not average away.
const MAX_NOISE_EXPONENT: f32 = 0.5;

/// What the frame being analysed is, from the pipeline's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisContext {
    /// The frame is the accumulated stack rather than a single sub.
    pub showing_stack: bool,
    /// Frames in that stack.
    pub stack_depth: u32,
}

impl AnalysisContext {
    /// A frame that has to be analysed on its own terms.
    ///
    /// Live view, and every one-shot caller: the stacked-PNG export and the FITS export
    /// both run once per session against a frame nothing else will see, so there is
    /// neither anything to reuse nor anything worth storing.
    pub const ONE_SHOT: Self = Self {
        showing_stack: false,
        stack_depth: 0,
    };
}

/// Everything the three estimates read, reduced to something comparable.
///
/// Explicit rather than a `PartialEq` on the config types: `BackgroundConfig` carries
/// `f32` fields, and deriving equality on floats would make a `NaN` setting compare
/// unequal to itself and silently disable the cache — or, with the comparison the other
/// way round, pin it open. Bit patterns are total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AnalysisKey {
    dimensions: (usize, usize, usize),
    background_subtraction: bool,
    algorithm: BackgroundExtractionAlgorithm,
    grid: (usize, usize),
    star_rejection_sigma: u32,
    gradient_only: bool,
    reference_percentile: u32,
    aggressiveness: u32,
    edge_inset: u32,
    /// The catalog protection's generation: a plate solve landing mid-session must
    /// rebuild the model once, not wait for the stack to grow.
    protection: Option<u64>,
    scnr: bool,
    scnr_amount: u32,
    auto_stretch: bool,
}

impl AnalysisKey {
    fn new(
        dimensions: (usize, usize, usize),
        background_subtraction: bool,
        background: &BackgroundConfig,
        scnr: bool,
        scnr_amount: f32,
        auto_stretch: bool,
    ) -> Self {
        Self {
            dimensions,
            background_subtraction,
            algorithm: background.algorithm,
            grid: (background.grid_width, background.grid_height),
            star_rejection_sigma: background.star_rejection_sigma.to_bits(),
            gradient_only: background.gradient_only,
            reference_percentile: background.reference_percentile.to_bits(),
            aggressiveness: background.aggressiveness.to_bits(),
            edge_inset: background.edge_inset.to_bits(),
            protection: background.protection.as_ref().map(|p| p.generation),
            scnr,
            scnr_amount: scnr_amount.to_bits(),
            auto_stretch,
        }
    }
}

/// One frame's worth of reusable estimates, and the key they were taken under.
#[derive(Debug)]
struct Cached {
    key: AnalysisKey,
    /// Stack depth when the estimates were taken.
    depth: u32,
    /// Frames served from them since.
    age: u32,
    white_balance: Option<[f32; 3]>,
    background: Option<BackgroundModel>,
    stats: Option<ImageStats>,
}

/// How the stack's noise falls with depth, from its last two measurements.
///
/// Between refreshes a reused MAD is carried from the depth it was measured at to the
/// frame's, as `sigma ∝ N^-exponent`: the curve then moves every frame as the stack does,
/// and a refresh corrects only the trend's error. The exponent is clamped to
/// `[0, MAX_NOISE_EXPONENT]` — a measurement that rose (a cloud, a bright sub) holds
/// rather than predicting noise that grows with depth. Over the most a trend is carried,
/// a quarter's growth, a clamped extreme moves the MAD at most 12 %.
#[derive(Debug, Default, Clone, Copy)]
struct NoiseTrend {
    /// Depth and mean sigma of the latest measurement.
    latest: Option<(u32, f32)>,
    exponent: f32,
}

impl NoiseTrend {
    fn observe(&mut self, depth: u32, sigma: f32) {
        if depth == 0 || !sigma.is_finite() || sigma <= 0.0 {
            return;
        }
        if let Some((previous_depth, previous_sigma)) = self.latest {
            if depth > previous_depth {
                let exponent =
                    (previous_sigma / sigma).ln() / (depth as f32 / previous_depth as f32).ln();
                self.exponent = exponent.clamp(0.0, MAX_NOISE_EXPONENT);
            }
        }
        self.latest = Some((depth, sigma));
    }

    /// Forgets the latest measurement but keeps the exponent: the next one was taken over a
    /// different crop of the stack, and an exponent derived across the two would read the
    /// border's removal (~15 % less MAD) as noise falling with depth.
    fn rebase(&mut self) {
        self.latest = None;
    }

    /// Factor carrying the latest measurement's noise to `depth`.
    fn scale_to(&self, depth: u32) -> f32 {
        match self.latest {
            Some((measured_at, _)) if depth > measured_at => {
                (measured_at as f32 / depth as f32).powf(self.exponent)
            }
            _ => 1.0,
        }
    }
}

/// The render thread's analysis cache.
///
/// Owned by the render task for the life of the thread, the same way
/// `render_task::ConversionCache` owns the denoise buffers, and passed into
/// `process_preview_frame` explicitly rather than kept in a thread-local — a one-shot
/// caller on a pooled blocking thread must not strand a background model per worker.
#[derive(Debug, Default)]
pub struct PreviewAnalysis {
    cached: Option<Cached>,
    /// Whether the current frame may read from `cached`. Set by [`Self::begin_frame`].
    reuse: bool,
    /// Stack depth of the current frame, which a reused MAD is carried to.
    depth: u32,
    /// Kept across refreshes of one stack, dropped with it.
    trend: NoiseTrend,
    /// The stack crop of the frame being analysed — see [`Self::note_crop`].
    crop: Option<CropRect>,
    /// `crop` moved since the last frame.
    crop_changed: bool,
    /// The stack's catalog footprints on the frame being analysed — see
    /// [`Self::set_protection`].
    protection: Option<Arc<FieldProtection>>,
}

impl PreviewAnalysis {
    pub fn new() -> Self {
        Self::default()
    }

    /// The crop the render task cut the next frame to (`StackedFrame::crop`), before
    /// [`Self::begin_frame`].
    ///
    /// A crop change is the same stack seen through a new window: its estimates describe the
    /// old one and are measured again, but the `NoiseTrend` carries on, rebased. Treated as a
    /// new stack, every change (one per 3-4 subs under drift) dropped the trend the stacked
    /// view holds still with. A size change under an unchanged crop still resets everything.
    pub fn note_crop(&mut self, crop: Option<CropRect>) {
        self.crop_changed = self.crop != crop;
        self.crop = crop;
    }

    /// The stack's catalog footprints, mapped onto the next frame, before
    /// [`Self::begin_frame`]. A plate solve landing mid-stack is treated like a crop
    /// change: the estimates are measured again, the noise trend carries on.
    pub fn set_protection(&mut self, protection: Option<Arc<FieldProtection>>) {
        self.protection = protection;
    }

    /// What [`Self::set_protection`] last set, for the background config.
    pub fn protection(&self) -> Option<Arc<FieldProtection>> {
        self.protection.clone()
    }

    /// Decide once, for this frame, whether the stored estimates still describe it.
    ///
    /// Called before any of the three getters, and the decision is shared by all of
    /// them: mixing a fresh background model with a stale set of statistics would
    /// measure the sky against a gradient that had already been removed differently.
    ///
    /// Returns whether anything will be reused, for the caller's span.
    // The inputs `AnalysisKey` is built from, one each: bundled, they would be the key itself.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_frame(
        &mut self,
        ctx: AnalysisContext,
        dimensions: (usize, usize, usize),
        background_subtraction: bool,
        background: &BackgroundConfig,
        scnr: bool,
        scnr_amount: f32,
        auto_stretch: bool,
    ) -> bool {
        let key = AnalysisKey::new(
            dimensions,
            background_subtraction,
            background,
            scnr,
            scnr_amount,
            auto_stretch,
        );

        let crop_changed = std::mem::take(&mut self.crop_changed);
        let protection_changed = self.cached.as_ref().is_some_and(|c| c.key.protection != key.protection);
        let same_stack = ctx.showing_stack && self.continues_stack(&key, ctx, crop_changed, protection_changed);
        let remeasure = crop_changed || protection_changed;
        if !same_stack {
            self.trend = NoiseTrend::default();
        } else if remeasure {
            self.trend.rebase();
        }
        self.reuse = same_stack && !remeasure && self.is_current(ctx);
        self.depth = ctx.stack_depth;

        if self.reuse {
            if let Some(cached) = self.cached.as_mut() {
                cached.age += 1;
            }
        } else {
            self.cached = Some(Cached {
                key,
                depth: ctx.stack_depth,
                age: 0,
                white_balance: None,
                background: None,
                stats: None,
            });
        }

        self.reuse
    }

    /// The stored estimates were taken from an earlier frame of this same stack.
    /// `crop_changed` lets the dimensions differ, since a new crop is what changed them;
    /// `protection_changed` the protection, since a solve landing is what changed it.
    fn continues_stack(&self, key: &AnalysisKey, ctx: AnalysisContext, crop_changed: bool, protection_changed: bool) -> bool {
        // A stack that has not been measured yet, or that restarted, has nothing to
        // compare against — `stack_depth` going *down* is a reset.
        self.cached.as_ref().is_some_and(|cached| {
            let comparable = AnalysisKey {
                dimensions: if crop_changed { key.dimensions } else { cached.key.dimensions },
                protection: if protection_changed { key.protection } else { cached.key.protection },
                ..cached.key
            };
            let same_key = comparable == *key;
            same_key && cached.depth != 0 && ctx.stack_depth >= cached.depth
        })
    }

    /// ...and recently enough to be reused.
    fn is_current(&self, ctx: AnalysisContext) -> bool {
        self.cached.as_ref().is_some_and(|cached| {
            cached.age < MAX_AGE_FRAMES
                && (ctx.stack_depth as f32) < cached.depth as f32 * DEPTH_GROWTH
        })
    }

    /// White-balance multipliers, computing them if this frame cannot reuse the stored
    /// set.
    pub fn white_balance<F>(&mut self, compute: F) -> Result<[f32; 3]>
    where
        F: FnOnce() -> Result<[f32; 3]>,
    {
        if self.reuse {
            if let Some(value) = self.cached.as_ref().and_then(|c| c.white_balance) {
                return Ok(value);
            }
        }
        let value = compute()?;
        if let Some(cached) = self.cached.as_mut() {
            cached.white_balance = Some(value);
        }
        Ok(value)
    }

    /// The background model, computing it if this frame cannot reuse the stored one.
    ///
    /// Hands back a borrow rather than a clone: the model carries one `Vec<f32>` per
    /// channel of `eval_width * eval_height`, and copying it per frame would give back a
    /// slice of what caching it saves.
    pub fn background<F>(&mut self, compute: F) -> Result<&BackgroundModel>
    where
        F: FnOnce() -> Result<BackgroundModel>,
    {
        let needs_compute = !self.reuse
            || self
                .cached
                .as_ref()
                .is_none_or(|c| c.background.is_none());

        if needs_compute {
            let model = compute()?;
            if let Some(cached) = self.cached.as_mut() {
                cached.background = Some(model);
            }
        }

        self.cached
            .as_ref()
            .and_then(|c| c.background.as_ref())
            .ok_or_else(|| {
                night_amplifier_core::error::StackError::InvalidConfiguration(
                    "background model missing after computation".into(),
                )
            })
    }

    /// Per-channel statistics, computing them if this frame cannot reuse the stored set.
    ///
    /// A reused set comes back with its spread carried to this frame's depth along the
    /// stack's `NoiseTrend`; its levels are served as measured.
    pub fn stats<F>(&mut self, compute: F) -> Result<ImageStats>
    where
        F: FnOnce() -> Result<ImageStats>,
    {
        if self.reuse {
            if let Some(value) = self.cached.as_ref().and_then(|c| c.stats.as_ref()) {
                return Ok(value.with_noise_scaled(self.trend.scale_to(self.depth)));
            }
        }
        let value = compute()?;
        if let Some(cached) = self.cached.as_mut() {
            cached.stats = Some(value.clone());
            self.trend.observe(cached.depth, value.mean_sigma());
        }
        Ok(value)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BackgroundConfig {
        BackgroundConfig::default()
    }

    fn begin(
        analysis: &mut PreviewAnalysis,
        ctx: AnalysisContext,
        cfg: &BackgroundConfig,
    ) -> bool {
        analysis.begin_frame(ctx, (100, 100, 3), true, cfg, true, 1.0, true)
    }

    fn stacked(depth: u32) -> AnalysisContext {
        AnalysisContext {
            showing_stack: true,
            stack_depth: depth,
        }
    }

    #[test]
    fn the_first_frame_has_nothing_to_reuse() {
        let mut a = PreviewAnalysis::new();
        assert!(!begin(&mut a, stacked(10), &config()));
    }

    #[test]
    fn a_deep_stack_reuses_between_refreshes() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        assert!(!begin(&mut a, stacked(100), &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));

        // 101..124 are all inside the 25 % growth band.
        for depth in [101, 110, 120] {
            assert!(begin(&mut a, stacked(depth), &cfg), "depth {depth}");
        }
    }

    /// The rule that matters: proportional growth, not elapsed frames. A fixed TTL would
    /// serve a one-frame stack's statistics to a four-frame stack, whose noise is half
    /// as large.
    #[test]
    fn a_shallow_stack_refreshes_every_frame() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        assert!(!begin(&mut a, stacked(1), &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));

        assert!(
            !begin(&mut a, stacked(2), &cfg),
            "doubling the stack halves the noise; the stretch cannot be stale here"
        );
    }

    #[test]
    fn reuse_stops_at_the_growth_threshold() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        assert!(!begin(&mut a, stacked(100), &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        assert!(!begin(&mut a, stacked(125), &cfg), "125 == 100 * 1.25");
    }

    #[test]
    fn reuse_stops_at_the_age_cap() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        // A stack that is not growing: only the age cap can refresh it.
        assert!(!begin(&mut a, stacked(1000), &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        // The measuring frame leaves the entry at age 0, so `MAX_AGE_FRAMES` frames are
        // served from it before the cap is reached.
        for served in 0..MAX_AGE_FRAMES {
            assert!(begin(&mut a, stacked(1000), &cfg), "frame {served}");
        }
        assert!(
            !begin(&mut a, stacked(1000), &cfg),
            "a stack that stops growing still has a sky that moves"
        );
    }

    #[test]
    fn live_view_never_reuses() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        let live = AnalysisContext {
            showing_stack: false,
            stack_depth: 0,
        };
        assert!(!begin(&mut a, live, &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        assert!(!begin(&mut a, live, &cfg), "every sub is a different image");
    }

    #[test]
    fn a_settings_change_invalidates() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        assert!(!begin(&mut a, stacked(100), &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        assert!(begin(&mut a, stacked(101), &cfg));

        let mut changed = config();
        changed.aggressiveness += 0.1;
        assert!(!begin(&mut a, stacked(102), &changed));
    }

    #[test]
    fn a_shape_change_invalidates() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        assert!(!a.begin_frame(stacked(100), (100, 100, 3), true, &cfg, true, 1.0, true));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        assert!(
            !a.begin_frame(stacked(101), (50, 50, 3), true, &cfg, true, 1.0, true),
            "binning changed the frame under the model"
        );
    }

    /// A stack reset takes the depth backwards. Serving the deep stack's statistics to
    /// the new one would stretch a single sub as though it had 140 frames of integration.
    #[test]
    fn a_stack_reset_invalidates() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        assert!(!begin(&mut a, stacked(140), &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        assert!(!begin(&mut a, stacked(1), &cfg));
    }

    /// The getters must not hand back a value the frame did not ask to reuse, and must
    /// store what they computed for the frames that follow.
    #[test]
    fn the_getters_follow_the_frames_decision() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();

        begin(&mut a, stacked(100), &cfg);
        assert_eq!(a.white_balance(|| Ok([2.0, 2.0, 2.0])).unwrap(), [2.0; 3]);

        // Reusing: the closure must not run at all.
        assert!(begin(&mut a, stacked(101), &cfg));
        let value = a
            .white_balance(|| panic!("must not recompute on a reusing frame"))
            .unwrap();
        assert_eq!(value, [2.0; 3]);
    }

    /// Statistics with one sky level and one noise figure on every channel.
    fn sky_stats(level: f32, sigma: f32) -> ImageStats {
        use night_amplifier_core::statistics::{ChannelStats, MAD_TO_SIGMA};
        let channel = ChannelStats::new(level, sigma / MAD_TO_SIGMA, 0.0, 1.0);
        ImageStats {
            channels: vec![channel; 3],
            sample_count: 100_000,
        }
    }

    /// The mean sigma [`sky_stats`] reports for `sigma`, through the same MAD round trip.
    fn measured(sigma: f32) -> f32 {
        sky_stats(0.05, sigma).mean_sigma()
    }

    /// Measure the stack at `depth`, as a refreshing frame does.
    fn measure(a: &mut PreviewAnalysis, depth: u32, sigma: f32) {
        assert!(!begin(a, stacked(depth), &config()), "depth {depth} must refresh");
        a.stats(|| Ok(sky_stats(0.05, sigma))).unwrap();
    }

    /// Serve the stack at `depth` from the stored set.
    fn reuse(a: &mut PreviewAnalysis, depth: u32) -> ImageStats {
        assert!(begin(a, stacked(depth), &config()), "depth {depth} must reuse");
        a.stats(|| panic!("a reusing frame must not measure")).unwrap()
    }

    /// Held as measured, the MAD met a depth gain that advances every frame, and the
    /// render sagged between refreshes and jumped at each. Carried along the stack's own
    /// fall, a reused frame reads what a fresh measurement would.
    #[test]
    fn a_reused_noise_figure_follows_the_stack_deeper() {
        let mut a = PreviewAnalysis::new();
        // Falling as N^-0.3 between the two measurements.
        measure(&mut a, 16, 0.004);
        measure(&mut a, 25, 0.004 * (16.0f32 / 25.0).powf(0.3));

        let sigma_25 = measured(0.004 * (16.0f32 / 25.0).powf(0.3));
        for depth in [26u32, 28, 30] {
            let served = reuse(&mut a, depth).mean_sigma();
            let expected = sigma_25 * (25.0 / depth as f32).powf(0.3);
            assert!(
                (served / expected - 1.0).abs() < 1e-4,
                "depth {depth}: served {served}, the trend says {expected}"
            );
        }
    }

    /// Only the spread moves: the levels the black point and the unlinked channels are
    /// measured from are the snapshot's.
    #[test]
    fn a_reused_set_keeps_its_levels() {
        let mut a = PreviewAnalysis::new();
        measure(&mut a, 16, 0.004);
        measure(&mut a, 25, 0.0032);
        let served = reuse(&mut a, 30);
        for channel in &served.channels {
            assert_eq!(channel.median, 0.05);
        }
    }

    /// A measurement that rose — a cloud, a bright sub joining the stack — is held, not
    /// extrapolated into noise that grows with depth; and nothing falls faster than
    /// averaging does, so a steeper drop is carried at `1/sqrt(N)`.
    #[test]
    fn the_trend_is_clamped_to_what_a_stack_can_do() {
        let mut rising = PreviewAnalysis::new();
        measure(&mut rising, 16, 0.004);
        measure(&mut rising, 25, 0.005);
        assert_eq!(reuse(&mut rising, 30).mean_sigma(), measured(0.005));

        let mut steep = PreviewAnalysis::new();
        measure(&mut steep, 16, 0.004);
        measure(&mut steep, 25, 0.002);
        let served = reuse(&mut steep, 30).mean_sigma();
        let expected = measured(0.002) * (25.0f32 / 30.0).sqrt();
        assert!((served / expected - 1.0).abs() < 1e-4, "{served} against {expected}");
    }

    /// A new stack owes nothing to the last one's trend: after a reset the first set is
    /// served as measured until a second measurement gives it a trend of its own.
    #[test]
    fn a_new_stack_starts_without_a_trend() {
        let mut a = PreviewAnalysis::new();
        measure(&mut a, 16, 0.004);
        measure(&mut a, 25, 0.002);

        // The stack restarted: depth went backwards.
        measure(&mut a, 5, 0.006);
        assert_eq!(reuse(&mut a, 6).mean_sigma(), measured(0.006));

        // Live view in between counts as a reset too.
        let mut b = PreviewAnalysis::new();
        measure(&mut b, 16, 0.004);
        let live = AnalysisContext {
            showing_stack: false,
            stack_depth: 0,
        };
        assert!(!begin(&mut b, live, &config()));
        b.stats(|| Ok(sky_stats(0.05, 0.01))).unwrap();
        measure(&mut b, 20, 0.003);
        assert_eq!(reuse(&mut b, 22).mean_sigma(), measured(0.003));
    }

    const CROP: CropRect = CropRect { x: 8, y: 0, width: 90, height: 100 };

    /// One frame of the stack at `depth` and size `dims`, cut to `crop`; whether it reused.
    fn frame_cut(a: &mut PreviewAnalysis, depth: u32, dims: (usize, usize), crop: Option<CropRect>) -> bool {
        a.note_crop(crop);
        a.begin_frame(stacked(depth), (dims.0, dims.1, 3), true, &config(), true, 1.0, true)
    }

    /// A new crop is the same stack through a new window: measured again, since every
    /// estimate described the old window, but the trend carries on — and its exponent is
    /// not re-derived from a pair either side of the cut, which saw the border's removal
    /// (here 15 % less noise) as the stack deepening.
    #[test]
    fn a_crop_change_measures_again_but_keeps_the_trend() {
        let mut a = PreviewAnalysis::new();
        measure(&mut a, 16, 0.004);
        let sigma_25 = 0.004 * (16.0f32 / 25.0).powf(0.3);
        measure(&mut a, 25, sigma_25);

        assert!(!frame_cut(&mut a, 26, (90, 100), Some(CROP)), "a new crop reused the old window's estimates");
        a.stats(|| Ok(sky_stats(0.05, sigma_25 * 0.85))).unwrap();

        for depth in [27u32, 28, 30] {
            assert!(frame_cut(&mut a, depth, (90, 100), Some(CROP)), "depth {depth} must reuse");
            let served = a.stats(|| panic!("a reusing frame must not measure")).unwrap().mean_sigma();
            let expected = measured(sigma_25 * 0.85) * (26.0 / depth as f32).powf(0.3);
            assert!((served / expected - 1.0).abs() < 1e-4, "depth {depth}: served {served}, the trend says {expected}");
        }
    }

    /// A stack plate solve landing mid-stack hands the background its catalog footprints:
    /// the model is built again with them, once, but it is the same stack — a dropped
    /// trend would jolt the stretch the moment the solve lands.
    #[test]
    fn a_landed_plate_solve_measures_again_but_keeps_the_trend() {
        use night_amplifier_core::background::{FieldProtection, ProtectedRegion};
        let mut a = PreviewAnalysis::new();
        measure(&mut a, 16, 0.004);
        let sigma_25 = 0.004 * (16.0f32 / 25.0).powf(0.3);
        measure(&mut a, 25, sigma_25);

        let footprint = ProtectedRegion { cx: 50.0, cy: 50.0, semi_major: 10.0, semi_minor: 10.0, angle: 0.0 };
        let solved = |generation| Some(Arc::new(FieldProtection { generation, width: 100, height: 100, regions: vec![footprint] }));
        let with = |protection: Option<Arc<FieldProtection>>| BackgroundConfig { protection, ..config() };

        assert!(!begin(&mut a, stacked(26), &with(solved(1))), "the model ignored the footprints");
        a.stats(|| Ok(sky_stats(0.05, sigma_25))).unwrap();
        for depth in [27u32, 30] {
            assert!(begin(&mut a, stacked(depth), &with(solved(1))), "depth {depth} must reuse");
            let served = a.stats(|| panic!("a reusing frame must not measure")).unwrap().mean_sigma();
            let expected = measured(sigma_25) * (26.0 / depth as f32).powf(0.3);
            assert!((served / expected - 1.0).abs() < 1e-4, "depth {depth}: served {served}, the trend says {expected}");
        }
        assert!(!begin(&mut a, stacked(31), &with(solved(2))), "a new solve must rebuild the model");
    }

    /// Only a crop excuses a size change: a resolution change under the same crop is a new
    /// image, as before.
    #[test]
    fn a_size_change_under_an_unchanged_crop_still_resets() {
        let mut a = PreviewAnalysis::new();
        assert!(!frame_cut(&mut a, 100, (90, 100), Some(CROP)));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        assert!(frame_cut(&mut a, 101, (90, 100), Some(CROP)));
        assert!(!frame_cut(&mut a, 102, (45, 50), Some(CROP)), "binning changed the frame under the model");
    }

    /// A crop change never excuses anything else: a settings change at the same moment
    /// still drops the trend.
    #[test]
    fn a_crop_change_with_a_settings_change_is_a_new_stack() {
        let mut a = PreviewAnalysis::new();
        measure(&mut a, 16, 0.004);
        measure(&mut a, 25, 0.002);
        a.note_crop(Some(CROP));
        let mut changed = config();
        changed.aggressiveness += 0.1;
        assert!(!a.begin_frame(stacked(26), (90, 100, 3), true, &changed, true, 1.0, true));
        a.stats(|| Ok(sky_stats(0.05, 0.003))).unwrap();
        a.note_crop(Some(CROP));
        assert!(a.begin_frame(stacked(27), (90, 100, 3), true, &changed, true, 1.0, true));
        assert_eq!(a.stats(|| panic!("must reuse")).unwrap().mean_sigma(), measured(0.003), "the old trend survived");
    }
}

#[cfg(test)]
#[path = "analysis_stability_tests.rs"]
mod stability_tests;
