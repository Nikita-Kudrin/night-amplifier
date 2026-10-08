//! Per-camera capture settings — exposure, gain, cooler, dew heater — swapped into
//! `CaptureSettings` when that camera connects.

use super::CaptureSettings;
use night_amplifier_core::camera::{CameraInfo, DualSamplingMode};

/// Hardware-specific capture settings scoped to a single camera
/// (keyed by `"{provider}/{model_name}"` in `CaptureSettings::camera_profiles`).
///
/// These are swapped into the flat `CaptureSettings` fields on connect so the
/// rest of the pipeline (capture loop, cooler monitor, UI DTO) stays unaware
/// of the per-camera indirection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
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

/// Written out rather than derived: a derived `Default` is all zeros, and `bin: 0` with
/// `exposure_us: 0` is a config every SDK rejects in `CaptureConfig::validate` — which
/// is what a guide camera got before it had ever been configured. The `serde(default)`
/// attributes above cover a different case (a field missing from a settings file) and
/// never run for `Default::default()`.
impl Default for CameraCaptureProfile {
    fn default() -> Self {
        Self {
            exposure_us: 1_000_000,
            gain: 0,
            offset: 10,
            bin: 1,
            cooler_enabled: false,
            target_temp_c: None,
            sensor_mode_override: None,
            cooler_fast_mode: false,
            dew_heater_enabled: default_dew_heater_enabled(),
            dew_heater_power: default_dew_heater_power(),
        }
    }
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

    /// This profile with the cooler setpoint and heater power inside what the hardware
    /// takes. Idempotent.
    pub fn sanitized(mut self) -> Self {
        self.target_temp_c = self.target_temp_c.map(|t| t.clamp(-60.0, 30.0));
        self.dew_heater_power = self.dew_heater_power.clamp(0, 100);
        self
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
