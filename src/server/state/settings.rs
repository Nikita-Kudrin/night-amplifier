use std::collections::HashMap;

use crate::background::BackgroundExtractionAlgorithm;
use crate::camera::{CameraInfo, CaptureConfig, DualSamplingMode};
use crate::planetary::AlignmentRoi;
use crate::render::{SaturationBoostConfig, StretchAggressiveness};
use crate::stacking::{RejectionMethod, StackingType, WeightingPreset};

/// Capture settings that can be modified during a session
#[derive(Debug, Clone)]
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
    /// Outlier rejection method (None, SigmaClip, etc.)
    pub rejection_method: RejectionMethod,
    /// Enable background subtraction
    pub background_subtraction: bool,
    /// Algorithm for background extraction (GridBilinear or RBF)
    pub background_extraction_algorithm: BackgroundExtractionAlgorithm,
    /// Enable saving raw frames to disk (FITS format)
    pub save_raw_frames: bool,
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
    pub target_temp_c: Option<f64>,
    /// Bypass the 5 °C/min ramp and cool/warm as fast as the hardware allows.
    /// Defeats sensor-stress / condensation protections — user-opt-in only.
    pub cooler_fast_mode: bool,
    /// Manual override for camera sensor mode. None means "derive from stacking_type".
    pub sensor_mode_override: Option<DualSamplingMode>,
    /// Region of interest for comet nucleus tracking
    pub comet_roi: Option<AlignmentRoi>,
    /// Region of interest for planetary alignment
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
    /// Eyepiece view settings
    pub eyepiece: EyepieceSettings,
    /// Telescope and camera parameters for FOV calculation
    pub telescope: TelescopeSettings,
    /// Per-camera telescope profiles keyed by camera name
    pub camera_telescope_profiles: HashMap<String, TelescopeSettings>,
    /// Per-camera capture profiles keyed by `"{provider}/{model_name}"`.
    /// Holds the seven hardware-specific fields so switching between cameras
    /// doesn't leak stale values (e.g. cooler=true from a cooled camera into
    /// an uncooled one).
    pub camera_profiles: HashMap<String, CameraCaptureProfile>,
    /// Name of the last active camera (for profile inheritance)
    pub last_camera_name: Option<String>,
    /// Whether the user has accepted the End User License Agreement
    pub eula_accepted: bool,
    /// INDI server host
    pub indi_server_host: String,
    /// INDI server port
    pub indi_server_port: u16,
}

/// Hardware-specific capture settings scoped to a single camera
/// (keyed by `"{provider}/{model_name}"` in `CaptureSettings::camera_profiles`).
///
/// These are swapped into the flat `CaptureSettings` fields on connect so the
/// rest of the pipeline (capture loop, cooler monitor, UI DTO) stays unaware
/// of the per-camera indirection.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CameraCaptureProfile {
    pub exposure_us: u64,
    pub gain: i32,
    pub offset: i32,
    pub bin: u8,
    pub cooler_enabled: bool,
    pub target_temp_c: Option<f64>,
    pub sensor_mode_override: Option<DualSamplingMode>,
    #[serde(default)]
    pub cooler_fast_mode: bool,
    #[serde(default = "default_dew_heater_enabled")]
    pub dew_heater_enabled: bool,
    #[serde(default = "default_dew_heater_power")]
    pub dew_heater_power: i32,
}

fn default_dew_heater_enabled() -> bool {
    true
}

fn default_dew_heater_power() -> i32 {
    10
}

impl CameraCaptureProfile {
    /// Capture the seven flat fields from `settings`, zeroing any field the
    /// camera can't support: cooler fields on uncooled cameras, and
    /// `sensor_mode_override` on cameras that advertise no sensor modes.
    /// The clamp prevents stale values from a previous camera's session
    /// from leaking into a freshly-seeded profile.
    pub fn from_settings_clamped(settings: &CaptureSettings, info: &CameraInfo) -> Self {
        let (cooler_enabled, target_temp_c) = if info.has_cooler {
            (settings.cooler_enabled, settings.target_temp_c)
        } else {
            (false, None)
        };
        let sensor_mode_override = if info.sensor_modes.is_empty() {
            None
        } else {
            settings.sensor_mode_override
        };
        let (dew_heater_enabled, dew_heater_power) = if info.has_dew_heater {
            (settings.dew_heater_enabled, settings.dew_heater_power)
        } else {
            (false, 10)
        };
        Self {
            exposure_us: settings.exposure_us,
            gain: settings.gain,
            offset: settings.offset,
            bin: settings.bin,
            cooler_enabled,
            target_temp_c,
            sensor_mode_override,
            cooler_fast_mode: settings.cooler_fast_mode,
            dew_heater_enabled,
            dew_heater_power,
        }
    }

    /// Write the fields onto the flat `CaptureSettings`.
    pub fn apply_to(&self, settings: &mut CaptureSettings) {
        settings.exposure_us = self.exposure_us;
        settings.gain = self.gain;
        settings.offset = self.offset;
        settings.bin = self.bin;
        settings.cooler_enabled = self.cooler_enabled;
        settings.target_temp_c = self.target_temp_c;
        settings.sensor_mode_override = self.sensor_mode_override;
        settings.cooler_fast_mode = self.cooler_fast_mode;
        settings.dew_heater_enabled = self.dew_heater_enabled;
        settings.dew_heater_power = self.dew_heater_power;
    }
}

/// Telescope and camera parameters for FOV calculation
///
/// `PartialEq` so callers can act on a telescope block that actually *changed*
/// rather than one that was merely present in the request. Every field feeds the
/// FOV, so any difference matters and a whole-struct comparison is the right test.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct TelescopeSettings {
    /// Telescope focal length in mm
    #[serde(default)]
    pub focal_length_mm: Option<f32>,
    /// Pixel size X in micrometers (manual override or from camera database)
    #[serde(default)]
    pub pixel_size_x_um: Option<f32>,
    /// Pixel size Y in micrometers (manual override or from camera database)
    #[serde(default)]
    pub pixel_size_y_um: Option<f32>,
    /// Sensor width in pixels
    #[serde(default)]
    pub sensor_width_px: Option<u32>,
    /// Sensor height in pixels
    #[serde(default)]
    pub sensor_height_px: Option<u32>,
    /// Barlow/reducer coefficient (effective_fl = focal_length * coeff; default 1.0)
    #[serde(default)]
    pub barlow_coeff: Option<f32>,
}

/// Settings specifically for the eyepiece view feature
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EyepieceSettings {
    /// Enable Binoview
    pub binoview: bool,
    /// Screen width
    pub screen_width: f32,
    /// Screen height
    pub screen_height: f32,
    /// Measurement unit (e.g. "mm", "inches")
    pub screen_measurement: String,
    /// Screen resolution X
    pub screen_resolution_x: u32,
    /// Screen resolution Y
    pub screen_resolution_y: u32,
    /// Enable Circular view
    #[serde(default = "default_circular_view")]
    pub circular_view: bool,
    /// Dark background enhancement intensity (0.0 to 1.0)
    #[serde(default = "default_intensity")]
    pub intensity: f32,
}

fn default_intensity() -> f32 {
    0.3
}

fn default_circular_view() -> bool {
    true
}

impl Default for EyepieceSettings {
    fn default() -> Self {
        Self {
            binoview: true,
            screen_width: 140.0,
            screen_height: 67.0,
            screen_measurement: "mm".to_string(),
            screen_resolution_x: 2880,
            screen_resolution_y: 1440,
            circular_view: true,
            intensity: 0.3,
        }
    }
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            exposure_us: 1_000_000,
            gain: 0,
            offset: 10,
            bin: 1,
            auto_stretch: true,
            stacking: true,
            rejection_sigma: 2.5,
            rejection_method: RejectionMethod::default(),
            background_subtraction: true,
            background_extraction_algorithm: BackgroundExtractionAlgorithm::default(),
            save_raw_frames: false,
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
            cooler_enabled: false,
            target_temp_c: None,
            cooler_fast_mode: false,
            sensor_mode_override: None,
            comet_roi: None,
            planetary_roi: None,
            planetary_auto_tracking: true,
            planetary_multi_point_alignment: false,
            dew_heater_enabled: true,
            dew_heater_power: 10,
            wanderer_mode: false,
            auto_reconnect: true,
            auto_resume_capture: true,
            eyepiece: EyepieceSettings::default(),
            telescope: TelescopeSettings::default(),
            camera_telescope_profiles: HashMap::new(),
            camera_profiles: HashMap::new(),
            last_camera_name: None,
            eula_accepted: false,
            indi_server_host: "127.0.0.1".to_string(),
            indi_server_port: 7624,
        }
    }
}

impl CaptureSettings {
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

    /// Convert to camera capture config
    pub fn to_capture_config(&self) -> CaptureConfig {
        // "Low Noise" dual-sampling trades frame rate for read noise, so it's
        // only worth selecting while frames are actually being integrated —
        // not during a raw live-view feed under a stacking-capable target
        // type. `stacking` alone covers both the "Stacking" and "Wanderer"
        // UI modes: the frontend always sets `stacking: true` whenever it
        // sets `wanderer_mode: true` (see `CaptureControls.vue`'s
        // `applyStackingMode`), so there's no case Wanderer needs to add here
        // — and OR-ing `wanderer_mode` in directly would be wrong for the
        // orthogonal-misuse case of `stacking: false, wanderer_mode: true`
        // (no frames integrated there either).
        let is_actively_stacking = self.stacking && self.stacking_type.supports_stacking();
        let sensor_mode = self.sensor_mode_override.unwrap_or_else(|| {
            if is_actively_stacking {
                self.stacking_type.desired_sensor_mode()
            } else {
                DualSamplingMode::Normal
            }
        });
        let mut config = CaptureConfig::new()
            .with_exposure_us(self.exposure_us)
            .with_gain(self.gain)
            .with_offset(self.offset)
            .with_bin(self.bin)
            .with_simulated_preload_images(self.simulated_preload_images)
            .with_cooler(self.cooler_enabled)
            .with_sensor_mode(sensor_mode);
        if let Some(temp) = self.target_temp_c {
            config.target_temp_c = Some(temp);
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
