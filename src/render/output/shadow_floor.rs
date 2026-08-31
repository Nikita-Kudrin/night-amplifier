//! The darkening half of the black floor: how the sky gets to black.
//!
//! # Why this is not the pedestal
//!
//! [`super::DisplayOutput::pedestal`] lifts the output into `[pedestal, 1]` so
//! no pixel reaches an OLED's off state. This does the opposite, and it is a
//! different transform rather than the same one with the sign flipped: the
//! pedestal is a property of the panel, so it is an absolute fraction of full
//! scale, while the floor is a property of the *sky*, so it is a fraction of
//! wherever the sky actually landed.
//!
//! # Why a fraction of the sky rather than of full scale
//!
//! The autostretch pins the sky at `target_background` before anything here
//! runs, so a fixed absolute floor tracks reasonably well across targets — 5.5 %
//! of full scale measured 79 % and 71 % darker on the two fixtures. It only
//! tracks reasonably, though: the two figures differ because the two skies
//! landed at 14 and 17 output levels. Anchoring to the measured level instead
//! makes one slider position mean one thing, which is what lets the setting
//! survive a change of target or of sky brightness.
//!
//! # Why a knee
//!
//! Sky noise is as wide as the sky level itself — sigma is 5.2 and 10.2 output
//! levels against medians of 14 and 17 — so a hard black point puts 38-40 % of
//! all samples on exactly zero, which at eyepiece magnification is the same
//! black speckle the pedestal exists to remove. The softplus knee is linear
//! above the floor and compresses exponentially below it, so the sub-floor half
//! of the sky lands in a narrow dark band instead of on zero: measured 0.00 %
//! of samples at zero, against 38 % for the hard form, at the same sky level.
//!
//! The hard form is still reachable — it buys the deepest sky and slightly more
//! target-to-sky separation — which is what the "Darker sky" setting selects.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;

/// The knee width, as a fraction of the sky level.
///
/// Scaled with the sky rather than fixed so the curve keeps its shape when the
/// anchor moves; a fixed width would sharpen into a hard clip under a bright
/// sky and smear into a plain dim under a dark one.
const KNEE_FRACTION: f32 = 0.15;

/// The deepest floor that still leaves a usable range above it.
const MAX_DEPTH: f32 = 0.5;

/// Where black sits below the sky, and how sharply the image gets there.
///
/// Both fields are absolute output-referred levels. Build one with
/// [`ShadowFloor::from_sky`] rather than by hand — the anchoring to the measured
/// sky level is the whole point, and a literal loses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowFloor {
    /// Output level that maps to black, in `[0, 0.5]`. Zero disables.
    pub depth: f32,
    /// Softplus knee width. Zero clips at `depth` instead of rolling off into
    /// it, which is the only way to reach a true zero output.
    pub knee: f32,
}

impl Default for ShadowFloor {
    fn default() -> Self {
        Self::NONE
    }
}

impl ShadowFloor {
    /// No darkening: [`ShadowFloor::apply`] is the identity.
    pub const NONE: Self = Self {
        depth: 0.0,
        knee: 0.0,
    };

    /// A floor `fraction` of the way down from the measured sky level.
    ///
    /// `fraction` is the slider: `0.0` disables, `1.0` puts the floor exactly at
    /// the sky's own level, so half its noise falls under the knee. `hard`
    /// selects the clipping form.
    pub fn from_sky(fraction: f32, sky_level: f32, hard: bool) -> Self {
        let depth = (fraction.max(0.0) * sky_level.max(0.0)).min(MAX_DEPTH);
        if depth <= 0.0 {
            return Self::NONE;
        }
        Self {
            depth,
            knee: if hard { 0.0 } else { KNEE_FRACTION * depth },
        }
    }

    /// True when this transform is the identity, letting callers skip it.
    #[inline]
    pub fn is_none(&self) -> bool {
        self.depth <= 0.0
    }

    /// Map one output-referred value through the floor.
    ///
    /// `apply(0.0) == 0.0` and `apply(1.0) == 1.0` for every configuration.
    /// White staying white is what keeps star cores at 255 while the sky moves;
    /// black staying black is what lets this be composed into a *scale* table,
    /// which stores `curve(L) / L` and cannot represent a curve that misses the
    /// origin — the ratio diverges, and the entry meant to hold its limit ends
    /// up scaling the faintest bin toward white instead.
    ///
    /// The softplus does not pass through the origin on its own, so its value at
    /// zero is subtracted before normalizing. That costs nothing visible: the
    /// only input it moves to exactly zero is exactly zero, which is where the
    /// autostretch black point already put those pixels, and the pedestal lifts
    /// them off the panel's off state afterwards regardless.
    #[inline]
    pub fn apply(&self, y: f32) -> f32 {
        if self.is_none() {
            return y;
        }
        if self.knee <= 0.0 {
            return ((y - self.depth) / (1.0 - self.depth)).max(0.0);
        }
        let base = self.softplus(0.0);
        (self.softplus(y) - base) / (self.softplus(1.0) - base)
    }

    /// The `y → 0` limit of `apply(y) / y`.
    ///
    /// For callers composing this into a scale table whose entry 0 holds that
    /// limit rather than a sample of the curve — see `render::stretch`. The hard
    /// form's limit is genuinely zero: everything under the floor is black, and
    /// that is what the setting asks for.
    #[inline]
    pub fn slope_at_zero(&self) -> f32 {
        if self.is_none() {
            return 1.0;
        }
        if self.knee <= 0.0 {
            return 0.0;
        }
        // d/dy softplus(y) is the logistic sigmoid of the same argument. A knee
        // narrow enough to overflow the exp saturates the sigmoid to zero, which
        // is the hard form's answer and so is faithful rather than merely safe.
        let sigmoid = 1.0 / (1.0 + (self.depth / self.knee).exp());
        sigmoid / (self.softplus(1.0) - self.softplus(0.0))
    }

    /// `knee * ln(1 + exp((y - depth) / knee))`, guarded against overflow.
    ///
    /// For large `t` the function is `t` to well within f32 precision, and
    /// `exp(t)` would otherwise reach infinity around `t = 88` and make the
    /// normalization NaN.
    #[inline]
    fn softplus(&self, y: f32) -> f32 {
        let t = (y - self.depth) / self.knee;
        self.knee * if t > 20.0 { t } else { (1.0 + t.exp()).ln() }
    }
}

/// [`ShadowFloor::apply`] resampled onto a table, for the per-pixel path.
///
/// The curve costs a `ln` and an `exp`. Fused into the scale LUT that is paid
/// 8192 times per slider position and never again, which is why the common path
/// carries no cost at all; run directly over a 1440-square frame it would be six
/// million of each, per frame. This is the form the encoder's row tail uses when
/// saturation boost has kept the floor out of that LUT.
///
/// 4096 entries with linear interpolation, matching the scale LUT's own sizing
/// rationale: the curve's whole shape lives below `depth`, which is around 0.05,
/// so a table an order of magnitude coarser would resolve the knee with a
/// handful of samples.
pub struct ShadowFloorTable {
    entries: Vec<f32>,
}

impl ShadowFloorTable {
    const SIZE: usize = 4096;

    pub fn new(floor: ShadowFloor) -> Self {
        let entries = (0..Self::SIZE)
            .map(|i| floor.apply(i as f32 / (Self::SIZE - 1) as f32))
            .collect();
        Self { entries }
    }

    #[inline]
    pub fn lookup(&self, y: f32) -> f32 {
        let last = self.entries.len() - 1;
        let pos = (y.clamp(0.0, 1.0) * last as f32).min(last as f32);
        let i = (pos as usize).min(last - 1);
        let frac = pos - i as f32;
        self.entries[i] + (self.entries[i + 1] - self.entries[i]) * frac
    }
}

/// Apply a shadow floor to a flat interleaved RGB row in place.
///
/// Luminance-preserving, like the contrast pass it follows: the three channels
/// of a pixel are scaled together, so a sky pixel dims without its hue turning.
/// A per-channel subtraction would take a near-neutral shadow apart into its
/// components — `(12, 14, 13)` under a floor of 12 is `(0, 2, 1)`, which is
/// coloured speckle where there was grey sky.
pub fn apply_shadow_floor_slice(row: &mut [f32], table: &ShadowFloorTable) {
    crate::render::simd::apply_luminance_preserving_simd(row, 1.0, |l| table.lookup(l));
}

/// Apply a shadow floor to a whole planar frame in place.
///
/// The unfused counterpart to folding the curve into the scale LUT, for the
/// arms of [`crate::render::auto_stretch_frame`] that cannot fuse it: MTF
/// stretches each channel through its own midtone, so there is no single scale
/// table to carry the floor, and a mono frame never reaches that kernel at all.
///
/// Accepts 1 or 3 channels, matching `auto_stretch_frame` itself. Three channels
/// go through the same luminance-preserving scale as
/// [`apply_shadow_floor_slice`]; one channel *is* its own luminance, so the
/// curve applies directly.
pub fn apply_shadow_floor_frame(frame: &mut Frame, floor: ShadowFloor) -> Result<()> {
    let channels = frame.channels();
    if channels != 1 && channels != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "apply_shadow_floor_frame requires 1 or 3 channels, got {}",
            channels
        )));
    }

    if floor.is_none() {
        return Ok(());
    }

    let table = ShadowFloorTable::new(floor);
    let width = frame.width();

    if channels == 1 {
        frame
            .channel_data_mut(0)
            .par_chunks_mut(width.max(1))
            .with_min_len(32)
            .for_each(|row| {
                for v in row.iter_mut() {
                    *v = table.lookup(*v);
                }
            });
        return Ok(());
    }

    let (r, g, b) = frame.planes_mut();
    crate::render::simd::apply_luminance_preserving_simd_planar(r, g, b, width, 1.0, |l| {
        table.lookup(l)
    });

    Ok(())
}

/// The shadow floor as the settings express it, before the solve has said where
/// the sky landed.
///
/// Two stages exist because the two facts arrive at different times:
/// `get_render_pipeline_config` knows what the observer asked for and nothing
/// about the frame, while the autostretch solver knows where the sky ended up
/// and nothing about the request. [`resolve`](Self::resolve) is where they meet.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShadowFloorRequest {
    /// How far down to put the floor, as a fraction of the sky level. `0.0`
    /// disables; `1.0` puts it exactly at the sky, so half the sky's noise falls
    /// under the knee.
    pub fraction: f32,
    /// Clip at the floor instead of rolling off into it — deeper sky and a
    /// little more separation, paid for in samples on the panel's off state.
    pub hard: bool,
}

impl ShadowFloorRequest {
    pub const NONE: Self = Self {
        fraction: 0.0,
        hard: false,
    };

    #[inline]
    pub fn is_none(&self) -> bool {
        self.fraction <= 0.0
    }

    /// Turn the request into a curve, given where the sky actually landed.
    #[inline]
    pub fn resolve(&self, sky_level: f32) -> ShadowFloor {
        ShadowFloor::from_sky(self.fraction, sky_level, self.hard)
    }
}

/// Where the sky sits by the time the floor sees it.
///
/// The autostretch maps the sky to `target_background`, and the contrast S-curve
/// then moves it — at the shipped settings, 0.08 becomes 0.052. Anchoring to the
/// value *after* contrast is what makes the slider mean the same thing whether
/// or not contrast is on.
///
/// `target_background` must be the solver's own
/// [`AutoStretchResult::target_background`](crate::render::AutoStretchResult),
/// not the configured one: a frame that is mostly signal has its target raised
/// by up to 30 %, and anchoring to the configured value would leave the floor
/// that much too shallow on exactly the frames with the most to protect.
pub fn sky_level_after_contrast(
    target_background: f32,
    contrast: Option<&super::ContrastConfig>,
) -> f32 {
    match contrast {
        Some(config) if !config.is_disabled() => {
            super::apply_s_curve(target_background, config)
        }
        _ => target_background,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that keeps stars where they are. A floor that failed this
    /// would dim the whole frame, which is exactly the complaint the black level
    /// slider already has.
    #[test]
    fn white_stays_white_for_every_configuration() {
        for fraction in [0.1f32, 0.5, 1.0, 2.0, 10.0] {
            for sky in [0.01f32, 0.055, 0.2] {
                for hard in [false, true] {
                    let floor = ShadowFloor::from_sky(fraction, sky, hard);
                    let out = floor.apply(1.0);
                    assert!(
                        (out - 1.0).abs() < 1e-5,
                        "{floor:?} took white to {out}, not 1.0"
                    );
                }
            }
        }
    }

    #[test]
    fn none_is_the_identity() {
        for y in [0.0f32, 0.01, 0.5, 1.0] {
            assert_eq!(ShadowFloor::NONE.apply(y), y);
        }
        assert!(ShadowFloor::NONE.is_none());
        assert!(ShadowFloor::from_sky(0.0, 0.055, false).is_none());
        assert!(ShadowFloor::from_sky(1.0, 0.0, false).is_none());
    }

    /// Monotonicity is what makes this a tone curve rather than a scrambler: two
    /// pixels that differed in brightness must still differ, in the same order.
    #[test]
    fn every_form_is_monotone_over_the_whole_range() {
        for hard in [false, true] {
            let floor = ShadowFloor::from_sky(1.0, 0.055, hard);
            let mut previous = -1.0;
            for i in 0..=1000 {
                let out = floor.apply(i as f32 / 1000.0);
                assert!(out >= previous, "{floor:?} went backwards at {i}: {out}");
                previous = out;
            }
        }
    }

    /// The hard form's defining behaviour, and the reason it is opt-in.
    #[test]
    fn the_hard_form_clips_exactly_at_the_floor() {
        let floor = ShadowFloor::from_sky(1.0, 0.055, true);
        assert_eq!(floor.knee, 0.0);
        assert_eq!(floor.apply(0.055), 0.0);
        assert_eq!(floor.apply(0.0), 0.0);
        assert!(floor.apply(0.056) > 0.0);
    }

    /// The soft form's defining behaviour: it *compresses* the sub-floor sky
    /// instead of deleting it. Two values below the floor stay two values, which
    /// is exactly what the hard form cannot do — and it is the whole reason the
    /// soft form leaves no sample on the panel's off state.
    #[test]
    fn the_soft_form_compresses_below_the_floor_rather_than_clipping() {
        let soft = ShadowFloor::from_sky(1.0, 0.055, false);
        let hard = ShadowFloor::from_sky(1.0, 0.055, true);
        assert!(soft.knee > 0.0);

        for (lo, hi) in [(0.005f32, 0.010f32), (0.02, 0.03), (0.04, 0.05)] {
            assert!(
                soft.apply(hi) > soft.apply(lo),
                "soft floor collapsed {lo} and {hi} onto {}",
                soft.apply(lo)
            );
            assert_eq!(
                hard.apply(hi),
                hard.apply(lo),
                "the hard form is supposed to clip this pair"
            );
        }

        // Compressed, though: the whole sub-floor range lands inside one output
        // level, which is what reads as a smooth dark field rather than speckle.
        assert!(soft.apply(0.055) * 255.0 < 2.0);
        assert!(soft.apply(0.0) == 0.0);
    }

    /// The anchoring, stated as a test.
    ///
    /// The claim is *not* that two skies come out identical — they cannot, since
    /// the curve is normalized against white and a deeper floor leaves less room
    /// below it. The claim is that anchoring collapses the spread that an
    /// absolute floor leaves behind, and the two sky levels here are the ones
    /// the fixtures actually produce: 14 and 17 output levels.
    #[test]
    fn anchoring_collapses_what_an_absolute_floor_leaves_spread() {
        const DIM: f32 = 14.0 / 255.0;
        const BRIGHT: f32 = 17.0 / 255.0;
        let levels = |floor: ShadowFloor, sky: f32| floor.apply(sky) * 255.0;

        let anchored = (
            levels(ShadowFloor::from_sky(1.0, DIM, false), DIM),
            levels(ShadowFloor::from_sky(1.0, BRIGHT, false), BRIGHT),
        );
        // One absolute floor, tuned on the dim sky, applied to both.
        let fixed = ShadowFloor::from_sky(1.0, DIM, false);
        let absolute = (levels(fixed, DIM), levels(fixed, BRIGHT));

        let anchored_spread = (anchored.0 - anchored.1).abs();
        let absolute_spread = (absolute.0 - absolute.1).abs();

        assert!(
            anchored_spread < 1.0,
            "anchored skies landed {:.2} and {:.2} levels apart",
            anchored.0,
            anchored.1
        );
        assert!(
            anchored_spread < absolute_spread * 0.5,
            "anchoring bought nothing: spread {anchored_spread:.2} against \
             {absolute_spread:.2} for a fixed floor"
        );

        // The mechanism, so a failure above says which half moved.
        let bright_floor = ShadowFloor::from_sky(1.0, BRIGHT, false);
        assert!((bright_floor.depth / fixed.depth - BRIGHT / DIM).abs() < 1e-5);
        assert!((bright_floor.knee / fixed.knee - BRIGHT / DIM).abs() < 1e-5);
    }

    /// The anchor has to follow the solver's target, not the configured one.
    #[test]
    fn the_sky_anchor_follows_contrast_and_the_solved_target() {
        use crate::render::output::ContrastConfig;
        let shipped = ContrastConfig::default();

        // The shipped numbers: an 0.08 target reaches the floor at 0.052.
        let anchor = sky_level_after_contrast(0.08, Some(&shipped));
        assert!(
            (anchor - 0.0517).abs() < 1e-3,
            "anchor {anchor} is not where the S-curve puts the sky"
        );

        // No contrast, no move.
        assert_eq!(sky_level_after_contrast(0.08, None), 0.08);
        // A raised target has to raise the anchor with it.
        assert!(sky_level_after_contrast(0.104, Some(&shipped)) > anchor);
    }

    #[test]
    fn a_request_resolves_against_the_sky_it_is_given() {
        assert!(ShadowFloorRequest::NONE.is_none());
        assert!(ShadowFloorRequest::NONE.resolve(0.05).is_none());

        let soft = ShadowFloorRequest {
            fraction: 1.0,
            hard: false,
        };
        assert_eq!(soft.resolve(0.05).depth, 0.05);
        assert!(soft.resolve(0.05).knee > 0.0);

        let hard = ShadowFloorRequest {
            fraction: 0.5,
            hard: true,
        };
        assert_eq!(hard.resolve(0.05).depth, 0.025);
        assert_eq!(hard.resolve(0.05).knee, 0.0);
    }

    /// The table stands in for the curve in the encoder's row tail, so a
    /// visible disagreement between them is a visible difference between
    /// Community and Pro output at the same setting.
    #[test]
    fn the_table_tracks_the_curve_to_well_under_an_output_level() {
        for hard in [false, true] {
            let floor = ShadowFloor::from_sky(1.0, 0.055, hard);
            let table = ShadowFloorTable::new(floor);
            let mut worst = 0.0f32;
            for i in 0..=20_000 {
                let y = i as f32 / 20_000.0;
                worst = worst.max((table.lookup(y) - floor.apply(y)).abs());
            }
            assert!(
                worst * 255.0 < 0.15,
                "{floor:?}: table is off by {:.3} output levels",
                worst * 255.0
            );
        }
    }

    /// A nonsense slider value must not produce a curve that eats the image.
    #[test]
    fn depth_is_capped_and_negatives_disable() {
        assert_eq!(ShadowFloor::from_sky(100.0, 0.5, false).depth, MAX_DEPTH);
        assert!(ShadowFloor::from_sky(-1.0, 0.055, false).is_none());
        assert!(ShadowFloor::from_sky(1.0, -0.055, false).is_none());
    }

    /// Entry 0 of a composed scale table is a limit, not a sample, so the
    /// limit has to agree with the curve just above zero or the faintest bin
    /// steps.
    #[test]
    fn the_zero_limit_agrees_with_the_curve_just_above_zero() {
        for hard in [false, true] {
            let floor = ShadowFloor::from_sky(1.0, 0.055, hard);
            // Small enough to stay in the linear regime, large enough that the
            // f32 subtraction inside `apply` keeps its significant digits.
            let y = 1e-4;
            let sampled = floor.apply(y) / y;
            let limit = floor.slope_at_zero();
            assert!(
                (sampled - limit).abs() <= limit * 0.02 + 1e-9,
                "{floor:?}: limit {limit} against sampled {sampled}"
            );
        }
        assert_eq!(ShadowFloor::NONE.slope_at_zero(), 1.0);
    }

    /// Overflow guard: without the `t > 20` branch the normalizer is `inf` and
    /// every output becomes NaN.
    #[test]
    fn a_very_narrow_knee_stays_finite() {
        let floor = ShadowFloor {
            depth: 0.05,
            knee: 1e-4,
        };
        for i in 0..=100 {
            let out = floor.apply(i as f32 / 100.0);
            assert!(out.is_finite(), "non-finite output {out} at {i}");
        }
        assert!((floor.apply(1.0) - 1.0).abs() < 1e-5);
    }
}
