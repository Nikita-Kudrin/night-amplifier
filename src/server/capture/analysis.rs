//! Cross-frame reuse of the preview pipeline's estimates. `process_preview_frame`
//! computes three *statistical descriptions of the stack*, not the frame itself:
//! white-balance multipliers, the background model, and the per-channel median/MAD
//! the stretch solves against. The frame they describe moves by only 1/N between
//! renders (a running mean over N subs), so recomputing all three every frame is
//! most of the render thread's linear cost — 6.4ms of `process_preview_frame`'s
//! 11.2ms on an IMX464-shaped frame with Community's bilinear background (Pro's RBF
//! costs several times more). Everything touching pixels stays uncached —
//! neutralisation, model subtraction, black point all still run every frame.
//!
//! Invalidated by: **live view** (`showing_stack` false, nothing to reuse), **a
//! settings change** ([`AnalysisKey`] fingerprints every setting read, as bit
//! patterns so NaN can't pin the cache open), **a shape change** (binning/ROI/
//! superpixel), and **stack growth** — MAD falls as `1/sqrt(N)`, so what matters is
//! *relative* change in N (1->2 subs halves noise, 140->141 moves nothing).
//! [`DEPTH_GROWTH`] refreshes on proportional growth; [`MAX_AGE_FRAMES`] caps reuse
//! regardless, so a stalled stack still refreshes against a moving sky.

use crate::background::{BackgroundConfig, BackgroundExtractionAlgorithm, BackgroundModel};
use crate::error::Result;
use crate::statistics::ImageStats;

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
/// they were taken, which is a ~12 % change in MAD. Below that the black point moves by
/// less than the dither already applied at the 8-bit boundary.
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
}

impl PreviewAnalysis {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide once, for this frame, whether the stored estimates still describe it.
    ///
    /// Called before any of the three getters, and the decision is shared by all of
    /// them: mixing a fresh background model with a stale set of statistics would
    /// measure the sky against a gradient that had already been removed differently.
    ///
    /// Returns whether anything will be reused, for the caller's span.
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

        self.reuse = ctx.showing_stack && self.can_reuse(&key, ctx);

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

    fn can_reuse(&self, key: &AnalysisKey, ctx: AnalysisContext) -> bool {
        let Some(cached) = self.cached.as_ref() else {
            return false;
        };
        if cached.key != *key || cached.age >= MAX_AGE_FRAMES {
            return false;
        }
        // A stack that has not been measured yet, or that restarted, has nothing to
        // compare against — `stack_depth` going *down* is a reset.
        if cached.depth == 0 || ctx.stack_depth < cached.depth {
            return false;
        }
        (ctx.stack_depth as f32) < cached.depth as f32 * DEPTH_GROWTH
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
                crate::error::StackError::InvalidConfiguration(
                    "background model missing after computation".into(),
                )
            })
    }

    /// Per-channel statistics, computing them if this frame cannot reuse the stored set.
    pub fn stats<F>(&mut self, compute: F) -> Result<ImageStats>
    where
        F: FnOnce() -> Result<ImageStats>,
    {
        if self.reuse {
            if let Some(value) = self.cached.as_ref().and_then(|c| c.stats.clone()) {
                return Ok(value);
            }
        }
        let value = compute()?;
        if let Some(cached) = self.cached.as_mut() {
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
}
