//! The darkening half of the black floor. Not the pedestal with the sign flipped —
//! [`super::DisplayOutput::pedestal`] is a property of the *panel* (absolute fraction of
//! full scale), while darkening is a property of the *sky* (fraction of wherever the sky
//! actually landed), so one slider position means one thing across targets.
//!
//! Two forms. The default is spatial ([`super::SkyShadow`]): a gain chosen from the
//! neighbourhood, because every pointwise roll-off raised relative sky grain. The clip
//! here is the "Darker sky" setting: deepest background, paid for in pixels switched off.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;

use super::sky_shadow::SkyShadow;

/// The deepest floor that still leaves a usable range above it.
const MAX_DEPTH: f32 = 0.5;

/// A hard floor: output below `depth` clips to black, the rest is rescaled so white
/// stays white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowFloor {
    /// Output level that maps to black, in `[0, 0.5]`. Zero disables.
    pub depth: f32,
}

impl Default for ShadowFloor {
    fn default() -> Self {
        Self::NONE
    }
}

impl ShadowFloor {
    /// No darkening: [`ShadowFloor::apply`] is the identity.
    pub const NONE: Self = Self { depth: 0.0 };

    /// A floor `fraction` of the way up to the measured sky level.
    pub fn from_sky(fraction: f32, sky_level: f32) -> Self {
        Self {
            depth: (fraction.max(0.0) * sky_level.max(0.0)).min(MAX_DEPTH),
        }
    }

    /// True when this transform is the identity, letting callers skip it.
    #[inline]
    pub fn is_none(&self) -> bool {
        self.depth <= 0.0
    }

    /// Map one output-referred value through the floor. `apply(0.0) == 0.0` and
    /// `apply(1.0) == 1.0` always: white staying white keeps star cores at 255, and
    /// black staying black lets this compose into a *scale* table (`curve(L) / L`).
    #[inline]
    pub fn apply(&self, y: f32) -> f32 {
        if self.is_none() {
            return y;
        }
        ((y - self.depth) / (1.0 - self.depth)).max(0.0)
    }

    /// The `y → 0` limit of `apply(y) / y`, for a scale table's entry 0: genuinely
    /// zero for a clip — everything under the floor is black, as asked.
    #[inline]
    pub fn slope_at_zero(&self) -> f32 {
        if self.is_none() {
            1.0
        } else {
            0.0
        }
    }
}

/// [`ShadowFloor::apply`] resampled onto a table, for the per-pixel path. The curve
/// costs a `ln` and an `exp` — fused into the scale LUT (paid 8192 times per slider
/// position, never again) the common path is free; run directly over a 1440² frame
/// it would be six million of each, per frame. Used by the encoder's row tail when
/// saturation boost has pushed the floor out of that LUT.
///
/// 4096 entries, linear interpolation — matching the scale LUT's sizing rationale:
/// the curve's shape lives below `depth` (~0.05), so a coarser table would still
/// resolve the clip with a handful of samples.
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

/// Apply a shadow floor to a whole planar frame in place — the unfused counterpart
/// to folding the curve into the scale LUT, for [`crate::render::auto_stretch_frame`]
/// arms that can't fuse it: MTF stretches each channel through its own midtone (no
/// single scale table), and mono never reaches that kernel at all. Accepts 1 or 3
/// channels; three go through the same luminance-preserving scale as
/// [`apply_shadow_floor_slice`], one channel applies the curve directly (it *is* its
/// own luminance).
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

/// The darkening as the settings express it, before the solve has said where the sky
/// landed. `get_render_pipeline_config` knows the request and nothing about the frame;
/// the autostretch solver knows the sky and nothing about the request —
/// [`resolve`](Self::resolve) is where they meet.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShadowFloorRequest {
    /// Slider position in nominal sky levels. `0.0` disables.
    pub fraction: f32,
    /// Clip at `fraction` of the sky ("Darker sky") instead of the spatial gain.
    pub hard: bool,
}

/// A request resolved against a sky level: at most one of the two forms is active.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ResolvedShadow {
    /// Pointwise, so it can ride the scale LUT.
    pub floor: ShadowFloor,
    /// Spatial, so the encoder applies it after the per-row tail.
    pub sky: Option<SkyShadow>,
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

    /// Turn the request into a transform, given where the sky actually landed.
    pub fn resolve(&self, sky_level: f32) -> ResolvedShadow {
        if self.hard {
            ResolvedShadow {
                floor: ShadowFloor::from_sky(self.fraction, sky_level),
                sky: None,
            }
        } else {
            ResolvedShadow {
                floor: ShadowFloor::NONE,
                sky: SkyShadow::from_sky(self.fraction, sky_level),
            }
        }
    }
}

/// Where the sky sits by the time the floor sees it. The autostretch maps the sky to
/// `target_background`, then the contrast S-curve moves it (0.08 -> 0.045 at shipped
/// settings) — anchoring *after* contrast is what makes the slider mean the same
/// thing whether contrast is on or not. Must be the solver's own
/// [`AutoStretchResult::target_background`](crate::render::AutoStretchResult), not the
/// configured one: a mostly-signal frame has its target raised up to 30%, and the
/// configured value would leave the floor too shallow on exactly the frames with the
/// most to protect.
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

    #[test]
    fn white_stays_white_for_every_configuration() {
        for fraction in [0.1f32, 0.5, 1.0, 2.0, 10.0] {
            for sky in [0.01f32, 0.055, 0.2] {
                let floor = ShadowFloor::from_sky(fraction, sky);
                let out = floor.apply(1.0);
                assert!((out - 1.0).abs() < 1e-5, "{floor:?} took white to {out}");
            }
        }
    }

    #[test]
    fn none_is_the_identity() {
        for y in [0.0f32, 0.01, 0.5, 1.0] {
            assert_eq!(ShadowFloor::NONE.apply(y), y);
        }
        assert!(ShadowFloor::from_sky(0.0, 0.055).is_none());
        assert!(ShadowFloor::from_sky(1.0, 0.0).is_none());
    }

    #[test]
    fn the_clip_is_monotone_over_the_whole_range() {
        let floor = ShadowFloor::from_sky(1.0, 0.055);
        let mut previous = -1.0;
        for i in 0..=1000 {
            let out = floor.apply(i as f32 / 1000.0);
            assert!(out >= previous, "went backwards at {i}: {out}");
            previous = out;
        }
    }

    /// The hard form's defining behaviour, and the reason it is opt-in.
    #[test]
    fn the_hard_form_clips_exactly_at_the_floor() {
        let floor = ShadowFloor::from_sky(1.0, 0.055);
        assert_eq!(floor.apply(0.055), 0.0);
        assert_eq!(floor.apply(0.0), 0.0);
        assert!(floor.apply(0.056) > 0.0);
    }

    /// The anchor has to follow the solver's target, not the configured one.
    #[test]
    fn the_sky_anchor_follows_contrast_and_the_solved_target() {
        use crate::render::output::ContrastConfig;
        let shipped = ContrastConfig::default();

        // The shipped numbers: an 0.08 target reaches the floor at 0.045.
        let anchor = sky_level_after_contrast(0.08, Some(&shipped));
        assert!(
            (anchor - 0.0447).abs() < 1e-3,
            "anchor {anchor} is not where the S-curve puts the sky"
        );
        assert_eq!(sky_level_after_contrast(0.08, None), 0.08);
        assert!(sky_level_after_contrast(0.104, Some(&shipped)) > anchor);
    }

    #[test]
    fn a_request_resolves_to_exactly_one_form() {
        assert!(ShadowFloorRequest::NONE.is_none());
        let none = ShadowFloorRequest::NONE.resolve(0.05);
        assert!(none.floor.is_none() && none.sky.is_none());

        let soft = ShadowFloorRequest {
            fraction: 1.0,
            hard: false,
        }
        .resolve(0.05);
        assert!(soft.floor.is_none());
        assert_eq!(soft.sky.map(|s| s.sky), Some(0.05));

        let hard = ShadowFloorRequest {
            fraction: 0.5,
            hard: true,
        }
        .resolve(0.05);
        assert_eq!(hard.floor.depth, 0.025);
        assert!(hard.sky.is_none());
    }

    /// The table stands in for the curve in the encoder's row tail.
    #[test]
    fn the_table_tracks_the_curve_to_well_under_an_output_level() {
        let floor = ShadowFloor::from_sky(1.0, 0.055);
        let table = ShadowFloorTable::new(floor);
        let mut worst = 0.0f32;
        for i in 0..=20_000 {
            let y = i as f32 / 20_000.0;
            worst = worst.max((table.lookup(y) - floor.apply(y)).abs());
        }
        assert!(worst * 255.0 < 0.15, "table is off by {:.3} levels", worst * 255.0);
    }

    #[test]
    fn depth_is_capped_and_negatives_disable() {
        assert_eq!(ShadowFloor::from_sky(100.0, 0.5).depth, MAX_DEPTH);
        assert!(ShadowFloor::from_sky(-1.0, 0.055).is_none());
        assert!(ShadowFloor::from_sky(1.0, -0.055).is_none());
    }

    #[test]
    fn the_zero_limit_agrees_with_the_curve_just_above_zero() {
        let floor = ShadowFloor::from_sky(1.0, 0.055);
        assert_eq!(floor.apply(1e-4) / 1e-4, floor.slope_at_zero());
        assert_eq!(ShadowFloor::NONE.slope_at_zero(), 1.0);
    }
}
