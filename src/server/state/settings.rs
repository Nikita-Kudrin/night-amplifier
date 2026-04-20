use std::collections::HashMap;

use crate::background::BackgroundExtractionAlgorithm;
use crate::camera::CaptureConfig;
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
    /// Enable shadow saturation boost
    pub saturation_boost: bool,
    /// Shadow saturation boost strength (0.0-1.0)
    pub saturation_boost_strength: f32,
    /// Use simulated camera instead of a real one
    pub use_simulated_camera: bool,
    /// Number of images to preload for simulated camera
    pub simulated_preload_images: usize,
    /// Whether the cooler should be active during capture (cooled cameras only)
    pub cooler_enabled: bool,
    /// Target sensor temperature in Celsius (None means "no target set")
    pub target_temp_c: Option<f64>,
    /// Region of interest for comet nucleus tracking
    pub comet_roi: Option<AlignmentRoi>,
    /// Region of interest for planetary alignment
    pub planetary_roi: Option<AlignmentRoi>,
    /// Enable "Wanderer" mode for automatic stack reset on movement
    pub wanderer_mode: bool,
    /// Deduced field of view from successful plate solves
    pub push_to_fov: Option<f32>,
    /// Eyepiece view settings
    pub eyepiece: EyepieceSettings,
    /// Telescope and camera parameters for FOV calculation
    pub telescope: TelescopeSettings,
    /// Per-camera telescope profiles keyed by camera name
    pub camera_telescope_profiles: HashMap<String, TelescopeSettings>,
    /// Name of the last active camera (for profile inheritance)
    pub last_camera_name: Option<String>,
}

/// Telescope and camera parameters for FOV calculation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

impl Default for TelescopeSettings {
    fn default() -> Self {
        Self {
            focal_length_mm: None,
            pixel_size_x_um: None,
            pixel_size_y_um: None,
            sensor_width_px: None,
            sensor_height_px: None,
            barlow_coeff: None,
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
            saturation_boost: false,
            saturation_boost_strength: 0.5,
            use_simulated_camera: false,
            simulated_preload_images: 5,
            cooler_enabled: false,
            target_temp_c: None,
            comet_roi: None,
            planetary_roi: None,
            wanderer_mode: false,
            push_to_fov: None,
            eyepiece: EyepieceSettings::default(),
            telescope: TelescopeSettings::default(),
            camera_telescope_profiles: HashMap::new(),
            last_camera_name: None,
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
        let mut config = CaptureConfig::new()
            .with_exposure_us(self.exposure_us)
            .with_gain(self.gain)
            .with_offset(self.offset)
            .with_bin(self.bin)
            .with_simulated_preload_images(self.simulated_preload_images)
            .with_cooler(self.cooler_enabled);
        if let Some(temp) = self.target_temp_c {
            config.target_temp_c = Some(temp);
        }
        config
    }
}
