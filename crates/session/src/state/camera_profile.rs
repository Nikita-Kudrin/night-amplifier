//! Per-camera capture settings — exposure, gain, cooler, dew heater — swapped into
//! `CaptureSettings` when that camera connects.

use super::CaptureSettings;
use night_amplifier_core::camera::DualSamplingMode;
use night_amplifier_core::render::planetary::{DaylightBalance, DaylightBalanceSource};

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
    /// The camera's colour response to sunlight, which Planetary renders through
    /// (`render::planetary::daylight`): the prior until read off the Moon.
    #[serde(default)]
    pub daylight_balance: DaylightBalance,
    /// Whether `daylight_balance` was measured or is the prior. A file from before the
    /// measure held identity: read as the prior, it renders a sensible colour rather than raw.
    #[serde(default)]
    pub daylight_balance_source: DaylightBalanceSource,
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
            daylight_balance: DaylightBalance::PRIOR,
            daylight_balance_source: DaylightBalanceSource::Prior,
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
    /// This profile with the cooler setpoint and heater power inside what the hardware
    /// takes, and an unmeasured balance at the prior whatever value it carried. Idempotent.
    pub fn sanitized(mut self) -> Self {
        self.target_temp_c = self.target_temp_c.map(|t| t.clamp(-60.0, 30.0));
        self.dew_heater_power = self.dew_heater_power.clamp(0, 100);
        self.daylight_balance = match self.daylight_balance_source {
            DaylightBalanceSource::Prior => DaylightBalance::PRIOR,
            DaylightBalanceSource::Moon => self.daylight_balance.sanitized(),
        };
        self
    }

    /// This profile with its balance back at the prior: a new camera must not render through
    /// the last one's sensor, and a mono camera has no colour to balance.
    pub fn uncalibrated(mut self) -> Self {
        self.daylight_balance = DaylightBalance::PRIOR;
        self.daylight_balance_source = DaylightBalanceSource::Prior;
        self
    }

    /// This profile rendering through `balance`, read off the Moon.
    pub fn calibrated_on_the_moon(mut self, balance: DaylightBalance) -> Self {
        self.daylight_balance = balance.sanitized();
        self.daylight_balance_source = DaylightBalanceSource::Moon;
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
        settings.daylight_balance = self.daylight_balance;
        settings.daylight_balance_source = self.daylight_balance_source;
    }
}
