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
use night_amplifier_core::frame::Frame;
use night_amplifier_core::statistics::ImageStats;
use night_amplifier_core::stacking::CropRect;

#[path = "analysis_noise_trend.rs"]
mod noise_trend;
use noise_trend::NoiseTrend;

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
    edge_profiles: bool,
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
            edge_profiles: background.edge_profiles,
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

/// Pixels a moved crop's probe reads on each side, a share of the rows: every one of them is
/// measured, so where the two crops overlap both sides read the same pixels and the ratio moves
/// only with what the window added or cut.
const PROBE_PIXELS: usize = 400_000;

/// The rows of a `width x height` cut a probe reads: every `stride`-th, aligned to the stack's
/// row grid (rows `y0 + first + k * stride`, `y0` the cut's first row in the stack, binned) so
/// two crops of one stack read the same rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeRows {
    pub first: usize,
    pub stride: usize,
}

impl ProbeRows {
    fn of(y0: usize, width: usize, height: usize) -> Self {
        Self::aligned(y0, (width * height / PROBE_PIXELS).max(1))
    }

    fn aligned(y0: usize, stride: usize) -> Self {
        Self { first: (stride - y0 % stride) % stride, stride }
    }

    /// How many rows of a `height`-row cut are read.
    fn count(&self, height: usize) -> usize {
        height.saturating_sub(self.first).div_ceil(self.stride)
    }
}

/// A moved crop measured two-sided, through the estimates the crop it left was measured with:
/// the stack's rows there (`left`, [`PreviewAnalysis::note_left_window`]) and the frame's own
/// rows (`entered` of it, the crop it moved to). Their noise ratio is what the window changed.
#[derive(Debug)]
pub struct WindowProbe {
    pub white_balance: Option<[f32; 3]>,
    pub left: Frame,
    /// The last model on `left`'s rows.
    pub left_background: Option<BackgroundModel>,
    pub entered: ProbeRows,
    /// The last model on the frame's `entered` rows.
    pub entered_background: Option<BackgroundModel>,
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
    /// The noise window the first stack starts from ([`Self::through_noise_window`]).
    seeded_window: Option<f32>,
    /// The stack crop of the frame being analysed — see [`Self::note_crop`].
    crop: Option<CropRect>,
    /// `crop` moved since the last frame.
    crop_changed: bool,
    /// The crop it moved from, when it moved from one crop to another (not to or from none).
    moved_from: Option<CropRect>,
    /// The stack's rows on `moved_from` ([`Self::note_left_window`]).
    left: Option<Frame>,
    /// The last window's estimates, for the frame cut to a moved crop to be measured through
    /// once ([`Self::take_window_probe`]).
    probe: Option<WindowProbe>,
    /// The trend waits for [`Self::probe_window`] before the frame's refit is measured.
    probe_pending: bool,
    /// The last [`Self::note_stack`] generation, and whether it changed since the last frame.
    generation: u64,
    rebuilt: bool,
    /// The stack's catalog footprints on the frame being analysed — see
    /// [`Self::set_protection`].
    protection: Option<Arc<FieldProtection>>,
}

impl PreviewAnalysis {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh analysis serving its noise figure through `window`: a one-shot render of a
    /// stack the render task carried there ([`Self::noise_window`]), so the two agree.
    pub fn through_noise_window(window: f32) -> Self {
        Self { seeded_window: Some(window), ..Self::default() }
    }

    /// What this stack's noise figure is served at, after the crop moves it has seen
    /// (`NoiseTrend::move_window`); 1 for a stack whose crop never moved.
    pub fn noise_window(&self) -> f32 {
        self.trend.window()
    }

    /// The crop the render task cut the next frame to (`StackedFrame::crop`), before
    /// [`Self::begin_frame`].
    ///
    /// A crop change is the same stack seen through a new window: its estimates describe the
    /// old one and are measured again, but the `NoiseTrend` carries on, and from one crop to
    /// another keeps the noise figure where it had it (`NoiseTrend::move_window`). Treated as a
    /// new stack, every change (one per 3-8 subs under drift) dropped the trend the stacked
    /// view holds still with.
    pub fn note_crop(&mut self, crop: Option<CropRect>) {
        self.crop_changed = self.crop != crop;
        self.moved_from = self.crop.filter(|_| self.crop_changed && crop.is_some());
        self.crop = crop;
        self.left = None;
    }

    /// Before [`Self::note_crop`]: when `crop` moves from the crop this analysis last measured
    /// to another, the stack's rows its probe reads there, cut and binned by `bin` as the
    /// render task cuts a frame (`stack_crop::cut_rows(stack, rect, bin, stride)`). Hand them
    /// to [`Self::note_left_window`] after `note_crop`.
    pub fn left_window(&self, crop: Option<CropRect>, bin: usize) -> Option<(CropRect, usize)> {
        let (old, bin) = (self.crop?, bin.max(1));
        if crop? == old {
            return None;
        }
        let rows = ProbeRows::of(old.y / bin, old.width / bin, old.height / bin);
        let skipped = rows.first * bin;
        let rect = CropRect { y: old.y + skipped, height: old.height.checked_sub(skipped)?, ..old };
        Some((rect, rows.stride))
    }

    /// The stack's rows [`Self::left_window`] named, after [`Self::note_crop`]: the side of the
    /// move the frame no longer shows.
    pub fn note_left_window(&mut self, rows: Frame) {
        self.left = self.moved_from.is_some().then_some(rows);
    }

    /// The stack's `StackedFrame::stack_generation`, before [`Self::begin_frame`]: a stack
    /// rebuilt in place is another stack whatever its depth says, and a probation's commit
    /// can keep or raise it, which [`Self::begin_frame`] alone reads as the same stack.
    pub fn note_stack(&mut self, generation: u64) {
        self.rebuilt |= self.generation != generation;
        self.generation = generation;
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
        let moved_from = self.moved_from.take();
        let left = self.left.take();
        let rebuilt = std::mem::take(&mut self.rebuilt);
        let protection_changed = self.cached.as_ref().is_some_and(|c| c.key.protection != key.protection);
        let same_stack = ctx.showing_stack && !rebuilt && self.continues_stack(ctx);
        let same_key = self.same_key(&key, crop_changed, protection_changed);
        let remeasure = crop_changed || protection_changed || !same_key;
        let probe = moved_from
            .zip(left)
            .filter(|_| same_stack && same_key && !protection_changed)
            .and_then(|(old, left)| self.carry(old, left, &key));
        self.reuse = same_stack && !remeasure && self.is_current(ctx);
        self.depth = ctx.stack_depth;

        if !same_stack {
            self.trend = self.seeded_window.take().map_or_else(NoiseTrend::default, NoiseTrend::through);
        } else if crop_changed && moved_from.is_none() {
            self.trend.unwindow();
        } else if remeasure && probe.is_none() {
            self.trend.rebase();
        }
        self.probe_pending = probe.is_some();
        self.probe = probe;

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

    /// The white balance and background model the last window was measured with, on both
    /// sides of a move at the same bin: the stack's `left` rows of the crop it left, and the
    /// frame's rows of the crop it entered (`BackgroundModel::windowed`). Both sides are one
    /// stack at one depth through one set of estimates, so their ratio is the window's alone;
    /// a one-sided probe against the trend booked the trend's error into the window at every
    /// move (0.61 by 300 subs once its exponent froze). Refitted there, M33's levels moved
    /// 2 ADU and its MAD -33 %: the frame still refits on schedule, the probe only sets the
    /// window. `None` for a window too far past the last one, or rows that are not the
    /// last window's as [`Self::left_window`] named them.
    fn carry(&self, old: CropRect, left: Frame, key: &AnalysisKey) -> Option<WindowProbe> {
        let cached = self.cached.as_ref()?;
        let new = self.crop?;
        let bin = |rect: &CropRect, width: usize| (width > 0).then(|| (rect.width + width / 2) / width).filter(|&b| b >= 1);
        let b = bin(&new, key.dimensions.0)?;
        if bin(&old, cached.key.dimensions.0)? != b {
            return None;
        }
        let ((old_w, old_h, channels), (new_w, new_h, _)) = (cached.key.dimensions, key.dimensions);
        let left_rows = ProbeRows::of(old.y / b, old_w, old_h);
        if (left.width(), left.height(), left.channels()) != (old_w, left_rows.count(old_h), channels) {
            return None;
        }
        let entered = ProbeRows::aligned(new.y / b, left_rows.stride);
        let offset = |new: usize, old: usize| (new as isize - old as isize).div_euclid(b as isize);
        let (x, y) = (offset(new.x, old.x), offset(new.y, old.y));
        let on_rows = |model: &BackgroundModel, x: isize, y: isize, (w, h): (usize, usize), rows: ProbeRows| {
            let skipped = rows.first.min(h);
            Some(model.windowed(x, y + skipped as isize, w, h.checked_sub(skipped).filter(|&h| h > 0)?)?.every_nth_row(rows.stride))
        };
        let (left_background, entered_background) = match (&cached.background, key.background_subtraction) {
            (Some(model), true) => (
                Some(on_rows(model, 0, 0, (old_w, old_h), left_rows)?),
                Some(on_rows(model, x, y, (new_w, new_h), entered)?),
            ),
            (None, false) => (None, None),
            _ => return None,
        };
        Some(WindowProbe { white_balance: cached.white_balance, left, left_background, entered, entered_background })
    }

    /// The last window's estimates through a moved crop, once, before the frame is touched:
    /// measure the frame through them and hand the result to [`Self::probe_window`].
    pub fn take_window_probe(&mut self) -> Option<WindowProbe> {
        self.probe.take()
    }

    /// The noise [`Self::take_window_probe`]'s two sides read, `(left, entered)` (`None`: it
    /// could not be measured), which sets the window the frame's refit is served through.
    pub fn probe_window(&mut self, sigmas: Option<(f32, f32)>) {
        if std::mem::take(&mut self.probe_pending) {
            match sigmas {
                Some((left, entered)) => self.trend.move_window(left, entered),
                None => self.trend.rebase(),
            }
        }
    }

    /// The stored estimates were taken from an earlier frame of this same stack: one has been
    /// measured, and `stack_depth` has not gone *down*, which is a reset.
    fn continues_stack(&self, ctx: AnalysisContext) -> bool {
        self.cached.as_ref().is_some_and(|cached| cached.depth != 0 && ctx.stack_depth >= cached.depth)
    }

    /// ...under this frame's key. `crop_changed` lets the dimensions differ, since a new crop
    /// is what changed them; `protection_changed` the protection, since a solve landing is what
    /// changed it. Another key (a setting, the Processing Resolution) is the same stack measured
    /// again: the trend and its window carry on, or the steps the window held back (0.81 on the
    /// 106-sub dumbbell) landed on one frame.
    fn same_key(&self, key: &AnalysisKey, crop_changed: bool, protection_changed: bool) -> bool {
        self.cached.as_ref().is_some_and(|cached| {
            let comparable = AnalysisKey {
                dimensions: if crop_changed { key.dimensions } else { cached.key.dimensions },
                protection: if protection_changed { key.protection } else { cached.key.protection },
                ..cached.key
            };
            comparable == *key
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
        let mut value = compute()?;
        self.probe_window(None);
        if let Some(cached) = self.cached.as_mut() {
            value = value.with_noise_scaled(self.trend.observe(cached.depth, value.mean_sigma()));
            cached.stats = Some(value.clone());
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

    /// A probation's commit rebuilds the stack at the depth it had: only the generation
    /// tells the render task the estimates describe another stack, and only once.
    #[test]
    fn a_stack_rebuilt_at_its_depth_invalidates_once() {
        let mut a = PreviewAnalysis::new();
        let cfg = config();
        a.note_stack(0);
        assert!(!begin(&mut a, stacked(6), &cfg));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        a.note_stack(0);
        assert!(begin(&mut a, stacked(6), &cfg), "the same stack reuses");

        a.note_stack(1);
        assert!(!begin(&mut a, stacked(6), &cfg), "the rebuilt stack reused the old one's estimates");
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        a.note_stack(1);
        assert!(begin(&mut a, stacked(7), &cfg), "the rebuilt stack is measured once, then reused");
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

    /// One frame of the stack at `depth` and size `dims`, cut to `crop` as the render task
    /// cuts it (`render_task::cut_for_render`, unbinned): a moved crop hands over the stack's
    /// rows on the crop it left. Whether it reused.
    fn frame_cut(a: &mut PreviewAnalysis, depth: u32, dims: (usize, usize), crop: Option<CropRect>) -> bool {
        let left = a.left_window(crop, 1).map(|(rect, stride)| Frame::filled(rect.width, rect.height.div_ceil(stride), 3, 0.5).unwrap());
        a.note_crop(crop);
        if let Some(rows) = left {
            a.note_left_window(rows);
        }
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

    /// A model over a `w x h` frame, sloped so that where a window sits changes what it subtracts.
    fn model(w: usize, h: usize) -> BackgroundModel {
        BackgroundModel::new(vec![vec![0.0, 0.1, 0.2, 0.3]; 3], 2, 2, w, h, 3, false, 0.1, 1.0)
    }

    /// `model`'s subtraction from a flat `w x h` frame.
    fn subtracted_by(model: &BackgroundModel, (w, h): (usize, usize)) -> Vec<f32> {
        let mut frame = Frame::filled(w, h, 3, 0.5).unwrap();
        model.subtract_from(&mut frame);
        frame.data().to_vec()
    }

    const MOVED: CropRect = CropRect { x: 12, y: 4, width: 84, height: 92 };

    /// The stack measured at 16 and 25 subs, then cut to `CROP` at 26 with its estimates
    /// taken there: a white balance, `model(90, 100)` and `sigma` at 26.
    fn cropped_at_26(sigma: f32) -> PreviewAnalysis {
        let mut a = PreviewAnalysis::new();
        measure(&mut a, 16, 0.004);
        measure(&mut a, 25, 0.004 * (16.0f32 / 25.0).powf(0.3));
        assert!(!frame_cut(&mut a, 26, (90, 100), Some(CROP)));
        assert!(a.take_window_probe().is_none(), "a crop starting has no last window");
        a.white_balance(|| Ok([1.1, 1.0, 0.9])).unwrap();
        a.background(|| Ok(model(90, 100))).unwrap();
        a.stats(|| Ok(sky_stats(0.05, sigma))).unwrap();
        a
    }

    /// A crop moving cuts covered sky, not thin border: the frame MAD, which reads the
    /// target's structure too, rose 14 % at one such move on the 106-sub dumbbell. The stack
    /// is measured on both crops through the last window's estimates, and their ratio sets the
    /// window: the figure stays where it was, and the frame's own refit is served through it.
    #[test]
    fn a_moved_crop_keeps_the_noise_figure_where_it_was() {
        let mut a = cropped_at_26(0.0034);
        assert!(!frame_cut(&mut a, 30, (84, 92), Some(MOVED)), "a moved crop reused the old window's estimates");
        let probe = a.take_window_probe().expect("the last window's estimates");
        assert_eq!(probe.white_balance, Some([1.1, 1.0, 0.9]));
        assert_eq!((probe.left.width(), probe.left.height()), (90, 100), "the stack's rows on the crop it left");
        assert_eq!(probe.entered, ProbeRows { first: 0, stride: 1 });
        let (left, entered) = (probe.left_background.unwrap(), probe.entered_background.unwrap());
        assert_eq!(subtracted_by(&left, (90, 100)), subtracted_by(&model(90, 100), (90, 100)), "the last model where it was fitted");
        let moved = subtracted_by(&entered, (84, 92));
        assert_eq!(moved, subtracted_by(&model(90, 100).windowed(4, 4, 84, 92).unwrap(), (84, 92)), "the last model at the new window");
        assert_ne!(moved, subtracted_by(&model(90, 100).windowed(0, 0, 84, 92).unwrap(), (84, 92)), "the window's place matters");
        assert!(a.take_window_probe().is_none(), "one probe a move");
        a.probe_window(Some((0.0031, 0.0031 * 1.14)));
        let mut refits = 0;
        a.background(|| {
            refits += 1;
            Ok(model(84, 92))
        })
        .unwrap();
        assert_eq!(refits, 1, "the frame refits on schedule, the probe only sets the window");
        let window = 0.0031 / (0.0031 * 1.14);
        let served = a.stats(|| Ok(sky_stats(0.05, 0.0034 * 1.14 * 0.9))).unwrap();
        let expected = measured(0.0034 * 1.14 * 0.9) * window;
        assert!((served.mean_sigma() / expected - 1.0).abs() < 1e-4, "served {}, expected {expected}", served.mean_sigma());
        assert_eq!(served.channels[0].median, 0.05, "the levels are the new window's");
        assert_eq!(a.noise_window(), window);

        // The refit's 10 % is its own step: not bridged, not read as the stack deepening.
        assert!(frame_cut(&mut a, 32, (84, 92), Some(MOVED)));
        let held = a.stats(|| panic!("a reusing frame must not measure")).unwrap().mean_sigma();
        let along = served.mean_sigma() * (30.0f32 / 32.0).powf(0.3);
        assert!((held / along - 1.0).abs() < 1e-4, "{held} against {along}");

        // Switched off, the crop's border comes back as measured.
        assert!(!frame_cut(&mut a, 33, (100, 100), None));
        let whole = a.stats(|| Ok(sky_stats(0.05, 0.003))).unwrap().mean_sigma();
        assert_eq!(whole, measured(0.003));
    }

    /// A probe reads about `PROBE_PIXELS` a side, on every `stride`-th row of the stack's grid:
    /// the crop it left and the one it entered read the same stack rows, binned or not.
    #[test]
    fn a_move_s_two_sides_read_the_same_stack_rows() {
        let (from, to) = (CropRect { x: 0, y: 8, width: 4000, height: 2000 }, CropRect { x: 16, y: 24, width: 3968, height: 1968 });
        for bin in [1usize, 2] {
            let (w, h) = (from.width / bin, from.height / bin);
            let mut a = PreviewAnalysis::new();
            a.note_crop(Some(from));
            a.begin_frame(stacked(40), (w, h, 3), true, &config(), true, 1.0, true);
            a.white_balance(|| Ok([1.0; 3])).unwrap();
            a.background(|| Ok(model(w, h))).unwrap();
            a.stats(|| Ok(sky_stats(0.05, 0.003))).unwrap();

            let (rect, stride) = a.left_window(Some(to), bin).expect("a move");
            assert_eq!(stride, (w * h / PROBE_PIXELS).max(1), "bin {bin}");
            assert!(stride > 1 && (rect.y / bin).is_multiple_of(stride), "bin {bin}: rows from {} every {stride}", rect.y);
            let rows = Frame::filled(w, (rect.height / bin).div_ceil(stride), 3, 0.5).unwrap();
            a.note_crop(Some(to));
            a.note_left_window(rows);
            a.begin_frame(stacked(41), (to.width / bin, to.height / bin, 3), true, &config(), true, 1.0, true);
            let probe = a.take_window_probe().expect("both sides");
            assert_eq!(probe.entered.stride, stride);
            assert!((to.y / bin + probe.entered.first).is_multiple_of(stride), "bin {bin}: {:?}", probe.entered);
        }
    }

    /// A moved crop measured as any refit, as before: one whose left rows never came (the cut
    /// failed), were not the last window's, or whose frame was never probed (auto stretch off).
    #[test]
    fn a_moved_crop_never_probed_is_measured_as_before() {
        let mut a = cropped_at_26(0.003);
        a.note_crop(Some(MOVED));
        assert!(!a.begin_frame(stacked(30), (84, 92, 3), true, &config(), true, 1.0, true));
        assert!(a.take_window_probe().is_none(), "no rows of the crop it left");
        assert_eq!(a.stats(|| Ok(sky_stats(0.05, 0.0034))).unwrap().mean_sigma(), measured(0.0034));

        let mut a = cropped_at_26(0.003);
        a.note_crop(Some(MOVED));
        a.note_left_window(Frame::filled(90, 50, 3, 0.5).unwrap());
        a.begin_frame(stacked(30), (84, 92, 3), true, &config(), true, 1.0, true);
        assert!(a.take_window_probe().is_none(), "rows that are not the last window's");

        let mut a = cropped_at_26(0.003);
        assert!(!frame_cut(&mut a, 30, (84, 92), Some(MOVED)));
        assert_eq!(a.stats(|| Ok(sky_stats(0.05, 0.0034))).unwrap().mean_sigma(), measured(0.0034));
    }

    /// A crop that moves far past the last one cannot carry the model: it is refitted, and
    /// its step is served as measured, as before.
    #[test]
    fn a_crop_moving_out_of_the_last_is_measured_again() {
        const SHIFTED: CropRect = CropRect { x: 0, y: 0, width: 90, height: 100 };
        let mut a = cropped_at_26(0.003);
        assert!(!frame_cut(&mut a, 27, (90, 100), Some(SHIFTED)));
        assert!(a.take_window_probe().is_none(), "the model reached 8 px past a 90 px window");
        assert_eq!(a.stats(|| Ok(sky_stats(0.05, 0.0034))).unwrap().mean_sigma(), measured(0.0034));
    }

    /// A one-shot render of a stack the render task carried through a moved crop serves its
    /// noise figure at that window; a stack after it starts from its own.
    #[test]
    fn a_seeded_window_serves_the_first_stack_only() {
        let mut a = PreviewAnalysis::through_noise_window(0.8);
        measure(&mut a, 40, 0.004);
        assert_eq!(reuse(&mut a, 41).mean_sigma(), measured(0.004) * 0.8);
        assert_eq!(a.noise_window(), 0.8);
        measure(&mut a, 5, 0.006);
        assert_eq!(reuse(&mut a, 6).mean_sigma(), measured(0.006));
    }

    /// A size change under an unchanged crop (the Processing Resolution) is the frame measured
    /// again: nothing is reused.
    #[test]
    fn a_size_change_under_an_unchanged_crop_measures_again() {
        let mut a = PreviewAnalysis::new();
        assert!(!frame_cut(&mut a, 100, (90, 100), Some(CROP)));
        let _ = a.white_balance(|| Ok([1.0, 1.0, 1.0]));
        assert!(frame_cut(&mut a, 101, (90, 100), Some(CROP)));
        assert!(!frame_cut(&mut a, 102, (45, 50), Some(CROP)), "binning changed the frame under the model");
    }

    /// A settings change at a crop change measures again, but it is the same stack: the trend
    /// carries on (its exponent, here clamped to 0.5, and no exponent across the change).
    #[test]
    fn a_settings_change_at_a_crop_change_keeps_the_trend() {
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
        let served = a.stats(|| panic!("must reuse")).unwrap().mean_sigma();
        let along = measured(0.003) * (26.0f32 / 27.0).sqrt();
        assert!((served / along - 1.0).abs() < 1e-5, "{served} against {along}");
    }

    /// The stack cut to `MOVED` at 30 through a window the move set to `1 / 1.14`.
    fn moved_at_30() -> PreviewAnalysis {
        let mut a = cropped_at_26(0.0034);
        assert!(!frame_cut(&mut a, 30, (84, 92), Some(MOVED)));
        let _ = a.take_window_probe().unwrap();
        a.probe_window(Some((0.0031, 0.0031 * 1.14)));
        a.background(|| Ok(model(84, 92))).unwrap();
        a.stats(|| Ok(sky_stats(0.05, 0.0034 * 1.14))).unwrap();
        assert!((a.noise_window() - 1.0 / 1.14).abs() < 1e-6);
        a
    }

    /// A settings change mid-stack (aggressiveness, or the Processing Resolution binning the
    /// frame, at an unmoved crop) keeps the window the crop moves built: dropped, every step
    /// they kept out of the figure landed on one frame (19 % here, 23 % at the 106-sub
    /// dumbbell's 0.81).
    #[test]
    fn a_settings_change_keeps_the_window_the_crop_moves_built() {
        let mut changed = config();
        changed.aggressiveness += 0.1;
        for (what, config, dims) in [("aggressiveness", changed, (84, 92)), ("binning", config(), (42, 46))] {
            let mut a = moved_at_30();
            let window = a.noise_window();
            a.note_crop(Some(MOVED));
            assert!(!a.begin_frame(stacked(31), (dims.0, dims.1, 3), true, &config, true, 1.0, true), "{what}");
            a.background(|| Ok(model(dims.0, dims.1))).unwrap();
            let served = a.stats(|| Ok(sky_stats(0.05, 0.0034 * 1.14))).unwrap().mean_sigma();
            let step = served / (measured(0.0034 * 1.14) * window) - 1.0;
            assert!(step.abs() < 1e-5, "{what} stepped the noise figure {:.1} % (window now {:.3})", 100.0 * step, a.noise_window());
        }
    }

    /// The window compounds at every move, so nothing but the window may enter a move's step.
    /// Crops that change nothing, a move per `every` subs (4-8 on the fixtures) from
    /// `MIN_CROP_DEPTH` (20), the 106-sub IMX533 set's own fall (`N^-0.41` to 32 subs, then
    /// `N^-0.19`), `jitter_share` on every reading of the stack. Returns the moves and the
    /// window's range. Against the trend's carry, the frozen exponent was booked into the
    /// window at every move: 0.53-0.64 by 300 subs at either cadence, with or without jitter.
    fn walk_the_window(jitter_share: f32, every: u32) -> (u32, (f32, f32)) {
        const A: CropRect = CropRect { x: 8, y: 4, width: 84, height: 92 };
        const B: CropRect = CropRect { x: 10, y: 4, width: 84, height: 92 };
        let mut seed = 0x9E37_79B9u32;
        let mut jitter = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            1.0 + jitter_share * ((seed % 2001) as f32 / 1000.0 - 1.0)
        };
        let truth = |depth: u32| {
            let d = depth as f32;
            if d <= 32.0 { 0.01 * d.powf(-0.41) } else { 0.01 * 32f32.powf(-0.41) * (32.0 / d).powf(0.19) }
        };
        let mut a = PreviewAnalysis::new();
        let (mut extremes, mut moves) = ((1.0f32, 1.0f32), 0);
        for depth in 1..=300u32 {
            let crop = match depth {
                ..20 => None,
                _ if (depth / every).is_multiple_of(2) => Some(A),
                _ => Some(B),
            };
            if !frame_cut(&mut a, depth, (84, 92), crop) {
                if a.take_window_probe().is_some() {
                    moves += 1;
                    // Two crops that change nothing read one stack the same on both sides.
                    let reading = measured(truth(depth) * jitter());
                    a.probe_window(Some((reading, reading)));
                }
                a.background(|| Ok(model(84, 92))).unwrap();
                a.stats(|| Ok(sky_stats(0.05, truth(depth) * jitter()))).unwrap();
            }
            let w = a.noise_window();
            extremes = (extremes.0.min(w), extremes.1.max(w));
        }
        (moves, extremes)
    }

    #[test]
    fn moves_that_change_nothing_leave_the_window_at_one() {
        for (jitter, every) in [(0.0, 4), (0.02, 4), (0.0, 8), (0.02, 8)] {
            let (moves, (low, high)) = walk_the_window(jitter, every);
            assert!(moves >= 35, "{moves} moves");
            assert_eq!((low, high), (1.0, 1.0), "±{:.0} %, a move per {every} subs: {moves} moves walked it", 100.0 * jitter);
        }
    }
}

#[cfg(test)]
#[path = "analysis_stability_tests.rs"]
mod stability_tests;
