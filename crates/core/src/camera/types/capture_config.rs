//! What a capture asks of a camera: exposure, gain, format, region, how frames are taken,
//! and how long one may take before it counts as stalled.

use std::time::Duration;

use super::{CameraInfo, DualSamplingMode, ImageFormat};
use crate::camera::error::{CameraError, CameraResult};

/// Fixed part of [`CaptureConfig::stall_budget`]: readout and SDK hand-off on top of
/// the exposure. Healthy captures were measured jittering up to ~1.6 s, so 3 s only
/// trips on a frame that is not coming.
pub const FRAME_STALL_ALLOWANCE: Duration = Duration::from_secs(3);

/// Slowest link a frame transfer is budgeted for: well under USB 2.0's ~35 MB/s, so a
/// busy shared bus still fits.
pub const TRANSFER_FLOOR_BYTES_PER_SEC: u64 = 10_000_000;

/// How a shim takes frames: a continuous video stream, or one exposure per `capture()`.
///
/// `Auto` is the long-standing rule, video for exposures up to one second. The others are
/// for field experiments: see `config_overrides::apply_guide_acquisition_override`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AcquisitionMode {
    #[default]
    Auto,
    Video,
    Snap,
}

impl AcquisitionMode {
    /// `auto`, `video` or `snap`, in any case.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "video" => Some(Self::Video),
            "snap" => Some(Self::Snap),
            _ => None,
        }
    }
}

/// Environment switch for a field test of USB traffic: the share of USB bandwidth a camera
/// may use, applied by providers exposing that control (Player One).
pub const USB_BANDWIDTH_ENV: &str = "NIGHT_AMPLIFIER_USB_BANDWIDTH";

/// A bandwidth percentage as [`USB_BANDWIDTH_ENV`] takes it: 1 to 100.
pub fn parse_usb_bandwidth_percent(value: &str) -> Option<u8> {
    value
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|percent| (1..=100).contains(percent))
}

/// The limit to set for a requested `percent`, or the camera's advertised `(min, max)` when
/// it lies outside it. `None` for a range the camera did not report: the SDK then decides.
/// The parser takes any 1-100 because the floor is per camera, and a refused value used to
/// log only "not applied".
pub fn usb_bandwidth_within(percent: u8, range: Option<(i64, i64)>) -> Result<i64, (i64, i64)> {
    let percent = i64::from(percent);
    match range {
        Some((min, max)) if !(min..=max).contains(&percent) => Err((min, max)),
        _ => Ok(percent),
    }
}

/// [`USB_BANDWIDTH_ENV`], read once. A value that does not parse is reported and ignored.
pub fn usb_bandwidth_override() -> Option<u8> {
    static PERCENT: std::sync::OnceLock<Option<u8>> = std::sync::OnceLock::new();
    *PERCENT.get_or_init(|| {
        let raw = std::env::var(USB_BANDWIDTH_ENV).ok()?;
        let percent = parse_usb_bandwidth_percent(&raw);
        if percent.is_none() {
            tracing::warn!(value = %raw, "Ignoring {USB_BANDWIDTH_ENV}: expected a percentage from 1 to 100");
        }
        percent
    })
}

/// Configuration for image capture
#[derive(Debug, Clone, PartialEq)]
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
    /// Enable high speed mode (may reduce image quality)
    pub high_speed: bool,
    /// Enable hardware binning (vs software binning)
    pub hardware_bin: bool,
    /// Number of images to preload for simulated camera
    pub simulated_preload_images: usize,
    /// Desired dual-sampling sensor mode. None leaves the camera's current mode unchanged.
    pub sensor_mode: Option<DualSamplingMode>,
    /// Video stream or single exposures. See [`AcquisitionMode`].
    pub acquisition: AcquisitionMode,
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
            high_speed: false,
            hardware_bin: true,
            simulated_preload_images: 5,
            sensor_mode: None,
            acquisition: AcquisitionMode::Auto,
        }
    }
}

impl CaptureConfig {
    /// Whether the shim runs a continuous video stream: for exposures up to one second,
    /// unless [`CaptureConfig::acquisition`] says otherwise.
    pub fn is_continuous(&self) -> bool {
        match self.acquisition {
            AcquisitionMode::Auto => self.exposure_us <= 1_000_000,
            AcquisitionMode::Video => true,
            AcquisitionMode::Snap => false,
        }
    }

    /// Width and height of the frame this config reads from `info`'s sensor.
    pub fn frame_dimensions(&self, info: &CameraInfo) -> (u32, u32) {
        if let Some((_, _, w, h)) = self.roi {
            return (w, h);
        }
        let bin = u32::from(self.bin.max(1));
        (info.max_width / bin, info.max_height / bin)
    }

    /// Bytes of sensor data one frame of this config transfers.
    pub fn frame_bytes(&self, info: &CameraInfo) -> usize {
        let (width, height) = self.frame_dimensions(info);
        let bytes_per_pixel = match self.format {
            ImageFormat::Raw8 => 1,
            ImageFormat::Raw16 => 2,
            ImageFormat::Rgb24 => 3,
        };
        (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(bytes_per_pixel)
    }

    /// How long a shim waits for a frame before declaring the stream stalled and restarting it in
    /// place. Timed from entering `capture()` (config reapply included), the same instant the watchdog
    /// starts, so it stays `WATCHDOG_SLACK` above it — timing from after the reapply let a slow reapply
    /// desync the clocks and the watchdog abandon the handle instead of costing one in-place retry.
    ///
    /// = exposure + [`FRAME_STALL_ALLOWANCE`] + transfer at [`TRANSFER_FLOOR_BYTES_PER_SEC`]; replaces
    /// `timeout + exposure` (120s + exposure, always beaten by the outer watchdog, costing a whole
    /// reconnect for one lost frame). The watchdog derives from this and must stay above it.
    pub fn stall_budget(&self, frame_bytes: usize) -> Duration {
        let transfer_ms = (frame_bytes as u64).saturating_mul(1000) / TRANSFER_FLOOR_BYTES_PER_SEC;
        Duration::from_micros(self.exposure_us)
            + FRAME_STALL_ALLOWANCE
            + Duration::from_millis(transfer_ms)
    }

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

    /// Whether this config differs from the last one applied to hardware and must be
    /// re-sent. Backends call this at the top of `capture()` to skip redundant
    /// blocking FFI/network round trips when nothing changed (the common case in a
    /// live-stacking session). Compares the whole struct by value, not per-field —
    /// per-field would tie `CaptureConfig` to which fields each backend actually
    /// uses; the cost is one extra reapply after a field changes that a backend
    /// happens to ignore, not a recurring one.
    pub fn should_reapply(&self, cached: Option<&CaptureConfig>) -> bool {
        cached != Some(self)
    }
}
