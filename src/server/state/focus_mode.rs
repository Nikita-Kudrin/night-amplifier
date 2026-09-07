//! Focus/Finder mode: hold the cosmetic pipeline stages off while framing.
//!
//! Focusing and star-hopping need frame rate, not a clean image — the seven
//! settings below are the ones that cost per-frame work and buy nothing at a
//! focus mask. The mode is reversible, so entering it snapshots what it
//! overwrites and leaving it puts every value back.
//!
//! Deliberately *not* in the managed set: `superpixel_debayer`, which is the
//! cheap demosaic (bins 2x2 rather than interpolating), so forcing it either
//! way would trade against the frame rate this mode exists to buy.

use super::capture_mode::CaptureMode;
use super::settings::CaptureSettings;
use super::types::CaptureState;

/// The seven booleans Focus/Finder mode forces off, as they were before it did.
///
/// Stored rather than recomputed because there is nothing to recompute from:
/// once the live values are `false` they no longer say what the observer chose.
///
/// Every field is `#[serde(default)]` on purpose: a snapshot that fails to parse takes
/// the whole `PersistedSettings` down with it and `load()` then returns `None`, resetting
/// *every* setting the observer has. Each default is the setting's own default rather
/// than `false`, so a field missing from an older file restores the correction rather
/// than silently disabling it — six of the seven are on by default.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusModeSnapshot {
    #[serde(default = "default_on")]
    pub background_subtraction: bool,
    #[serde(default)]
    pub saturation_boost: bool,
    #[serde(default = "default_on")]
    pub hot_pixel_rejection: bool,
    #[serde(default = "default_on")]
    pub fpn_removal: bool,
    #[serde(default = "default_on")]
    pub denoise_chroma: bool,
    #[serde(default = "default_on")]
    pub denoise_luma: bool,
    #[serde(default = "default_on")]
    pub dither: bool,
}

fn default_on() -> bool {
    true
}

impl FocusModeSnapshot {
    fn capture(settings: &CaptureSettings) -> Self {
        Self {
            background_subtraction: settings.background_subtraction,
            saturation_boost: settings.saturation_boost,
            hot_pixel_rejection: settings.sensor_correction.hot_pixel_rejection,
            fpn_removal: settings.sensor_correction.fpn_removal,
            denoise_chroma: settings.denoise.chroma,
            denoise_luma: settings.denoise.luma,
            dither: settings.eyepiece.dither,
        }
    }

    fn restore_into(self, settings: &mut CaptureSettings) {
        settings.background_subtraction = self.background_subtraction;
        // Shadow saturation boost is Pro-gated at the API. A licence can lapse while
        // the mode is on, and restoring blind would hand back a Pro stage through a
        // request that never passed the check that guards it.
        settings.saturation_boost = self.saturation_boost && saturation_boost_licensed();
        settings.sensor_correction.hot_pixel_rejection = self.hot_pixel_rejection;
        settings.sensor_correction.fpn_removal = self.fpn_removal;
        settings.denoise.chroma = self.denoise_chroma;
        settings.denoise.luma = self.denoise_luma;
        settings.eyepiece.dither = self.dither;
    }
}

fn saturation_boost_licensed() -> bool {
    crate::license::pro_plugin(&crate::render::SATURATION_PLUGIN).is_some()
}

/// Whether entering the mode now would damage a stack already being integrated.
///
/// Two of the seven — `hot_pixel_rejection` and `fpn_removal` — run on the raw mosaic
/// *before* demosaic, so the frame they produce is the frame that goes into the
/// accumulator. Turning them off mid-session mixes hot pixels and row/column banding
/// into an existing master, and those are precisely the defects averaging cannot
/// remove: nothing later can take them back out. The other five are render-only.
///
/// Live view accumulates nothing, so it is left alone — the mode is most useful exactly
/// there. Leaving the mode is never a conflict.
pub fn conflicts_with_capture(settings: &CaptureSettings, capture_state: CaptureState) -> bool {
    if capture_state == CaptureState::Idle {
        return false;
    }
    matches!(
        settings.capture_mode(),
        CaptureMode::Stacking | CaptureMode::Wanderer
    )
}

/// Force every managed setting off, leaving the rest of the block alone.
fn force_off(settings: &mut CaptureSettings) {
    settings.background_subtraction = false;
    settings.saturation_boost = false;
    settings.sensor_correction.hot_pixel_rejection = false;
    settings.sensor_correction.fpn_removal = false;
    settings.denoise.chroma = false;
    settings.denoise.luma = false;
    settings.eyepiece.dither = false;
}

/// Enter or leave Focus/Finder mode.
///
/// Idempotent in both directions, and that is the whole contract: entering while
/// already entered must not re-snapshot, because by then every managed value is
/// the forced `false` and taking it would destroy what the observer chose.
pub fn set(settings: &mut CaptureSettings, on: bool) {
    if on == settings.focus_mode {
        return;
    }
    if on {
        settings.focus_mode_snapshot = Some(FocusModeSnapshot::capture(settings));
        force_off(settings);
        settings.focus_mode = true;
        return;
    }
    if let Some(snapshot) = settings.focus_mode_snapshot.take() {
        snapshot.restore_into(settings);
    }
    settings.focus_mode = false;
}

/// Absorb any managed value that drifted away from the forced `false` into the
/// snapshot, and force it off again.
///
/// The UI disables these controls while the mode is on, so drift only reaches
/// here from a stale client or a second one — but without this the write would
/// be silently reverted on the next toggle instead of remembered.
pub fn reconcile(settings: &mut CaptureSettings) {
    if !settings.focus_mode {
        return;
    }
    let Some(snapshot) = settings.focus_mode_snapshot.as_mut() else {
        return;
    };
    let absorb = |live: &mut bool, saved: &mut bool| {
        if *live {
            *saved = true;
            *live = false;
        }
    };
    absorb(
        &mut settings.background_subtraction,
        &mut snapshot.background_subtraction,
    );
    absorb(&mut settings.saturation_boost, &mut snapshot.saturation_boost);
    absorb(
        &mut settings.sensor_correction.hot_pixel_rejection,
        &mut snapshot.hot_pixel_rejection,
    );
    absorb(
        &mut settings.sensor_correction.fpn_removal,
        &mut snapshot.fpn_removal,
    );
    absorb(&mut settings.denoise.chroma, &mut snapshot.denoise_chroma);
    absorb(&mut settings.denoise.luma, &mut snapshot.denoise_luma);
    absorb(&mut settings.eyepiece.dither, &mut snapshot.dither);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings block where the managed values are a mix, so a restore that
    /// blanket-sets them either way fails instead of passing by luck.
    fn mixed() -> CaptureSettings {
        let mut settings = CaptureSettings {
            background_subtraction: true,
            saturation_boost: false,
            ..Default::default()
        };
        settings.sensor_correction.hot_pixel_rejection = true;
        settings.sensor_correction.fpn_removal = false;
        settings.denoise.chroma = false;
        settings.denoise.luma = true;
        settings.eyepiece.dither = true;
        settings
    }

    fn all_managed_off(settings: &CaptureSettings) -> bool {
        !settings.background_subtraction
            && !settings.saturation_boost
            && !settings.sensor_correction.hot_pixel_rejection
            && !settings.sensor_correction.fpn_removal
            && !settings.denoise.chroma
            && !settings.denoise.luma
            && !settings.eyepiece.dither
    }

    #[test]
    fn entering_forces_every_managed_setting_off() {
        let mut settings = mixed();
        set(&mut settings, true);

        assert!(settings.focus_mode);
        assert!(all_managed_off(&settings));
        assert!(settings.focus_mode_snapshot.is_some());
    }

    #[test]
    fn entering_leaves_unmanaged_settings_alone() {
        let mut settings = mixed();
        settings.sensor_correction.superpixel_debayer = true;
        settings.sensor_correction.hot_pixel_sigma = 7.5;
        settings.denoise.chroma_strength = 0.25;
        settings.denoise.star_protection = 0.4;
        settings.stacking = true;

        set(&mut settings, true);

        assert!(settings.sensor_correction.superpixel_debayer);
        assert_eq!(settings.sensor_correction.hot_pixel_sigma, 7.5);
        assert_eq!(settings.denoise.chroma_strength, 0.25);
        assert_eq!(settings.denoise.star_protection, 0.4);
        assert!(settings.stacking);
    }

    #[test]
    fn entering_twice_keeps_the_pre_focus_snapshot() {
        let mut settings = mixed();
        let before = mixed();

        set(&mut settings, true);
        set(&mut settings, true);
        set(&mut settings, false);

        assert!(!settings.focus_mode);
        assert_eq!(
            settings.background_subtraction,
            before.background_subtraction
        );
        assert_eq!(
            settings.sensor_correction.hot_pixel_rejection,
            before.sensor_correction.hot_pixel_rejection
        );
        assert_eq!(settings.denoise.luma, before.denoise.luma);
        assert_eq!(settings.eyepiece.dither, before.eyepiece.dither);
    }

    #[test]
    fn round_trip_restores_every_managed_setting() {
        let mut settings = mixed();
        let before = mixed();

        set(&mut settings, true);
        set(&mut settings, false);

        assert!(!settings.focus_mode);
        assert!(settings.focus_mode_snapshot.is_none());
        assert_eq!(
            settings.background_subtraction,
            before.background_subtraction
        );
        assert_eq!(settings.saturation_boost, before.saturation_boost);
        assert_eq!(
            settings.sensor_correction.hot_pixel_rejection,
            before.sensor_correction.hot_pixel_rejection
        );
        assert_eq!(
            settings.sensor_correction.fpn_removal,
            before.sensor_correction.fpn_removal
        );
        assert_eq!(settings.denoise.chroma, before.denoise.chroma);
        assert_eq!(settings.denoise.luma, before.denoise.luma);
        assert_eq!(settings.eyepiece.dither, before.eyepiece.dither);
    }

    #[test]
    fn leaving_without_a_snapshot_is_a_no_op() {
        let mut settings = mixed();
        let before = mixed();

        set(&mut settings, false);

        assert!(!settings.focus_mode);
        assert_eq!(
            settings.background_subtraction,
            before.background_subtraction
        );
        assert_eq!(settings.denoise.luma, before.denoise.luma);
    }

    #[test]
    fn reconcile_absorbs_a_drifted_value_into_the_snapshot() {
        let mut settings = CaptureSettings::default();
        settings.denoise.chroma = false;
        set(&mut settings, true);

        // A stale client re-enables it behind the mode's back.
        settings.denoise.chroma = true;
        reconcile(&mut settings);

        assert!(!settings.denoise.chroma, "the mode must re-force it off");

        set(&mut settings, false);
        assert!(
            settings.denoise.chroma,
            "the write must survive as the restored value"
        );
    }

    #[test]
    fn reconcile_leaves_unmanaged_siblings_untouched() {
        let mut settings = CaptureSettings::default();
        set(&mut settings, true);

        settings.denoise.chroma_strength = 0.75;
        settings.sensor_correction.hot_pixel_sigma = 9.0;
        settings.sensor_correction.superpixel_debayer = true;
        reconcile(&mut settings);

        assert_eq!(settings.denoise.chroma_strength, 0.75);
        assert_eq!(settings.sensor_correction.hot_pixel_sigma, 9.0);
        assert!(settings.sensor_correction.superpixel_debayer);
    }

    #[test]
    fn stacking_captures_conflict_with_entering_the_mode() {
        let settings = CaptureSettings {
            stacking: true,
            wanderer_mode: false,
            ..Default::default()
        };

        assert!(conflicts_with_capture(
            &settings,
            CaptureState::Capturing
        ));
        assert!(conflicts_with_capture(&settings, CaptureState::Starting));
    }

    /// Wanderer throws its stack away when the mount moves, but it still integrates
    /// between resets.
    #[test]
    fn wanderer_captures_conflict_too() {
        let settings = CaptureSettings {
            stacking: true,
            wanderer_mode: true,
            ..Default::default()
        };

        assert!(conflicts_with_capture(
            &settings,
            CaptureState::Capturing
        ));
    }

    /// The guard is about the accumulator, not about the camera being busy.
    #[test]
    fn live_view_never_conflicts() {
        let settings = CaptureSettings {
            stacking: false,
            ..Default::default()
        };

        assert!(!conflicts_with_capture(
            &settings,
            CaptureState::Capturing
        ));
    }

    #[test]
    fn an_idle_rig_never_conflicts() {
        let settings = CaptureSettings {
            stacking: true,
            ..Default::default()
        };

        assert!(!conflicts_with_capture(&settings, CaptureState::Idle));
    }

    #[test]
    fn reconcile_outside_focus_mode_does_nothing() {
        let mut settings = mixed();
        let before = mixed();

        reconcile(&mut settings);

        assert_eq!(
            settings.background_subtraction,
            before.background_subtraction
        );
        assert!(settings.focus_mode_snapshot.is_none());
    }
}
