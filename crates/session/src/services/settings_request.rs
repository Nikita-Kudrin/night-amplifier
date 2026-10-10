//! The body of `POST /api/settings`: every field optional, applied by
//! [`super::SettingsService`]. A new setting gets a field here — see AGENTS.md
//! "Settings Persistence".

use std::collections::HashMap;

use serde::Deserialize;

use night_amplifier_core::background::BackgroundExtractionAlgorithm;
use night_amplifier_core::camera::DualSamplingMode;
use night_amplifier_core::cfa::SensorCorrectionSettings;
use night_amplifier_core::planetary::AlignmentRoi;
use night_amplifier_core::push_to::TelescopeSettings;
use night_amplifier_core::render::denoise::DenoiseSettings;
use night_amplifier_core::render::StretchAggressiveness;
use crate::state::{
    CameraCaptureProfile, CameraRole, EyepieceSettings, RawFrameSaving, Resolution,
};
use night_amplifier_core::stacking::{RejectionMethod, StackingType, WeightingPreset};

/// Update settings request
#[derive(Debug, Deserialize, Default)]
pub struct UpdateSettingsRequest {
    /// Which camera the hardware fields below (exposure, gain, offset, bin, cooler and
    /// dew heater) apply to. Absent means the imaging camera.
    #[serde(default)]
    pub camera_role: Option<CameraRole>,
    #[serde(default)]
    pub exposure_us: Option<u64>,
    #[serde(default)]
    pub gain: Option<i32>,
    #[serde(default)]
    pub offset: Option<i32>,
    #[serde(default)]
    pub bin: Option<u8>,
    #[serde(default)]
    pub auto_stretch: Option<bool>,

    #[serde(default)]
    pub stacking: Option<bool>,
    #[serde(default)]
    pub rejection_sigma: Option<f32>,
    #[serde(default)]
    pub rejection_method: Option<RejectionMethod>,
    #[serde(default)]
    pub background_subtraction: Option<bool>,
    /// Algorithm for background extraction
    #[serde(default)]
    pub background_extraction_algorithm: Option<BackgroundExtractionAlgorithm>,
    /// Which capture modes write their raw frames to disk
    #[serde(default)]
    pub raw_frame_saving: Option<RawFrameSaving>,
    #[serde(default)]
    pub save_stacked_image: Option<bool>,
    #[serde(default)]
    pub stacking_type: Option<StackingType>,

    /// Quality-based frame weighting preset for stacking
    #[serde(default)]
    pub weighting_preset: Option<WeightingPreset>,
    /// Auto stretch aggressiveness (Low, Medium, High)
    #[serde(default)]
    pub stretch_aggressiveness: Option<StretchAggressiveness>,
    /// Auto Stretch intensity multiplier
    #[serde(default)]
    pub auto_stretch_intensity: Option<f32>,
    /// Enable shadow saturation boost
    #[serde(default)]
    pub saturation_boost: Option<bool>,
    /// Shadow saturation boost strength (0.0-1.0)
    #[serde(default)]
    pub saturation_boost_strength: Option<f32>,
    /// Use simulated camera
    #[serde(default)]
    pub use_simulated_camera: Option<bool>,
    /// Number of images to preload for simulated camera
    #[serde(default)]
    pub simulated_preload_images: Option<usize>,
    /// Show the focus image when waiting for frames
    #[serde(default)]
    pub show_focus_image: Option<bool>,
    /// Force showing the focus image even when the stream is active
    #[serde(default)]
    pub force_focus_image_now: Option<bool>,
    /// Region of interest for comet nucleus tracking (used in Comet stacking mode)
    #[serde(default)]
    pub comet_roi: Option<AlignmentRoi>,
    /// Region of interest for planetary alignment (used in Planetary stacking mode)
    #[serde(default)]
    pub planetary_roi: Option<AlignmentRoi>,
    /// Enable auto tracking of planetary ROI
    #[serde(default)]
    pub planetary_auto_tracking: Option<bool>,
    /// Enable multi-point alignment for planetary (Pro only)
    #[serde(default)]
    pub planetary_multi_point_alignment: Option<bool>,
    /// Enable "Wanderer" mode
    #[serde(default)]
    pub wanderer_mode: Option<bool>,
    pub auto_reconnect: Option<bool>,
    pub auto_resume_capture: Option<bool>,

    #[serde(default)]
    pub sensor_correction: Option<SensorCorrectionSettings>,

    #[serde(default)]
    pub denoise: Option<DenoiseSettings>,

    #[serde(default)]
    pub preview_resolution: Option<Resolution>,

    #[serde(default)]
    pub streaming_resolution: Option<Resolution>,

    #[serde(default)]
    pub eyepiece: Option<EyepieceSettings>,

    #[serde(default)]
    pub telescope: Option<TelescopeSettings>,

    /// Per-camera telescope profiles keyed by camera name
    #[serde(default)]
    pub camera_telescope_profiles: Option<HashMap<String, TelescopeSettings>>,
    /// Per-camera capture profiles (mainly for tests to seed the map without
    /// going through a camera connect).
    #[serde(default)]
    pub camera_profiles: Option<HashMap<String, CameraCaptureProfile>>,
    /// Name of the last active camera
    #[serde(default)]
    pub last_camera_name: Option<String>,
    /// Whether the cooler should be active during capture
    #[serde(default)]
    pub cooler_enabled: Option<bool>,
    /// Target sensor temperature in Celsius. Use `Some(None)` is not possible via JSON;
    /// pass `null` to clear by sending `target_temp_c_clear` instead.
    #[serde(default)]
    pub target_temp_c: Option<f64>,
    /// Bypass the 5 °C/min cool/warm ramp (advanced users only)
    #[serde(default)]
    pub cooler_fast_mode: Option<bool>,
    #[serde(default)]
    pub sensor_mode_override: Option<DualSamplingMode>,
    /// Whether anti-dew heater is enabled
    #[serde(default)]
    pub dew_heater_enabled: Option<bool>,
    /// Anti-dew heater power level (0-100)
    #[serde(default)]
    pub dew_heater_power: Option<i32>,
    /// Whether the user has accepted the End User License Agreement
    #[serde(default)]
    pub eula_accepted: Option<bool>,
    /// INDI server host
    #[serde(default)]
    pub indi_server_host: Option<String>,
    /// INDI server port
    #[serde(default)]
    pub indi_server_port: Option<u16>,
    /// Enter or leave Focus/Finder mode. Applied after every other field in the
    /// request, so it wins over a managed setting sent alongside it.
    #[serde(default)]
    pub focus_mode: Option<bool>,
}
