use std::collections::HashMap;

use super::camera_profile::CameraCaptureProfile;
use super::capture_mode::{CaptureMode, RawFrameSaving};
use super::display_settings::{
    EyepieceSettings, Resolution, DEFAULT_PREVIEW_RESOLUTION, DEFAULT_STREAMING_RESOLUTION,
};
use super::focus_mode::FocusModeSnapshot;
use super::StreamKind;
use night_amplifier_core::background::BackgroundExtractionAlgorithm;
use night_amplifier_core::camera::{CaptureConfig, DualSamplingMode};
use night_amplifier_core::cfa::SensorCorrectionSettings;
use night_amplifier_core::planetary::AlignmentRoi;
use night_amplifier_core::push_to::TelescopeSettings;
use night_amplifier_core::render::denoise::DenoiseSettings;
use night_amplifier_core::render::{SaturationBoostConfig, StretchAggressiveness};
use night_amplifier_core::stacking::{RejectionMethod, StackingType, WeightingPreset};

/// Capture settings that can be modified during a session.
///
/// Also the schema of `settings.json` and of the settings API's answer, so a new setting
/// is one field here. A key missing from a file takes its value from `Default`, which
/// keeps a file written by any older build loadable; `settings_persistence::migrate`
/// rewrites the keys an older build named differently.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct CaptureSettings {
    /// Exposure time in microseconds
    pub exposure_us: u64,
    /// Gain value
    pub gain: i32,
    /// Offset (black level)
    pub offset: i32,
    /// Binning factor
    pub bin: u8,
    /// Enable auto-stretch for preview
    pub auto_stretch: bool,
    /// Enable live stacking
    pub stacking: bool,
    /// Sigma for rejection during stacking
    pub rejection_sigma: f32,
    /// Outlier rejection method. A fresh install and a file without the key both start on
    /// [`RejectionMethod::best_available`].
    pub rejection_method: RejectionMethod,
    /// Enable background subtraction
    pub background_subtraction: bool,
    /// Algorithm for background extraction (GridBilinear or RBF)
    pub background_extraction_algorithm: BackgroundExtractionAlgorithm,
    /// Which capture modes write their raw frames to disk (FITS format)
    pub raw_frame_saving: RawFrameSaving,
    /// Enable saving stacked image to disk (FITS + PNG)
    pub save_stacked_image: bool,
    /// Stacking type (Deep Sky or Planetary)
    pub stacking_type: StackingType,
    /// Quality-based frame weighting preset for stacking
    pub weighting_preset: WeightingPreset,
    /// Auto stretch aggressiveness (Low, Medium, High)
    pub stretch_aggressiveness: StretchAggressiveness,
    /// Auto Stretch intensity multiplier (0.0 to 1.0, where 0.0 means no color boost, default 0.3)
    pub auto_stretch_intensity: f32,
    /// Enable shadow saturation boost
    pub saturation_boost: bool,
    /// Shadow saturation boost strength (0.0-1.0)
    pub saturation_boost_strength: f32,
    /// Use simulated camera instead of a real one
    pub use_simulated_camera: bool,
    /// Number of images to preload for simulated camera
    pub simulated_preload_images: usize,
    /// Show the focus image when waiting for frames
    pub show_focus_image: bool,
    /// Force showing the focus image even when the stream is active
    pub force_focus_image_now: bool,
    /// Whether the cooler should be active during capture (cooled cameras only)
    pub cooler_enabled: bool,
    /// Target sensor temperature in Celsius (None means "no target set")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_temp_c: Option<f64>,
    /// Bypass the 5 °C/min ramp and cool/warm as fast as the hardware allows.
    /// Defeats sensor-stress / condensation protections — user-opt-in only.
    pub cooler_fast_mode: bool,
    /// Manual override for camera sensor mode. None means "derive from stacking_type".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensor_mode_override: Option<DualSamplingMode>,
    /// Region of interest for comet nucleus tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comet_roi: Option<AlignmentRoi>,
    /// Region of interest for planetary alignment
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planetary_roi: Option<AlignmentRoi>,
    /// Enable auto tracking of planetary ROI
    pub planetary_auto_tracking: bool,
    /// Enable multi-point alignment for planetary (Pro only)
    pub planetary_multi_point_alignment: bool,
    /// Whether anti-dew heater is enabled
    pub dew_heater_enabled: bool,
    /// Anti-dew heater power level (0-100)
    pub dew_heater_power: i32,
    /// Enable "Wanderer" mode for automatic stack reset on movement
    pub wanderer_mode: bool,
    /// Reopen the camera automatically after it drops out mid-session (a USB
    /// stall or an unplug), instead of leaving the session dead until someone
    /// clicks Connect.
    pub auto_reconnect: bool,
    /// After an automatic reconnect, resume the capture that was interrupted —
    /// same mode, same settings, same stack. Without this the camera comes
    /// back but the session does not.
    pub auto_resume_capture: bool,
    /// Corrections applied to the raw sensor mosaic, before demosaic
    pub sensor_correction: SensorCorrectionSettings,
    /// Spatial denoising, applied at stream resolution inside the encoders
    pub denoise: DenoiseSettings,
    /// How much sensor resolution the preview pipeline may bin away before it runs
    pub preview_resolution: Resolution,
    /// The JPEG size every `/` and `/eyepiece` client receives
    pub streaming_resolution: Resolution,
    /// Eyepiece view settings
    pub eyepiece: EyepieceSettings,
    /// Telescope and camera parameters for FOV calculation
    pub telescope: TelescopeSettings,
    /// Per-camera telescope profiles keyed by camera name
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub camera_telescope_profiles: HashMap<String, TelescopeSettings>,
    /// Per-camera capture profiles keyed by `"{provider}/{model_name}"`.
    /// Holds the seven hardware-specific fields so switching between cameras
    /// doesn't leak stale values (e.g. cooler=true from a cooled camera into
    /// an uncooled one).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub camera_profiles: HashMap<String, CameraCaptureProfile>,
    /// The guide camera's live hardware values.
    ///
    /// The flat fields above are the *main* camera's; the guide camera cannot share them
    /// because both are connected at once and a guide sub is typically seconds where an
    /// imaging sub is minutes. Same shape as a stored profile, so `camera_profiles`
    /// remembers it across reconnects exactly as it does the main camera's.
    pub guide_camera: CameraCaptureProfile,
    /// Name of the last active camera (for profile inheritance)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_camera_name: Option<String>,
    /// Whether the user has accepted the End User License Agreement
    pub eula_accepted: bool,
    /// INDI server host
    pub indi_server_host: String,
    /// INDI server port
    pub indi_server_port: u16,
    /// Hold the cosmetic pipeline stages off while framing and focusing.
    ///
    /// Invariant: true exactly when [`Self::focus_mode_snapshot`] is `Some`. Drive it
    /// through [`super::focus_mode::set`], never by assignment — a bare write leaves the
    /// snapshot behind and the next toggle restores the wrong values.
    pub focus_mode: bool,
    /// What the managed settings were before Focus/Finder mode overwrote them. Never sent
    /// to a client: see `SettingsResponse`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "api-schema", schemars(skip))]
    pub focus_mode_snapshot: Option<FocusModeSnapshot>,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        // The flat fields *are* the main camera's profile, so they take their defaults
        // from the same place the guide camera's do — two literal lists would drift.
        let hw = CameraCaptureProfile::default();
        Self {
            exposure_us: hw.exposure_us,
            gain: hw.gain,
            offset: hw.offset,
            bin: hw.bin,
            auto_stretch: true,
            stacking: true,
            rejection_sigma: 2.5,
            // Not `RejectionMethod::default()`, which is `None` — right for a
            // `StackingConfig`, but this field is what a session *asks* for, and every Pro
            // session has run sigma clipping since the plugin existed.
            rejection_method: RejectionMethod::best_available(),
            background_subtraction: true,
            background_extraction_algorithm: BackgroundExtractionAlgorithm::default(),
            preview_resolution: DEFAULT_PREVIEW_RESOLUTION,
            streaming_resolution: DEFAULT_STREAMING_RESOLUTION,
            raw_frame_saving: RawFrameSaving::default(),
            save_stacked_image: false,
            stacking_type: StackingType::default(),
            weighting_preset: WeightingPreset::default(),
            stretch_aggressiveness: StretchAggressiveness::default(),
            auto_stretch_intensity: 0.3,
            saturation_boost: false,
            saturation_boost_strength: 0.5,
            use_simulated_camera: false,
            simulated_preload_images: 5,
            show_focus_image: true,
            force_focus_image_now: false,
            cooler_enabled: hw.cooler_enabled,
            target_temp_c: hw.target_temp_c,
            cooler_fast_mode: hw.cooler_fast_mode,
            sensor_mode_override: hw.sensor_mode_override,
            comet_roi: None,
            planetary_roi: None,
            planetary_auto_tracking: true,
            planetary_multi_point_alignment: false,
            dew_heater_enabled: hw.dew_heater_enabled,
            dew_heater_power: hw.dew_heater_power,
            wanderer_mode: false,
            auto_reconnect: true,
            auto_resume_capture: true,
            sensor_correction: SensorCorrectionSettings::default(),
            denoise: DenoiseSettings::default(),
            eyepiece: EyepieceSettings::default(),
            telescope: TelescopeSettings::default(),
            camera_telescope_profiles: HashMap::new(),
            camera_profiles: HashMap::new(),
            guide_camera: hw,
            last_camera_name: None,
            eula_accepted: false,
            indi_server_host: "127.0.0.1".to_string(),
            indi_server_port: 7624,
            focus_mode: false,
            focus_mode_snapshot: None,
        }
    }
}

impl CaptureSettings {
    /// Every value inside the range the rest of the server assumes, however it arrived:
    /// a request, or an older or hand-edited file. Idempotent, so every path runs it.
    pub fn sanitized(mut self) -> Self {
        self.rejection_sigma = self.rejection_sigma.clamp(0.5, 10.0);
        self.auto_stretch_intensity = self.auto_stretch_intensity.clamp(0.0, 1.0);
        self.saturation_boost_strength = self.saturation_boost_strength.clamp(0.0, 1.0);
        self.simulated_preload_images = self.simulated_preload_images.max(1);
        self.main_camera_profile().sanitized().apply_to(&mut self);
        self.guide_camera = self.guide_camera.sanitized();
        for profile in self.camera_profiles.values_mut() {
            *profile = profile.clone().sanitized();
        }
        self.sensor_correction = self.sensor_correction.sanitized();
        self.denoise = self.denoise.sanitized();
        self.eyepiece = self.eyepiece.sanitized();
        // `focus_mode == focus_mode_snapshot.is_some()` holds both ways: a flag with no
        // snapshot has nothing to restore from, and a snapshot with no flag describes
        // values already in force.
        if !(self.focus_mode && self.focus_mode_snapshot.is_some()) {
            self.focus_mode = false;
            self.focus_mode_snapshot = None;
        }
        self
    }

    /// The size a stream family is sent at: Streaming Resolution for JPEG, Eyepiece
    /// Streaming Resolution for lossless.
    ///
    /// Encoders read it from the live settings, never from a frame's snapshot: the capture
    /// loop snapshots when an exposure *starts*, so a change landed one exposure late (up to
    /// two minutes at 60 s subs), and a client joining in between flipped new -> old -> new.
    pub fn stream_resolution(&self, kind: StreamKind) -> Resolution {
        match kind {
            StreamKind::Jpeg => self.streaming_resolution,
            StreamKind::Lossless => self.eyepiece.stream_resolution.resolution(),
        }
    }

    /// Which of the three capture modes this session is running in.
    pub fn capture_mode(&self) -> CaptureMode {
        CaptureMode::from_flags(self.stacking, self.wanderer_mode)
    }

    /// Whether raw frames captured under these settings go to disk.
    pub fn saves_raw_frames(&self) -> bool {
        self.raw_frame_saving.saves(self.capture_mode())
    }

    /// Whether the finished stack goes to disk.
    ///
    /// Stacking mode only: Live view never builds one, and Wanderer throws its stack
    /// away every time the telescope moves, so there is no single result to write.
    pub fn saves_stacked_image(&self) -> bool {
        self.save_stacked_image && self.capture_mode() == CaptureMode::Stacking
    }

    /// Whether the guide camera's raw frames go to disk.
    pub fn saves_guide_raw_frames(&self) -> bool {
        self.raw_frame_saving.saves(CaptureMode::Guide)
    }

    /// Whether the disk writer has anything at all to do.
    ///
    /// The guide switch counts even though `initialize_capture_session` — the only place
    /// that used to set the writer's master flag — runs at *main* capture start. A guide
    /// camera saving subs with no main capture running is an ordinary case, and without
    /// this the writer would silently refuse every one of its frames.
    pub fn disk_writing_enabled(&self) -> bool {
        self.saves_raw_frames() || self.saves_stacked_image() || self.saves_guide_raw_frames()
    }

    /// Get the saturation boost config based on current settings
    pub fn saturation_boost_config(&self) -> SaturationBoostConfig {
        if self.saturation_boost {
            SaturationBoostConfig {
                enabled: true,
                strength: self.saturation_boost_strength,
                shadow_peak: 0.15,
                upper_limit: 0.4,
            }
        } else {
            SaturationBoostConfig::default()
        }
    }

    /// Convert the main camera's settings to a capture config.
    pub fn to_capture_config(&self) -> CaptureConfig {
        self.to_capture_config_for(super::CameraRole::Main)
    }

    /// Build the capture config for one camera role.
    pub fn to_capture_config_for(&self, role: super::CameraRole) -> CaptureConfig {
        self.to_capture_config_with(&self.profile_for(role), role)
    }

    /// The main camera's hardware fields, read out of the flat settings.
    ///
    /// The flat fields *are* the main camera's live values; this just views them through
    /// the same shape the guide camera stores its own in, so one config builder serves
    /// both.
    pub fn main_camera_profile(&self) -> CameraCaptureProfile {
        CameraCaptureProfile {
            exposure_us: self.exposure_us,
            gain: self.gain,
            offset: self.offset,
            bin: self.bin,
            cooler_enabled: self.cooler_enabled,
            target_temp_c: self.target_temp_c,
            sensor_mode_override: self.sensor_mode_override,
            cooler_fast_mode: self.cooler_fast_mode,
            dew_heater_enabled: self.dew_heater_enabled,
            dew_heater_power: self.dew_heater_power,
        }
    }

    /// The hardware profile for one camera role.
    pub fn profile_for(&self, role: super::CameraRole) -> CameraCaptureProfile {
        match role {
            super::CameraRole::Main => self.main_camera_profile(),
            super::CameraRole::Guide => self.guide_camera.clone(),
        }
    }

    /// Build a capture config from one camera's hardware profile plus the
    /// session-wide fields (simulator preload) both cameras share.
    pub fn to_capture_config_with(
        &self,
        profile: &CameraCaptureProfile,
        role: super::CameraRole,
    ) -> CaptureConfig {
        // "Low Noise" dual-sampling trades frame rate for read noise, so it's only
        // worth selecting while frames are actually integrated, not during live view.
        // `stacking` alone covers both "Stacking" and "Wanderer" UI modes (frontend
        // always sets `stacking: true` with `wanderer_mode: true`, see
        // `CaptureControls.vue`'s `applyStackingMode`); OR-ing `wanderer_mode` in
        // directly would wrongly fire on `stacking: false, wanderer_mode: true`. The
        // flags describe the *imaging* session only — applying this to the guide
        // camera once flipped it to `LowReadoutNoise`, buying read noise it never needed.
        let is_actively_stacking = role == super::CameraRole::Main
            && self.stacking
            && self.stacking_type.supports_stacking();
        let sensor_mode = profile.sensor_mode_override.unwrap_or_else(|| {
            if is_actively_stacking {
                self.stacking_type.desired_sensor_mode()
            } else {
                DualSamplingMode::Normal
            }
        });
        let mut config = CaptureConfig::new()
            .with_exposure_us(profile.exposure_us)
            .with_gain(profile.gain)
            .with_offset(profile.offset)
            .with_bin(profile.bin)
            .with_simulated_preload_images(self.simulated_preload_images)
            .with_cooler(profile.cooler_enabled)
            .with_sensor_mode(sensor_mode);
        if let Some(temp) = profile.target_temp_c {
            config.target_temp_c = Some(temp);
        }
        config
    }

    /// Telescope parameters to plan a plate solve against, for the camera that is
    /// producing the solve frames.
    ///
    /// The guide camera sits on a different scope, so handing ASTAP the main scope's
    /// FOV for guide-scope frames sends it hunting at the wrong scale. The per-camera
    /// map is what the equipment UI already writes; this is the first thing that reads
    /// it, falling back to the flat block for a camera with no profile of its own.
    pub fn solver_telescope(&self, camera_name: Option<&str>) -> TelescopeSettings {
        camera_name
            .and_then(|name| self.camera_telescope_profiles.get(name))
            .cloned()
            .unwrap_or_else(|| self.telescope.clone())
    }

    /// Give a newly connected camera its own telescope profile, from what it reports.
    /// Without this, unprofiled cameras share the flat block's FOV — on 2026-09-07
    /// that gave the guide camera the main camera's 0.5152 deg for a rig imaging
    /// 1.4516 deg, and the 2.8x-wrong FOV *failed* the solve outright (19 min with no
    /// solve, vs. 0.1 s normally). Only the sensor is seeded (all the camera states);
    /// focal length/Barlow carry over from the flat block only when it already
    /// describes this sensor. Never overwrites an existing profile; no pixel size
    /// (simulator: zero) leaves it alone.
    pub fn ensure_camera_telescope_profile(
        &mut self,
        camera_name: &str,
        info: &night_amplifier_core::camera::CameraInfo,
    ) -> bool {
        if self.camera_telescope_profiles.contains_key(camera_name) {
            return false;
        }
        let positive = |v: f64| (v > 0.0).then_some(v as f32);
        let Some(pixel_size_y_um) = positive(info.pixel_size_y_um) else {
            return false;
        };

        // Two bodies of the same model in the two roles still land on one key: the
        // sensor cannot tell them apart, and only a focal length the user enters can.
        let same_sensor = self
            .telescope
            .pixel_size_y_um
            .is_some_and(|py| (py - pixel_size_y_um).abs() < 1e-4)
            && self.telescope.sensor_height_px == Some(info.max_height);

        self.camera_telescope_profiles.insert(
            camera_name.to_string(),
            TelescopeSettings {
                focal_length_mm: same_sensor.then_some(self.telescope.focal_length_mm).flatten(),
                pixel_size_x_um: positive(info.pixel_size_x_um),
                pixel_size_y_um: Some(pixel_size_y_um),
                sensor_width_px: Some(info.max_width),
                sensor_height_px: Some(info.max_height),
                barlow_coeff: same_sensor
                    .then_some(self.telescope.barlow_coeff)
                    .flatten()
                    .or(Some(1.0)),
            },
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EyepieceStreamResolution;

    #[test]
    fn each_stream_family_reads_its_own_resolution_setting() {
        let mut settings = CaptureSettings::default();
        assert_eq!(settings.stream_resolution(StreamKind::Jpeg), Resolution::Qhd1440);
        assert_eq!(settings.stream_resolution(StreamKind::Lossless), Resolution::Qhd1440);

        settings.streaming_resolution = Resolution::Native;
        settings.eyepiece.stream_resolution = EyepieceStreamResolution::Uhd2160;
        assert_eq!(settings.stream_resolution(StreamKind::Jpeg), Resolution::Native);
        assert_eq!(settings.stream_resolution(StreamKind::Lossless), Resolution::Uhd2160);
    }

    #[test]
    fn capture_config_picks_lrn_for_deep_sky() {
        let settings = CaptureSettings {
            stacking_type: StackingType::DeepSky,
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::LowReadoutNoise));
    }

    #[test]
    fn capture_config_picks_lrn_for_comet() {
        let settings = CaptureSettings {
            stacking_type: StackingType::Comet,
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::LowReadoutNoise));
    }

    #[test]
    fn capture_config_picks_normal_for_planetary() {
        let settings = CaptureSettings {
            stacking_type: StackingType::Planetary,
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::Normal));
    }

    #[test]
    fn sensor_mode_override_trumps_stacking_type_auto() {
        let settings = CaptureSettings {
            stacking_type: StackingType::Planetary,
            sensor_mode_override: Some(DualSamplingMode::LowReadoutNoise),
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::LowReadoutNoise));
    }

    #[test]
    fn capture_config_picks_normal_for_deep_sky_when_not_stacking() {
        let settings = CaptureSettings {
            stacking_type: StackingType::DeepSky,
            stacking: false,
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::Normal));
    }

    #[test]
    fn capture_config_picks_normal_for_comet_when_not_stacking() {
        let settings = CaptureSettings {
            stacking_type: StackingType::Comet,
            stacking: false,
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::Normal));
    }

    #[test]
    fn capture_config_picks_lrn_for_deep_sky_in_wanderer_mode() {
        // Wanderer mode always sets `stacking: true` alongside `wanderer_mode:
        // true` (enforced by the frontend, see `CaptureControls.vue`) — this
        // pins that "Stacking or Wanderer" both resolve through `stacking`.
        let settings = CaptureSettings {
            stacking_type: StackingType::DeepSky,
            stacking: true,
            wanderer_mode: true,
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::LowReadoutNoise));
    }

    /// `stacking: false` is Live view whatever `wanderer_mode` says. The frontend never
    /// sends that pair, but the field is settable over the API on its own, and every
    /// storage gate now resolves through this — so it has to land somewhere definite
    /// rather than fall through to Stacking.
    #[test]
    fn capture_mode_reads_the_stacking_pair() {
        let mode = |stacking, wanderer_mode| {
            CaptureSettings {
                stacking,
                wanderer_mode,
                ..CaptureSettings::default()
            }
            .capture_mode()
        };

        assert_eq!(mode(false, false), CaptureMode::LiveView);
        assert_eq!(mode(false, true), CaptureMode::LiveView);
        assert_eq!(mode(true, true), CaptureMode::Wanderer);
        assert_eq!(mode(true, false), CaptureMode::Stacking);
    }

    /// The whole point of the feature: each mode reads its own switch and no other.
    #[test]
    fn saves_raw_frames_pairs_each_mode_with_its_own_switch() {
        let cases = [
            (false, false, CaptureMode::LiveView),
            (true, true, CaptureMode::Wanderer),
            (true, false, CaptureMode::Stacking),
        ];

        for (stacking, wanderer_mode, mode) in cases {
            for enabled_mode in [
                CaptureMode::LiveView,
                CaptureMode::Wanderer,
                CaptureMode::Stacking,
            ] {
                let raw_frame_saving = RawFrameSaving {
                    live_view: enabled_mode == CaptureMode::LiveView,
                    wanderer: enabled_mode == CaptureMode::Wanderer,
                    stacking: enabled_mode == CaptureMode::Stacking,
                    guide: false,
                };
                let settings = CaptureSettings {
                    stacking,
                    wanderer_mode,
                    raw_frame_saving,
                    ..CaptureSettings::default()
                };
                assert_eq!(
                    settings.saves_raw_frames(),
                    enabled_mode == mode,
                    "capturing in {mode:?} with only {enabled_mode:?} enabled"
                );
            }
        }
    }

    /// A Live or Wanderer session has no finished stack, so the stacked-image switch
    /// must stay inert there even now that raw saving is not gated on the mode.
    #[test]
    fn saves_stacked_image_stays_stacking_only() {
        let saves = |stacking, wanderer_mode| {
            CaptureSettings {
                stacking,
                wanderer_mode,
                save_stacked_image: true,
                ..CaptureSettings::default()
            }
            .saves_stacked_image()
        };

        assert!(!saves(false, false));
        assert!(!saves(true, true));
        assert!(saves(true, false));
    }

    /// Raw saving in a mode that writes no stacked image still has to bring the disk
    /// writer up — this is the condition that used to read `stacking && !wanderer_mode`.
    #[test]
    fn disk_writing_is_enabled_by_raw_saving_alone_in_live_view() {
        let settings = CaptureSettings {
            stacking: false,
            save_stacked_image: true,
            raw_frame_saving: RawFrameSaving {
                live_view: true,
                ..Default::default()
            },
            ..CaptureSettings::default()
        };

        assert!(settings.disk_writing_enabled());
        assert!(
            !settings.saves_stacked_image(),
            "the stacked image must not follow raw saving into Live view"
        );
    }

    #[test]
    fn disk_writing_is_disabled_when_the_current_mode_saves_nothing() {
        let settings = CaptureSettings {
            stacking: false,
            raw_frame_saving: RawFrameSaving {
                stacking: true,
                ..Default::default()
            },
            ..CaptureSettings::default()
        };

        assert!(!settings.disk_writing_enabled());
    }

    #[test]
    fn sensor_mode_override_wins_even_when_not_stacking() {
        let settings = CaptureSettings {
            stacking_type: StackingType::DeepSky,
            stacking: false,
            sensor_mode_override: Some(DualSamplingMode::LowReadoutNoise),
            ..CaptureSettings::default()
        };
        let config = settings.to_capture_config();
        assert_eq!(config.sensor_mode, Some(DualSamplingMode::LowReadoutNoise));
    }

    // ---- the fallback that put two cameras on one Push-To rig key -------------------
    //
    // Reported 2026-09-07. A guide camera (Neptune-C II, 2.9um, 2712x1538, 1.4516 deg)
    // and a main camera (Ares-C PRO on 1250mm, 0.5152 deg) were both connected. Only
    // one had a telescope profile, so `solver_telescope` answered with the flat block
    // for the other, and the solver's per-rig FOV cache — keyed on exactly these
    // fields — held one entry for both. The guide camera was then handed 0.5152 deg
    // and returned no plate solve at all for the next 19 minutes.

    fn guide_optics() -> TelescopeSettings {
        TelescopeSettings {
            focal_length_mm: None,
            pixel_size_x_um: Some(2.9),
            pixel_size_y_um: Some(2.9),
            sensor_width_px: Some(2712),
            sensor_height_px: Some(1538),
            barlow_coeff: Some(1.0),
        }
    }

    fn main_optics() -> TelescopeSettings {
        TelescopeSettings {
            focal_length_mm: Some(1250.0),
            pixel_size_x_um: Some(3.76),
            pixel_size_y_um: Some(3.76),
            sensor_width_px: Some(3008),
            sensor_height_px: Some(3008),
            barlow_coeff: Some(1.0),
        }
    }

    #[test]
    fn a_camera_without_a_profile_is_given_the_other_cameras_optics() {
        // The fallback itself is deliberate and unchanged — a single-camera user with
        // only the flat block filled in must still get their optics. It is only unsafe
        // while a *second* camera can reach it, which is what
        // `ensure_camera_telescope_profile` prevents at connect.
        let mut settings = CaptureSettings {
            telescope: main_optics(),
            ..CaptureSettings::default()
        };
        settings
            .camera_telescope_profiles
            .insert("Ares-C PRO".to_string(), main_optics());

        assert_eq!(
            settings.solver_telescope(Some("Neptune-C II")),
            main_optics(),
            "the guide camera is described by the main camera's scope and sensor"
        );
    }

    #[test]
    fn two_cameras_without_profiles_are_indistinguishable_to_the_solver() {
        // The state the FOV cache cannot survive: both cameras answer with the same
        // optics, so both key to the same rig and share one remembered FOV. Reaching
        // it now requires a camera that reports no sensor at all — see
        // `seeding_gives_two_unprofiled_cameras_distinct_optics`.
        let settings = CaptureSettings {
            telescope: guide_optics(),
            ..CaptureSettings::default()
        };

        assert_eq!(
            settings.solver_telescope(Some("Neptune-C II")),
            settings.solver_telescope(Some("Ares-C PRO")),
        );
    }

    #[test]
    fn a_profiled_camera_is_described_by_its_own_optics() {
        // What the map is for, and the only case that keys the two rigs apart.
        let mut settings = CaptureSettings {
            telescope: main_optics(),
            ..CaptureSettings::default()
        };
        settings
            .camera_telescope_profiles
            .insert("Neptune-C II".to_string(), guide_optics());

        assert_eq!(settings.solver_telescope(Some("Neptune-C II")), guide_optics());
        assert_eq!(settings.solver_telescope(Some("Ares-C PRO")), main_optics());
    }

    // ---- seeding a rig identity from the sensor the camera reports -----------------

    fn camera(name: &str, pixel_um: f64, width: u32, height: u32) -> night_amplifier_core::camera::CameraInfo {
        night_amplifier_core::camera::CameraInfo {
            name: name.to_string(),
            max_width: width,
            max_height: height,
            pixel_size_x_um: pixel_um,
            pixel_size_y_um: pixel_um,
            ..Default::default()
        }
    }

    /// The two bodies of the 2026-09-07 session.
    fn neptune() -> night_amplifier_core::camera::CameraInfo {
        camera("Neptune-C II", 2.9, 2712, 1538)
    }

    fn ares() -> night_amplifier_core::camera::CameraInfo {
        camera("Ares-C PRO", 3.76, 3008, 3008)
    }

    #[test]
    fn seeding_gives_two_unprofiled_cameras_distinct_optics() {
        // The fix: connect both bodies against a flat block describing one of them,
        // and they no longer answer with the same sensor — so they no longer share a
        // Push-To rig key, or the FOV filed under it.
        let mut settings = CaptureSettings {
            telescope: main_optics(),
            ..CaptureSettings::default()
        };

        assert!(settings.ensure_camera_telescope_profile("Ares-C PRO", &ares()));
        assert!(settings.ensure_camera_telescope_profile("Neptune-C II", &neptune()));

        let guide = settings.solver_telescope(Some("Neptune-C II"));
        let main = settings.solver_telescope(Some("Ares-C PRO"));
        assert_ne!(guide, main);
        assert_eq!(guide.pixel_size_y_um, Some(2.9));
        assert_eq!(guide.sensor_height_px, Some(1538));
        assert_eq!(main.pixel_size_y_um, Some(3.76));
        assert_eq!(main.sensor_height_px, Some(3008));
    }

    #[test]
    fn seeding_keeps_the_focal_length_for_the_camera_the_flat_block_describes() {
        // The main camera's own scope is in the flat block, so carrying it over is
        // correct — dropping it would cost that camera its configured FOV.
        let mut settings = CaptureSettings {
            telescope: main_optics(),
            ..CaptureSettings::default()
        };
        assert!(settings.ensure_camera_telescope_profile("Ares-C PRO", &ares()));

        assert_eq!(
            settings.solver_telescope(Some("Ares-C PRO")).focal_length_mm,
            Some(1250.0)
        );
    }

    #[test]
    fn seeding_withholds_a_focal_length_that_belongs_to_the_other_camera() {
        // 1250 mm is the main scope. Attaching it to the guide body would compute a
        // 0.20 deg field for one that measures 1.45 deg — the same class of error the
        // whole change exists to stop, arriving by a different route.
        let mut settings = CaptureSettings {
            telescope: main_optics(),
            ..CaptureSettings::default()
        };
        assert!(settings.ensure_camera_telescope_profile("Neptune-C II", &neptune()));

        let guide = settings.solver_telescope(Some("Neptune-C II"));
        assert_eq!(guide.focal_length_mm, None);
        assert_eq!(guide.barlow_coeff, Some(1.0));
    }

    #[test]
    fn seeding_never_overwrites_what_the_user_configured() {
        // The equipment UI owns this map. A connect fills a gap in it, nothing more.
        let mut settings = CaptureSettings::default();
        let configured = TelescopeSettings {
            focal_length_mm: Some(176.0),
            ..guide_optics()
        };
        settings
            .camera_telescope_profiles
            .insert("Neptune-C II".to_string(), configured.clone());

        assert!(!settings.ensure_camera_telescope_profile("Neptune-C II", &neptune()));
        assert_eq!(settings.solver_telescope(Some("Neptune-C II")), configured);
    }

    #[test]
    fn a_camera_that_reports_no_pixel_size_is_left_to_the_fallback() {
        // The simulator advertises zero. Seeding a profile from that would file a
        // camera under a sensor size of nothing, which is worse than not seeding.
        let mut settings = CaptureSettings {
            telescope: main_optics(),
            ..CaptureSettings::default()
        };

        assert!(!settings.ensure_camera_telescope_profile("Simulator", &camera("Simulator", 0.0, 2712, 1538)));
        assert!(settings.camera_telescope_profiles.is_empty());
        assert_eq!(settings.solver_telescope(Some("Simulator")), main_optics());
    }
}
