//! Focus/Finder mode: hold the cosmetic pipeline stages off while framing.
//!
//! Focusing/star-hopping need frame rate, not a clean image — six settings cost
//! per-frame work for nothing at a focus mask; reversible, so leaving restores them.
//!
//! Deliberately *not* managed: `superpixel_debayer` (cheap 2x2-bin demosaic, trading
//! against the frame rate this mode buys) and hot-pixel rejection, which has no
//! switch — Push-To needs it to solve these frames.

use super::capture_mode::CaptureMode;
use super::settings::CaptureSettings;
use super::types::CaptureState;
use crate::plugins::Plugins;

/// The six settings Focus/Finder mode forces off, as they were before it did. One is a
/// strength not a boolean (`denoise_luma_strength`), hence `PartialEq` not `Eq`.
/// Stored rather than recomputed: once live values are off they no longer say what
/// the observer chose.
///
/// Every field is `#[serde(default)]`: a parse failure here fails the whole settings
/// file. Each default is the setting's own, not `false`, so a missing
/// field restores the correction rather than disabling it (five of six are on).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FocusModeSnapshot {
    #[serde(default = "default_on")]
    pub background_subtraction: bool,
    #[serde(default)]
    pub saturation_boost: bool,
    #[serde(default = "default_on")]
    pub fpn_removal: bool,
    #[serde(default = "default_on")]
    pub denoise_chroma: bool,
    /// The wavelet's strength, which is what turns it off — zeroing it is how the mode
    /// holds the filter down, so a boolean could not say what to hand back. Note this is
    /// the *strength* and not the Background Grain dial: the dial also moves the tone
    /// curve's split, which this mode has no business touching, so it is unmanaged.
    #[serde(default = "default_luma_strength")]
    pub denoise_luma_strength: f32,
    #[serde(default = "default_on")]
    pub dither: bool,
}

fn default_on() -> bool {
    true
}

fn default_luma_strength() -> f32 {
    1.0
}

impl FocusModeSnapshot {
    fn capture(settings: &CaptureSettings) -> Self {
        Self {
            background_subtraction: settings.background_subtraction,
            saturation_boost: settings.saturation_boost,
            fpn_removal: settings.sensor_correction.fpn_removal,
            denoise_chroma: settings.denoise.chroma,
            denoise_luma_strength: settings.denoise.luma_strength,
            dither: settings.eyepiece.dither,
        }
    }

    fn restore_into(self, settings: &mut CaptureSettings, plugins: &Plugins) {
        settings.background_subtraction = self.background_subtraction;
        // Shadow saturation boost is Pro-gated at the API. A licence can lapse while
        // the mode is on, and restoring blind would hand back a Pro stage through a
        // request that never passed the check that guards it.
        settings.saturation_boost = self.saturation_boost && plugins.saturation().is_some();
        settings.sensor_correction.fpn_removal = self.fpn_removal;
        settings.denoise.chroma = self.denoise_chroma;
        settings.denoise.luma_strength = self.denoise_luma_strength;
        settings.eyepiece.dither = self.dither;
    }
}

/// Whether the mode would damage a stack integrated by this capture.
///
/// One of the six — `fpn_removal` — runs on the raw mosaic *before* demosaic, so
/// turning it off mid-session mixes row/column banding into an existing master, a
/// defect averaging cannot remove. The other five are render-only: no conflict there,
/// nor wherever no affected frame reaches an accumulator (live view integrates
/// nothing, planetary has no line flattening to lose, idle/stopping takes no new
/// frame). Leaving the mode is never a conflict.
pub fn conflicts_with_capture(
    mode: CaptureMode,
    stacking_type: crate::stacking::StackingType,
    capture_state: CaptureState,
) -> bool {
    takes_new_frames(capture_state)
        && matches!(mode, CaptureMode::Stacking | CaptureMode::Wanderer)
        && stacking_type.uses_fpn_removal()
}

/// `Recovering` counts: the resume takes its frames with these settings.
fn takes_new_frames(capture_state: CaptureState) -> bool {
    matches!(
        capture_state,
        CaptureState::Starting | CaptureState::Capturing | CaptureState::Recovering
    )
}

/// Whether `settings` hold the mode on in conflict with the capture.
pub fn in_conflict(settings: &CaptureSettings, capture_state: CaptureState) -> bool {
    settings.focus_mode
        && conflicts_with_capture(
            settings.capture_mode(),
            settings.stacking_type,
            capture_state,
        )
}

/// Leave the mode if it now conflicts with the capture; returns whether it left.
///
/// Every way into an accumulating session goes through here: a start, a resume, and a
/// running live view switched to stacking, which no start path sees. Live view keeps the
/// mode — 2026-09-07 two live-view starts dropped it and the observer turned it back on by
/// hand both times.
pub fn leave_if_conflicting(
    settings: &mut CaptureSettings,
    capture_state: CaptureState,
    plugins: &Plugins,
) -> bool {
    if !in_conflict(settings, capture_state) {
        return false;
    }
    set(settings, false, plugins);
    true
}

impl super::AppState {
    /// [`leave_if_conflicting`] on the shared settings; when it left, persist them and move
    /// every client's toggle. Returns whether it left.
    pub async fn leave_focus_mode_if_conflicting(&self, capture_state: CaptureState) -> bool {
        let left =
            leave_if_conflicting(&mut *self.settings.write().await, capture_state, &self.plugins);
        if left {
            self.save_settings().await;
            let _ = self
                .events
                .send(crate::server::events::ServerEvent::SettingsUpdated);
        }
        left
    }

    /// The settings a frame about to be exposed is captured and stacked with.
    ///
    /// Last line of defence for the stacking conflict: `update_settings` must read the
    /// capture state before taking the settings lock (lock order), so a Start landing in that
    /// gap beside `stacking: true` would stack under the mode. The capture loop is taking
    /// frames by definition, so a conflict found here is left before the snapshot. The
    /// common path costs only the clone the loop always made.
    pub async fn settings_for_new_frame(&self) -> CaptureSettings {
        {
            let settings = self.settings.read().await;
            if !in_conflict(&settings, CaptureState::Capturing) {
                return settings.clone();
            }
        }
        if self
            .leave_focus_mode_if_conflicting(CaptureState::Capturing)
            .await
        {
            tracing::warn!("Leaving Focus/Finder mode: the capture was about to stack under it");
            let _ = self
                .events
                .send(crate::server::events::ServerEvent::FocusModeLeft);
        }
        self.settings.read().await.clone()
    }
}

/// Force every managed setting off, leaving the rest of the block alone.
fn force_off(settings: &mut CaptureSettings) {
    settings.background_subtraction = false;
    settings.saturation_boost = false;
    settings.sensor_correction.fpn_removal = false;
    settings.denoise.chroma = false;
    // The strength, not the Background Grain dial: the dial also sets the tone curve's
    // split, and this mode has no business moving the tone curve. Zeroing the strength
    // is exactly what the old `luma` boolean did and nothing more.
    settings.denoise.luma_strength = 0.0;
    settings.eyepiece.dither = false;
}

/// Enter or leave Focus/Finder mode.
///
/// Idempotent in both directions, and that is the whole contract: entering while
/// already entered must not re-snapshot, because by then every managed value is
/// the forced `false` and taking it would destroy what the observer chose. `plugins` decide
/// what leaving may restore: a Pro stage only while its plugin is licensed.
pub fn set(settings: &mut CaptureSettings, on: bool, plugins: &Plugins) {
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
        snapshot.restore_into(settings, plugins);
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
        &mut settings.sensor_correction.fpn_removal,
        &mut snapshot.fpn_removal,
    );
    absorb(&mut settings.denoise.chroma, &mut snapshot.denoise_chroma);
    // The same shape as `absorb`, on the one managed setting that is a strength rather
    // than a boolean: whatever the write asked for becomes what the mode hands back.
    if settings.denoise.luma_strength > 0.0 {
        snapshot.denoise_luma_strength = settings.denoise.luma_strength;
        settings.denoise.luma_strength = 0.0;
    }
    absorb(&mut settings.eyepiece.dither, &mut snapshot.dither);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::events::ServerEvent;
    use crate::server::state::AppState;
    use crate::stacking::StackingType;

    /// A settings block where the managed values are a mix, so a restore that
    /// blanket-sets them either way fails instead of passing by luck.
    fn mixed() -> CaptureSettings {
        let mut settings = CaptureSettings {
            background_subtraction: true,
            saturation_boost: false,
            ..Default::default()
        };
        // The one pre-demosaic value the mode manages, set on so a restore that
        // blanket-forces it off fails.
        settings.sensor_correction.fpn_removal = true;
        settings.denoise.chroma = false;
        settings.denoise.luma_strength = 0.75;
        settings.eyepiece.dither = true;
        settings
    }

    fn all_managed_off(settings: &CaptureSettings) -> bool {
        !settings.background_subtraction
            && !settings.saturation_boost
            && !settings.sensor_correction.fpn_removal
            && !settings.denoise.chroma
            && settings.denoise.luma_strength == 0.0
            && !settings.eyepiece.dither
    }

    #[test]
    fn entering_forces_every_managed_setting_off() {
        let mut settings = mixed();
        set(&mut settings, true, &Plugins::none());

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
        settings.denoise.background_grain = 0.8;
        settings.stacking = true;

        set(&mut settings, true, &Plugins::none());

        assert!(settings.sensor_correction.superpixel_debayer);
        assert_eq!(settings.sensor_correction.hot_pixel_sigma, 7.5);
        assert_eq!(settings.denoise.chroma_strength, 0.25);
        // The dial is unmanaged on purpose: it moves the tone curve as well as the
        // wavelet, and the mode holds only the wavelet off.
        assert_eq!(settings.denoise.background_grain, 0.8);
        assert!(settings.stacking);
    }

    #[test]
    fn entering_twice_keeps_the_pre_focus_snapshot() {
        let mut settings = mixed();
        let before = mixed();

        set(&mut settings, true, &Plugins::none());
        set(&mut settings, true, &Plugins::none());
        set(&mut settings, false, &Plugins::none());

        assert!(!settings.focus_mode);
        assert_eq!(
            settings.background_subtraction,
            before.background_subtraction
        );
        assert_eq!(
            settings.sensor_correction.fpn_removal,
            before.sensor_correction.fpn_removal
        );
        assert_eq!(settings.denoise.luma_strength, before.denoise.luma_strength);
        assert_eq!(settings.eyepiece.dither, before.eyepiece.dither);
    }

    #[test]
    fn round_trip_restores_every_managed_setting() {
        let mut settings = mixed();
        let before = mixed();

        set(&mut settings, true, &Plugins::none());
        set(&mut settings, false, &Plugins::none());

        assert!(!settings.focus_mode);
        assert!(settings.focus_mode_snapshot.is_none());
        assert_eq!(
            settings.background_subtraction,
            before.background_subtraction
        );
        assert_eq!(settings.saturation_boost, before.saturation_boost);
        assert_eq!(
            settings.sensor_correction.fpn_removal,
            before.sensor_correction.fpn_removal
        );
        assert_eq!(settings.denoise.chroma, before.denoise.chroma);
        assert_eq!(settings.denoise.luma_strength, before.denoise.luma_strength);
        assert_eq!(settings.eyepiece.dither, before.eyepiece.dither);
    }

    #[test]
    fn leaving_without_a_snapshot_is_a_no_op() {
        let mut settings = mixed();
        let before = mixed();

        set(&mut settings, false, &Plugins::none());

        assert!(!settings.focus_mode);
        assert_eq!(
            settings.background_subtraction,
            before.background_subtraction
        );
        assert_eq!(settings.denoise.luma_strength, before.denoise.luma_strength);
    }

    /// Shadow saturation boost is a Pro stage: leaving the mode restores it only while
    /// its plugin is licensed, so a lapsed licence cannot hand it back.
    #[test]
    fn leaving_restores_saturation_boost_only_with_a_licensed_plugin() {
        use crate::render::stretch::{SaturationBoostConfig, SaturationPlugin};
        use std::sync::Arc;

        struct Saturation;
        impl SaturationPlugin for Saturation {
            fn apply_boost(&self, _: &mut crate::frame::Frame, _: &SaturationBoostConfig) -> crate::error::Result<()> {
                Ok(())
            }
            fn apply_boost_slice(&self, _: &mut [f32], _: &SaturationBoostConfig) {}
        }

        let licensed = Plugins::none().with_saturation(Arc::new(Saturation)).always_licensed();
        let lapsed = Plugins::none().with_saturation(Arc::new(Saturation));
        for (plugins, restored) in [(licensed, true), (lapsed, false), (Plugins::none(), false)] {
            let mut settings = CaptureSettings {
                saturation_boost: true,
                ..mixed()
            };
            set(&mut settings, true, &plugins);
            set(&mut settings, false, &plugins);
            assert_eq!(settings.saturation_boost, restored, "{plugins:?}");
        }
    }

    #[test]
    fn reconcile_absorbs_a_drifted_value_into_the_snapshot() {
        let mut settings = CaptureSettings::default();
        settings.denoise.chroma = false;
        set(&mut settings, true, &Plugins::none());

        // A stale client re-enables it behind the mode's back.
        settings.denoise.chroma = true;
        reconcile(&mut settings);

        assert!(!settings.denoise.chroma, "the mode must re-force it off");

        set(&mut settings, false, &Plugins::none());
        assert!(
            settings.denoise.chroma,
            "the write must survive as the restored value"
        );
    }

    /// The strength takes the same round trip as the booleans, and it is the one that
    /// could not: `absorb` only knows how to move a `bool`, so this setting is handled by
    /// its own branch and nothing else tests that branch's far end.
    #[test]
    fn reconcile_absorbs_a_drifted_strength_into_the_snapshot() {
        let mut settings = CaptureSettings::default();
        settings.denoise.luma_strength = 0.0;
        set(&mut settings, true, &Plugins::none());

        // A stale client sets a strength behind the mode's back.
        settings.denoise.luma_strength = 0.6;
        reconcile(&mut settings);
        assert_eq!(
            settings.denoise.luma_strength, 0.0,
            "the mode must hold the wavelet off"
        );

        set(&mut settings, false, &Plugins::none());
        assert_eq!(
            settings.denoise.luma_strength, 0.6,
            "the write must survive as the restored value, not fall back to the snapshot"
        );
    }

    /// The dial is deliberately outside the mode's six, because it moves the tone
    /// curve's split as well as the wavelet — so neither entering, nor a write while the
    /// mode is on, nor leaving may touch it.
    #[test]
    fn the_background_grain_dial_is_never_managed() {
        let mut settings = CaptureSettings::default();
        settings.denoise.background_grain = 0.8;
        set(&mut settings, true, &Plugins::none());
        assert_eq!(settings.denoise.background_grain, 0.8, "entering moved the dial");

        settings.denoise.background_grain = 0.2;
        reconcile(&mut settings);
        assert_eq!(settings.denoise.background_grain, 0.2, "reconcile moved the dial");

        set(&mut settings, false, &Plugins::none());
        assert_eq!(
            settings.denoise.background_grain, 0.2,
            "leaving restored a dial position the observer had since changed"
        );
    }

    #[test]
    fn reconcile_leaves_unmanaged_siblings_untouched() {
        let mut settings = CaptureSettings::default();
        set(&mut settings, true, &Plugins::none());

        settings.denoise.chroma_strength = 0.75;
        settings.sensor_correction.hot_pixel_sigma = 9.0;
        settings.sensor_correction.superpixel_debayer = true;
        reconcile(&mut settings);

        assert_eq!(settings.denoise.chroma_strength, 0.75);
        assert_eq!(settings.sensor_correction.hot_pixel_sigma, 9.0);
        assert!(settings.sensor_correction.superpixel_debayer);
    }

    /// `Recovering` included: the resume takes its frames with these settings.
    #[test]
    fn stacking_captures_conflict_with_entering_the_mode() {
        for state in [
            CaptureState::Starting,
            CaptureState::Capturing,
            CaptureState::Recovering,
        ] {
            assert!(
                conflicts_with_capture(CaptureMode::Stacking, StackingType::DeepSky, state),
                "{state:?}"
            );
        }
        assert!(conflicts_with_capture(
            CaptureMode::Stacking,
            StackingType::Comet,
            CaptureState::Capturing
        ));
    }

    /// Wanderer throws its stack away when the mount moves, but it still integrates
    /// between resets.
    #[test]
    fn wanderer_captures_conflict_too() {
        assert!(conflicts_with_capture(
            CaptureMode::Wanderer,
            StackingType::DeepSky,
            CaptureState::Capturing
        ));
    }

    /// The guard is about the accumulator, not about the camera being busy.
    #[test]
    fn live_view_never_conflicts() {
        assert!(!conflicts_with_capture(
            CaptureMode::LiveView,
            StackingType::DeepSky,
            CaptureState::Capturing
        ));
    }

    /// After Stop the capture loop checks `is_cancelled()` before it snapshots settings, so
    /// no frame taken under the mode can follow.
    #[test]
    fn a_capture_taking_no_new_frames_never_conflicts() {
        for state in [
            CaptureState::Idle,
            CaptureState::Stopping,
            CaptureState::Error,
        ] {
            assert!(
                !conflicts_with_capture(CaptureMode::Stacking, StackingType::DeepSky, state),
                "{state:?}"
            );
        }
    }

    /// Planetary never flattens lines, so the mode has nothing to take from its stack.
    #[test]
    fn a_planetary_stack_never_conflicts() {
        assert!(!conflicts_with_capture(
            CaptureMode::Stacking,
            StackingType::Planetary,
            CaptureState::Capturing
        ));
    }

    async fn app_state_stacking_under_focus_mode(
        stacking_type: StackingType,
    ) -> (AppState, crate::disk_writer::DiskWriter) {
        let (state, disk_writer) = AppState::new_for_testing();
        {
            let mut settings = state.settings.write().await;
            settings.stacking = true;
            settings.stacking_type = stacking_type;
            settings.sensor_correction.fpn_removal = true;
            set(&mut settings, true, &Plugins::none());
        }
        (state, disk_writer)
    }

    /// The gap `update_settings` cannot close: the mode is on under a stack the capture loop
    /// is about to feed. The frame's snapshot must already carry the correction.
    #[tokio::test]
    async fn a_frame_about_to_stack_under_the_mode_takes_it_off_first() {
        let (state, _disk_writer) = app_state_stacking_under_focus_mode(StackingType::DeepSky).await;
        let mut events = state.events.subscribe();

        let snapshot = state.settings_for_new_frame().await;

        assert!(!snapshot.focus_mode);
        assert!(snapshot.sensor_correction.fpn_removal);
        assert!(!state.settings.read().await.focus_mode);
        let sent: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        assert!(sent
            .iter()
            .any(|event| matches!(event, ServerEvent::SettingsUpdated)));
        assert!(sent
            .iter()
            .any(|event| matches!(event, ServerEvent::FocusModeLeft)));
    }

    #[tokio::test]
    async fn a_planetary_frame_keeps_the_mode_and_announces_nothing() {
        let (state, _disk_writer) =
            app_state_stacking_under_focus_mode(StackingType::Planetary).await;
        let mut events = state.events.subscribe();

        let snapshot = state.settings_for_new_frame().await;

        assert!(snapshot.focus_mode);
        assert!(state.settings.read().await.focus_mode);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn leaving_for_a_stacking_capture_restores_the_managed_settings() {
        let mut settings = CaptureSettings {
            stacking: true,
            wanderer_mode: false,
            ..mixed()
        };
        let before = mixed();
        set(&mut settings, true, &Plugins::none());

        assert!(leave_if_conflicting(&mut settings, CaptureState::Starting, &Plugins::none()));

        assert!(!settings.focus_mode);
        assert!(settings.focus_mode_snapshot.is_none());
        assert_eq!(
            settings.sensor_correction.fpn_removal,
            before.sensor_correction.fpn_removal
        );
        assert_eq!(settings.denoise.luma_strength, before.denoise.luma_strength);
    }

    #[test]
    fn live_view_and_an_idle_rig_keep_the_mode() {
        let mut settings = CaptureSettings {
            stacking: false,
            ..mixed()
        };
        set(&mut settings, true, &Plugins::none());

        assert!(!leave_if_conflicting(&mut settings, CaptureState::Capturing, &Plugins::none()));
        assert!(settings.focus_mode);

        settings.stacking = true;
        assert!(!leave_if_conflicting(&mut settings, CaptureState::Idle, &Plugins::none()));
        assert!(settings.focus_mode);
        assert!(settings.focus_mode_snapshot.is_some());
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
