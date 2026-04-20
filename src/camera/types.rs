//! Common camera types and configurations

use crate::CfaPattern;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::error::{CameraError, CameraResult};

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

/// Configuration for image capture
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Exposure time in microseconds
    pub exposure_us: u64,
    /// Gain value
    pub gain: i32,
    /// Offset value (black level)
    pub offset: i32,
    /// Binning factor (1 = no binning, 2 = 2x2, etc.)
    pub bin: u8,
    /// Image format to capture
    pub format: ImageFormat,
    /// Region of interest (start_x, start_y, width, height)
    /// None means full frame
    pub roi: Option<(u32, u32, u32, u32)>,
    /// Target temperature in Celsius (for cooled cameras)
    pub target_temp_c: Option<f64>,
    /// Enable cooler
    pub cooler_enabled: bool,
    /// Timeout for exposure completion
    pub timeout: Duration,
    /// Enable high speed mode (may reduce image quality)
    pub high_speed: bool,
    /// Enable hardware binning (vs software binning)
    pub hardware_bin: bool,
    /// Number of images to preload for simulated camera
    pub simulated_preload_images: usize,
    /// Desired dual-sampling sensor mode. None leaves the camera's current mode unchanged.
    pub sensor_mode: Option<DualSamplingMode>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            exposure_us: 1_000_000, // 1 second
            gain: 0,
            offset: 10,
            bin: 1,
            format: ImageFormat::Raw16,
            roi: None,
            target_temp_c: None,
            cooler_enabled: false,
            timeout: Duration::from_secs(120),
            high_speed: false,
            hardware_bin: true,
            simulated_preload_images: 5,
            sensor_mode: None,
        }
    }
}

impl CaptureConfig {
    /// Create a new capture configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set exposure time in microseconds
    pub fn with_exposure_us(mut self, exposure_us: u64) -> Self {
        self.exposure_us = exposure_us;
        self
    }

    /// Set exposure time from Duration
    pub fn with_exposure(mut self, exposure: Duration) -> Self {
        self.exposure_us = exposure.as_micros() as u64;
        self
    }

    /// Set gain value
    pub fn with_gain(mut self, gain: i32) -> Self {
        self.gain = gain;
        self
    }

    /// Set offset (black level)
    pub fn with_offset(mut self, offset: i32) -> Self {
        self.offset = offset;
        self
    }

    /// Set binning factor
    pub fn with_bin(mut self, bin: u8) -> Self {
        self.bin = bin;
        self
    }

    /// Set image format
    pub fn with_format(mut self, format: ImageFormat) -> Self {
        self.format = format;
        self
    }

    /// Set region of interest
    pub fn with_roi(mut self, start_x: u32, start_y: u32, width: u32, height: u32) -> Self {
        self.roi = Some((start_x, start_y, width, height));
        self
    }

    /// Set target cooling temperature
    pub fn with_target_temp(mut self, temp_c: f64) -> Self {
        self.target_temp_c = Some(temp_c);
        self.cooler_enabled = true;
        self
    }

    /// Enable or disable cooler
    pub fn with_cooler(mut self, enabled: bool) -> Self {
        self.cooler_enabled = enabled;
        self
    }

    /// Set timeout for exposure
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Enable high speed mode
    pub fn with_high_speed(mut self, enabled: bool) -> Self {
        self.high_speed = enabled;
        self
    }

    /// Enable hardware binning
    pub fn with_hardware_bin(mut self, enabled: bool) -> Self {
        self.hardware_bin = enabled;
        self
    }

    /// Set simulated preload images count
    pub fn with_simulated_preload_images(mut self, count: usize) -> Self {
        self.simulated_preload_images = count;
        self
    }

    /// Set the desired dual-sampling sensor mode
    pub fn with_sensor_mode(mut self, mode: DualSamplingMode) -> Self {
        self.sensor_mode = Some(mode);
        self
    }

    /// Validate configuration against camera capabilities
    pub fn validate(&self, info: &CameraInfo) -> CameraResult<()> {
        // Validate exposure
        if self.exposure_us < info.min_exposure_us {
            return Err(CameraError::InvalidParameter {
                name: "exposure_us".to_string(),
                message: format!(
                    "Exposure {} us is below minimum {} us",
                    self.exposure_us, info.min_exposure_us
                ),
            });
        }
        if self.exposure_us > info.max_exposure_us {
            return Err(CameraError::InvalidParameter {
                name: "exposure_us".to_string(),
                message: format!(
                    "Exposure {} us exceeds maximum {} us",
                    self.exposure_us, info.max_exposure_us
                ),
            });
        }

        // Validate gain
        if self.gain < info.min_gain || self.gain > info.max_gain {
            return Err(CameraError::InvalidParameter {
                name: "gain".to_string(),
                message: format!(
                    "Gain {} is outside valid range [{}, {}]",
                    self.gain, info.min_gain, info.max_gain
                ),
            });
        }

        // Validate binning
        if !info.supported_bins.contains(&self.bin) {
            return Err(CameraError::InvalidParameter {
                name: "bin".to_string(),
                message: format!(
                    "Binning {} is not supported. Available: {:?}",
                    self.bin, info.supported_bins
                ),
            });
        }

        // Validate format
        if !info.supported_formats.contains(&self.format) {
            return Err(CameraError::InvalidParameter {
                name: "format".to_string(),
                message: format!(
                    "Format {:?} is not supported. Available: {:?}",
                    self.format, info.supported_formats
                ),
            });
        }

        // Validate ROI
        if let Some((x, y, w, h)) = self.roi {
            let max_w = info.max_width / self.bin as u32;
            let max_h = info.max_height / self.bin as u32;

            if x + w > max_w || y + h > max_h {
                return Err(CameraError::InvalidParameter {
                    name: "roi".to_string(),
                    message: format!(
                        "ROI ({}, {}, {}, {}) exceeds sensor bounds ({}x{} with bin {})",
                        x, y, w, h, max_w, max_h, self.bin
                    ),
                });
            }

            // ROI dimensions must be even for most sensors
            if w % 2 != 0 || h % 2 != 0 {
                return Err(CameraError::InvalidParameter {
                    name: "roi".to_string(),
                    message: "ROI width and height must be even".to_string(),
                });
            }
        }

        // Validate cooling
        if self.cooler_enabled && !info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }

        // Validate sensor mode: only meaningful when the camera advertises modes.
        if self.sensor_mode.is_some() && info.sensor_modes.is_empty() {
            return Err(CameraError::ParameterNotSupported(
                "sensor_mode".to_string(),
            ));
        }

        Ok(())
    }
}

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
}

#[cfg(test)]
mod tests {
    include!("types_tests.rs");
}
