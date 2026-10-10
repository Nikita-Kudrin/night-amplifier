//! Common camera types and configurations: what a camera is ([`CameraInfo`]), what it
//! reports ([`CameraStatus`]), what a capture asks of it ([`CaptureConfig`]) and what it
//! hands back ([`RawFrame`]).

mod capture_config;
mod raw_frame;

pub use capture_config::{
    parse_usb_bandwidth_percent, usb_bandwidth_override, usb_bandwidth_within, AcquisitionMode,
    CaptureConfig, FRAME_STALL_ALLOWANCE, TRANSFER_FLOOR_BYTES_PER_SEC, USB_BANDWIDTH_ENV,
};
pub use raw_frame::{BufferPool, PooledBuffer, RawFrame};

use crate::CfaPattern;
use serde::{Deserialize, Serialize};

/// Camera sensor type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorType {
    /// Monochrome sensor
    Mono,
    /// Color sensor with Bayer CFA
    Color,
}

/// Dual sampling sensor mode (Player One terminology). Only meaningful for
/// cameras that advertise sensor-mode selection — other providers ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DualSamplingMode {
    /// Higher frame rate, lower dynamic range — suited for planetary imaging.
    Normal,
    /// Lower readout noise and higher dynamic range — suited for deep-sky and comet imaging.
    LowReadoutNoise,
}

/// A sensor-mode slot reported by the underlying camera SDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorMode {
    /// Zero-based index used to select the mode in SDK calls.
    pub index: u32,
    /// Short display name (e.g. "Normal", "LRN").
    pub name: String,
    /// Longer description, suitable for tooltips.
    pub description: String,
}

/// Image format from camera
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// 8-bit raw data
    Raw8,
    /// 16-bit raw data
    Raw16,
    /// 8-bit RGB (for color cameras in RGB mode)
    Rgb24,
}

impl ImageFormat {
    /// Bytes per pixel for this format
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            ImageFormat::Raw8 => 1,
            ImageFormat::Raw16 => 2,
            ImageFormat::Rgb24 => 3,
        }
    }

    /// Pick the best raw capture format from the camera's supported list.
    ///
    /// Prefers `Raw16` for its higher dynamic range; falls back to `Raw8`.
    /// Returns `None` when the camera advertises neither (pathological case —
    /// we never want to capture in hardware-debayered RGB24 for astronomy
    /// because calibration and debayering are done in software from raw data).
    pub fn best_raw_format(supported: &[ImageFormat]) -> Option<ImageFormat> {
        if supported.contains(&ImageFormat::Raw16) {
            Some(ImageFormat::Raw16)
        } else if supported.contains(&ImageFormat::Raw8) {
            Some(ImageFormat::Raw8)
        } else {
            None
        }
    }
}

/// Camera information
#[derive(Debug, Clone)]
pub struct CameraInfo {
    /// Camera name/model
    pub name: String,
    /// Camera ID (SDK-specific identifier)
    pub id: i32,
    /// Maximum image width
    pub max_width: u32,
    /// Maximum image height
    pub max_height: u32,
    /// Pixel size X in micrometers
    pub pixel_size_x_um: f64,
    /// Pixel size Y in micrometers
    pub pixel_size_y_um: f64,
    /// Sensor type (mono/color)
    pub sensor_type: SensorType,
    /// Bayer pattern (for color cameras)
    pub bayer_pattern: Option<CfaPattern>,
    /// Whether the camera supports cooling
    pub has_cooler: bool,
    /// Minimum target temperature in Celsius (None when has_cooler is false or vendor SDK does not expose it)
    pub min_temp_c: Option<f64>,
    /// Maximum target temperature in Celsius (None when has_cooler is false or vendor SDK does not expose it)
    pub max_temp_c: Option<f64>,
    /// Whether the camera has a mechanical shutter
    pub has_shutter: bool,
    /// Whether the camera supports USB3
    pub is_usb3: bool,
    /// Bit depth of the sensor
    pub bit_depth: u8,
    /// Supported bin modes (e.g., [1, 2, 4])
    pub supported_bins: Vec<u8>,
    /// Supported image formats
    pub supported_formats: Vec<ImageFormat>,
    /// Minimum exposure time in microseconds
    pub min_exposure_us: u64,
    /// Maximum exposure time in microseconds
    pub max_exposure_us: u64,
    /// Minimum gain value
    pub min_gain: i32,
    /// Maximum gain value
    pub max_gain: i32,
    /// Unity gain value (where e/ADU = 1)
    pub unity_gain: i32,
    /// HCG (High Conversion Gain) threshold
    pub hcg_gain: i32,
    /// Sensor modes advertised by the camera. Empty when mode selection is not supported.
    pub sensor_modes: Vec<SensorMode>,
    /// Whether the camera supports anti-dew heater
    pub has_dew_heater: bool,
    /// Vendor serial number, when the SDK exposes one without opening the device.
    /// What tells two bodies apart once USB re-enumeration has reordered the device
    /// list — see `camera::identity`.
    pub serial: Option<String>,
}

impl Default for CameraInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: 0,
            max_width: 0,
            max_height: 0,
            pixel_size_x_um: 0.0,
            pixel_size_y_um: 0.0,
            sensor_type: SensorType::Mono,
            bayer_pattern: None,
            has_cooler: false,
            min_temp_c: None,
            max_temp_c: None,
            has_shutter: false,
            is_usb3: false,
            bit_depth: 8,
            supported_bins: vec![1],
            supported_formats: vec![ImageFormat::Raw8],
            min_exposure_us: 1,
            max_exposure_us: 3600_000_000,
            min_gain: 0,
            max_gain: 100,
            unity_gain: 0,
            hcg_gain: 0,
            sensor_modes: Vec::new(),
            has_dew_heater: false,
            serial: None,
        }
    }
}

/// Gain presets from the camera
#[derive(Debug, Clone, Copy, Default)]
pub struct GainPresets {
    /// Gain at highest dynamic range (usually 0)
    pub highest_dr: i32,
    /// Gain at HCG (High Conversion Gain) mode
    pub hcg: i32,
    /// Unity gain (e/ADU = 1)
    pub unity: i32,
    /// Gain at lowest read noise
    pub lowest_rn: i32,
    /// Offset at highest dynamic range
    pub offset_highest_dr: i32,
    /// Offset at HCG mode
    pub offset_hcg: i32,
    /// Offset at unity gain
    pub offset_unity: i32,
    /// Offset at lowest read noise
    pub offset_lowest_rn: i32,
}

/// Sensor temperatures outside this band are a glitch, not a reading: a just-reopened
/// Ares-C PRO reported -300 °C on 2026-09-20, and a cooling ramp seeded from it would have
/// commanded the TEC to its floor. Wide enough for any TEC and a sensor in the sun.
pub const PLAUSIBLE_SENSOR_TEMP_C: std::ops::RangeInclusive<f64> = -80.0..=90.0;

/// Camera status information
#[derive(Debug, Clone, Default)]
pub struct CameraStatus {
    /// Current sensor temperature in Celsius
    pub temperature_c: f64,
    /// Cooler power percentage (0-100)
    pub cooler_power: Option<f64>,
    /// Whether cooler is currently active
    pub cooler_on: bool,
    /// Current exposure in progress
    pub is_exposing: bool,
    /// Current gain setting
    pub current_gain: i32,
    /// Current offset setting
    pub current_offset: i32,
    /// Current exposure time in microseconds
    pub current_exposure_us: u64,
    /// Whether anti-dew heater is currently active
    pub dew_heater_on: bool,
}

impl CameraStatus {
    /// Whether `temperature_c` can be a real sensor temperature. See
    /// [`PLAUSIBLE_SENSOR_TEMP_C`].
    pub fn has_plausible_temperature(&self) -> bool {
        PLAUSIBLE_SENSOR_TEMP_C.contains(&self.temperature_c)
    }
}

#[cfg(test)]
mod tests;
