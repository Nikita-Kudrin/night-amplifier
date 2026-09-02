use std::collections::HashMap;

use super::capture_mode::{CaptureMode, RawFrameSaving};
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
    /// Corrections applied to the raw sensor mosaic, before demosaic
    pub sensor_correction: SensorCorrectionSettings,
    /// Spatial denoising, applied at stream resolution inside the encoders
    pub denoise: DenoiseSettings,
    /// How much sensor resolution the preview pipeline may bin away before it runs
    pub preview_resolution: PreviewResolution,
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

/// Corrections that run on the raw CFA mosaic, before demosaic.
///
/// These target the defects stacking cannot remove: a hot pixel and a readout
/// offset are in the same place in every sub, so averaging frames leaves them
/// exactly where they were. Both are only well defined on the mosaic — after
/// demosaic a hot site has already been smeared into a coloured 3x3 cross, and
/// neighbouring sensor rows have been mixed together.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SensorCorrectionSettings {
    /// Replace isolated hot samples with their same-colour neighbourhood mean.
    #[serde(default = "default_hot_pixel_rejection")]
    pub hot_pixel_rejection: bool,
    /// How far above its brightest same-colour neighbour a sample must sit to
    /// count as hot, in sigmas of that colour site's own noise.
    #[serde(default = "default_hot_pixel_sigma")]
    pub hot_pixel_sigma: f32,
    /// Flatten per-row and per-column readout offsets.
    #[serde(default = "default_fpn_removal")]
    pub fpn_removal: bool,
    /// Bin each 2x2 CFA quad to one RGB pixel instead of interpolating.
    ///
    /// Halves both dimensions. Free on a sensor that oversamples the eyepiece
    /// screen (IMX533's 3008² becomes 1504², still above 1440²) and a real loss
    /// on one that does not (IMX464 lands at 1356x769), which is why it is off
    /// by default.
    #[serde(default)]
    pub superpixel_debayer: bool,
}

fn default_hot_pixel_rejection() -> bool {
    true
}

fn default_hot_pixel_sigma() -> f32 {
    5.0
}

fn default_fpn_removal() -> bool {
    true
}

impl Default for SensorCorrectionSettings {
    fn default() -> Self {
        Self {
            hot_pixel_rejection: default_hot_pixel_rejection(),
            hot_pixel_sigma: default_hot_pixel_sigma(),
            fpn_removal: default_fpn_removal(),
            superpixel_debayer: false,
        }
    }
}

/// How much sensor resolution the preview pipeline is allowed to bin away.
///
/// The preview stages — background neutralisation, background subtraction, SCNR, the
/// black-point pass — all walk every sample of the frame, and the encoder then throws
/// away whatever the client's tier does not need. Binning first makes the whole pipeline
/// run on a quarter of the samples, and `Frame::downsample`'s box average is an exact
/// integer bin, so the pixels that survive are the ones the encoder would have produced
/// anyway.
///
/// # Why this is a setting and not the connected client set
///
/// It was the client set. `preview_bin_factor` was resolved fresh every iteration
/// against the largest bounding box any connected client had asked for, so a phone
/// joining or leaving flipped the factor between 1 and 2 mid-session — and the tone
/// curve is solved from the frame's median and MAD, which 2x2 binning moves. Measured on
/// a 1200x1200 sky with 400 stars, the solved `scale_lut` gained **25.7 % at the 1 %
/// input point and 16.1 % at 10 %**: a visible shadow lift, applied to *every* connected
/// viewer, triggered by somebody else's browser tab.
///
/// The resolution the preview is analysed at is a property of the session, so it is
/// chosen once and held. It also makes the guarantee `JpegTier::Original` documents —
/// native sensor resolution, no downsampling — structurally true at the default rather
/// than something the client-set arithmetic has to be careful about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewResolution {
    /// Never bin: every stage sees every sensor sample.
    ///
    /// The default, because the alternative silently costs resolution that no part of
    /// the UI asks for. An observer on a small board who would rather have the frame
    /// rate picks one of the others.
    #[default]
    Native,
    /// Bin toward a 4K preview. Only reaches sensors above ~8000 px on an edge.
    Uhd2160,
    /// Bin toward a 1440p preview. A 3008x3008 sensor bins by 2 here.
    Qhd1440,
    /// Bin toward a 1080p preview — the cheapest, and the floor of what any client
    /// asks for.
    Hd1080,
}

impl PreviewResolution {
    /// The bounding box the preview may be binned down toward, or `None` for
    /// [`Self::Native`].
    ///
    /// Deliberately the same boxes as [`crate::server::state::JpegTier`], because the
    /// binned frame is what every payload is encoded from: choosing a box no tier uses
    /// would throw away pixels for a size nothing serves.
    pub fn target_box(self) -> Option<(u32, u32)> {
        match self {
            Self::Native => None,
            Self::Uhd2160 => Some((3840, 2160)),
            Self::Qhd1440 => Some((2560, 1440)),
            Self::Hd1080 => Some((1920, 1080)),
        }
    }
}

/// Spatial denoising of the streamed image.
///
/// Runs in the encoders at the resolution the client asked for, not at sensor
/// resolution: the filters cost `1/4.5` as much there and the box downsample
/// has already halved the noise before they start. Both are off for
/// `StackingType::Planetary`, where the fine detail lucky imaging exists to
/// recover is exactly what a denoiser removes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DenoiseSettings {
    /// Guided-filter smoothing of the chroma planes, against luminance as the
    /// guide. Removes colour mottle; the eye resolves little chroma detail, so
    /// this is the cheap half with almost nothing to lose.
    #[serde(default = "default_chroma_denoise")]
    pub chroma: bool,
    /// How far the chroma planes move toward the filtered result, `0..=1`.
    #[serde(default = "default_denoise_strength")]
    pub chroma_strength: f32,
    /// À trous wavelet denoising of the luminance plane.
    ///
    /// The half that can destroy signal: every denoiser has a setting at which
    /// nebulae turn to plastic, and only an observer at the eyepiece can find
    /// where that is. Hence a genuine off switch and a visible strength.
    #[serde(default = "default_luma_denoise")]
    pub luma: bool,
    /// Scales every wavelet threshold, `0..=2`. `1.0` is the tuned default.
    ///
    /// Note what this can and cannot reach: it multiplies all four per-level
    /// thresholds, and the level-1 threshold is zero unless `star_protection`
    /// lowers it — so raising this alone leans harder on the *coarse* scales,
    /// which is where faint nebulosity lives. It is the "the target still looks
    /// too noisy" control, not the "the sky still looks grainy" one.
    #[serde(default = "default_denoise_strength")]
    pub luma_strength: f32,
    /// How much of the finest scale is left alone, `0..=1`.
    ///
    /// The finest wavelet scale carries both the sky grain and the star cores,
    /// and a B3 spline transform puts ~94 % of the noise variance there — so
    /// this is the only setting that moves visible grain much. At `1.0` (the
    /// default, and what shipped before it was a control) the scale is untouched
    /// and stars are exactly as they were. Lowering it to zero takes measured
    /// sky sigma on the IMX533 fixture from 4.71 output levels to 1.47, and on
    /// the IMX464 fixture from 10.00 to 1.48, while integrated target flux moves
    /// under 1.5 % — at the cost of softening the tightest stars.
    #[serde(default = "default_star_protection")]
    pub star_protection: f32,
}

fn default_chroma_denoise() -> bool {
    true
}

fn default_luma_denoise() -> bool {
    true
}

fn default_denoise_strength() -> f32 {
    1.0
}

fn default_star_protection() -> f32 {
    1.0
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        Self {
            chroma: default_chroma_denoise(),
            chroma_strength: default_denoise_strength(),
            luma: default_luma_denoise(),
            luma_strength: default_denoise_strength(),
            star_protection: default_star_protection(),
        }
    }
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
    /// Where black sits, as a signed fraction of full scale, in `[-0.09, 0.5]`.
    ///
    /// **Positive** lifts the output floor by this fraction of full scale. An
    /// OLED switches a zero pixel fully off, and the autostretch black point
    /// clamps a few per cent of sky pixels to zero, which at eyepiece
    /// magnification reads as black speckle rather than as sky.
    ///
    /// **Negative** pushes the sky toward black instead. The number still reads
    /// as a fraction of full scale, but what it *does* is anchored to the sky:
    /// `-0.052` puts the floor at the sky level under a nominal sky and still
    /// puts it at the sky level under a brighter one, so one setting behaves the
    /// same on every target. The sky otherwise lands at 14-17 output levels,
    /// which is a clearly visible grey at the eyepiece, and lowering it through
    /// the stretch instead dims the target along with it — see
    /// [`intensity`](Self::intensity).
    #[serde(default = "default_black_floor")]
    pub black_floor: f32,

    /// Let the darkening half of `black_floor` clip to true black instead of
    /// rolling off into it.
    ///
    /// Sky noise is as wide as the sky level, so a hard floor puts around 40 %
    /// of all samples on exactly zero. That buys the deepest possible sky and a
    /// little more separation between target and background, and costs the black
    /// speckle the positive half of `black_floor` exists to remove. Off by
    /// default; no effect while `black_floor` is positive.
    #[serde(default)]
    pub darker_sky: bool,
    /// Ordered dithering at the 8-bit conversion, to keep smooth gradients from
    /// banding once denoising removes the noise that currently masks the steps.
    #[serde(default = "default_dither")]
    pub dither: bool,
}

fn default_intensity() -> f32 {
    0.3
}

fn default_circular_view() -> bool {
    true
}

fn default_black_floor() -> f32 {
    0.04
}

fn default_dither() -> bool {
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
            black_floor: default_black_floor(),
            darker_sky: false,
            dither: default_dither(),
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
            preview_resolution: PreviewResolution::default(),
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
            sensor_correction: SensorCorrectionSettings::default(),
            denoise: DenoiseSettings::default(),
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
    /// Which of the three capture modes this session is running in.
    pub fn capture_mode(&self) -> CaptureMode {
        if !self.stacking {
            return CaptureMode::LiveView;
        }
        if self.wanderer_mode {
            return CaptureMode::Wanderer;
        }
        CaptureMode::Stacking
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

    /// Whether the disk writer has anything at all to do.
    pub fn disk_writing_enabled(&self) -> bool {
        self.saves_raw_frames() || self.saves_stacked_image()
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
}
