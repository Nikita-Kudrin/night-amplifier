//! Data Transfer Objects (DTOs) for REST API
//!
//! This module contains request and response types used by the REST API handlers.

mod install;
mod push_to;

pub use install::*;
pub use push_to::*;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::state::{
    CameraCaptureProfile, CaptureSession, CaptureSettings, EyepieceSettings, TelescopeSettings,
};
use crate::background::BackgroundExtractionAlgorithm;
use crate::camera::{CameraInfo, DualSamplingMode, SensorMode};
use crate::planetary::AlignmentRoi;
use crate::render::StretchAggressiveness;
use crate::stacking::{RejectionMethod, StackingType, WeightingPreset};

// ============================================================================
// Response types
// ============================================================================

/// Standard API response wrapper
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Capabilities response for feature detection
#[derive(Debug, Serialize)]
pub struct CapabilitiesResponse {
    pub has_pro: bool,
    pub deep_sky: DeepSkyCapabilities,
    pub planetary: PlanetaryCapabilities,
    pub push_to: PushToCapabilities,
    pub comet: CometCapabilities,
}

#[derive(Debug, Serialize)]
pub struct DeepSkyCapabilities {
    pub advanced_rejection: bool,
    pub rbf_background: bool,
    pub saturation_boost: bool,
}

#[derive(Debug, Serialize)]
pub struct PlanetaryCapabilities {
    pub advanced_stacking: bool,
}

#[derive(Debug, Serialize)]
pub struct PushToCapabilities {
    pub astap_solver: bool,
}

#[derive(Debug, Serialize)]
pub struct CometCapabilities {
    pub pro_stacking: bool,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> axum::Json<Self> {
        axum::Json(Self {
            success: true,
            data: Some(data),
            error: None,
        })
    }
}

impl ApiResponse<()> {
    pub fn err<T: Serialize>(message: impl Into<String>) -> axum::Json<ApiResponse<T>> {
        axum::Json(ApiResponse {
            success: false,
            data: None,
            error: Some(message.into()),
        })
    }
}

/// Capture status response
#[derive(Debug, Serialize)]
pub struct CaptureStatusResponse {
    pub state: String,
    pub frame_count: u64,
    pub stacked_count: u64,
    pub rejected_count: u64,
    pub last_error: Option<String>,
    pub started_at: Option<u64>,
    pub exposure_us: u64,
    pub gain: i32,
}

impl From<&CaptureSession> for CaptureStatusResponse {
    fn from(session: &CaptureSession) -> Self {
        Self {
            state: format!("{:?}", session.state),
            frame_count: session.frame_count,
            stacked_count: session.stacked_count,
            rejected_count: session.rejected_count,
            last_error: session.last_error.clone(),
            started_at: session.started_at,
            exposure_us: session.exposure_us,
            gain: session.gain,
        }
    }
}

/// Camera sensor mode DTO (dual sampling mode slot)
#[derive(Debug, Serialize)]
pub struct SensorModeDto {
    pub index: u32,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl From<&SensorMode> for SensorModeDto {
    fn from(mode: &SensorMode) -> Self {
        Self {
            index: mode.index,
            name: mode.name.clone(),
            description: mode.description.clone(),
        }
    }
}

/// Camera info response
#[derive(Debug, Serialize)]
pub struct CameraInfoResponse {
    pub id: String,
    pub name: String,
    pub max_width: u32,
    pub max_height: u32,
    pub pixel_size_x_um: f64,
    pub pixel_size_y_um: f64,
    pub sensor_type: String,
    pub has_cooler: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_temp_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_temp_c: Option<f64>,
    pub bit_depth: u8,
    pub min_exposure_us: u64,
    pub max_exposure_us: u64,
    pub min_gain: i32,
    pub max_gain: i32,
    /// Sensor (dual sampling) modes reported by the camera. Empty when unsupported.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sensor_modes: Vec<SensorModeDto>,
}

impl CameraInfoResponse {
    pub fn from_info(info: &CameraInfo, id: &str) -> Self {
        Self {
            id: id.to_string(),
            name: info.name.clone(),
            max_width: info.max_width,
            max_height: info.max_height,
            pixel_size_x_um: info.pixel_size_x_um,
            pixel_size_y_um: info.pixel_size_y_um,
            sensor_type: format!("{:?}", info.sensor_type),
            has_cooler: info.has_cooler,
            min_temp_c: info.min_temp_c,
            max_temp_c: info.max_temp_c,
            bit_depth: info.bit_depth,
            min_exposure_us: info.min_exposure_us,
            max_exposure_us: info.max_exposure_us,
            min_gain: info.min_gain,
            max_gain: info.max_gain,
            sensor_modes: info.sensor_modes.iter().map(SensorModeDto::from).collect(),
        }
    }
}

/// Settings response
#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsResponse {
    pub exposure_us: u64,
    pub gain: i32,
    pub offset: i32,
    pub bin: u8,
    pub auto_stretch: bool,
    pub stacking: bool,
    pub rejection_sigma: f32,
    pub rejection_method: RejectionMethod,
    pub background_subtraction: bool,
    /// Algorithm for background extraction
    pub background_extraction_algorithm: BackgroundExtractionAlgorithm,
    pub save_raw_frames: bool,
    pub save_stacked_image: bool,
    pub stacking_type: StackingType,
    /// Quality-based frame weighting preset for stacking
    pub weighting_preset: WeightingPreset,
    /// Auto stretch aggressiveness (Low, Medium, High)
    pub stretch_aggressiveness: StretchAggressiveness,
    /// Enable shadow saturation boost for more vibrant deep-sky colors
    pub saturation_boost: bool,
    /// Shadow saturation boost strength (0.0-1.0)
    pub saturation_boost_strength: f32,
    /// Use simulated camera
    pub use_simulated_camera: bool,
    /// Number of images to preload for simulated camera
    pub simulated_preload_images: usize,
    /// Region of interest for comet nucleus tracking (used in Comet stacking mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comet_roi: Option<AlignmentRoi>,
    /// Region of interest for planetary alignment (used in Planetary stacking mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planetary_roi: Option<AlignmentRoi>,
    /// Enable "Wanderer" mode for automatic stack reset on movement
    pub wanderer_mode: bool,
    pub eyepiece: EyepieceSettings,
    pub telescope: TelescopeSettings,
    /// Per-camera telescope profiles keyed by camera name
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub camera_telescope_profiles: HashMap<String, TelescopeSettings>,
    /// Per-camera capture profiles keyed by `"{provider}/{model_name}"`
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub camera_profiles: HashMap<String, CameraCaptureProfile>,
    /// Name of the last active camera
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_camera_name: Option<String>,
    /// Whether the cooler should be active during capture
    pub cooler_enabled: bool,
    /// Target sensor temperature in Celsius
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_temp_c: Option<f64>,
    /// Bypass the 5 °C/min cool/warm ramp (advanced users only)
    #[serde(default)]
    pub cooler_fast_mode: bool,
    /// Manual override for camera sensor mode (Player One dual sampling).
    /// When null, the mode is auto-selected based on `stacking_type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_mode_override: Option<DualSamplingMode>,
    /// Whether anti-dew heater is enabled
    pub dew_heater_enabled: bool,
    /// Anti-dew heater power level (0-100)
    pub dew_heater_power: i32,
}

impl From<&CaptureSettings> for SettingsResponse {
    fn from(settings: &CaptureSettings) -> Self {
        Self {
            exposure_us: settings.exposure_us,
            gain: settings.gain,
            offset: settings.offset,
            bin: settings.bin,
            auto_stretch: settings.auto_stretch,
            stacking: settings.stacking,
            rejection_sigma: settings.rejection_sigma,
            rejection_method: settings.rejection_method,
            background_subtraction: settings.background_subtraction,
            background_extraction_algorithm: settings.background_extraction_algorithm,
            save_raw_frames: settings.save_raw_frames,
            save_stacked_image: settings.save_stacked_image,
            stacking_type: settings.stacking_type,
            weighting_preset: settings.weighting_preset,
            stretch_aggressiveness: settings.stretch_aggressiveness,
            saturation_boost: settings.saturation_boost,
            saturation_boost_strength: settings.saturation_boost_strength,
            use_simulated_camera: settings.use_simulated_camera,
            simulated_preload_images: settings.simulated_preload_images,
            comet_roi: settings.comet_roi.clone(),
            planetary_roi: settings.planetary_roi.clone(),
            wanderer_mode: settings.wanderer_mode,
            eyepiece: settings.eyepiece.clone(),
            telescope: settings.telescope.clone(),
            camera_telescope_profiles: settings.camera_telescope_profiles.clone(),
            camera_profiles: settings.camera_profiles.clone(),
            last_camera_name: settings.last_camera_name.clone(),
            cooler_enabled: settings.cooler_enabled,
            target_temp_c: settings.target_temp_c,
            cooler_fast_mode: settings.cooler_fast_mode,
            sensor_mode_override: settings.sensor_mode_override,
            dew_heater_enabled: settings.dew_heater_enabled,
            dew_heater_power: settings.dew_heater_power,
        }
    }
}

/// Camera list entry
#[derive(Debug, Serialize)]
pub struct CameraListEntry {
    pub id: String,
    pub name: String,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    pub info: CameraInfoResponse,
}

/// Simple message response
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_id: Option<String>,
}

/// Simulated camera configuration response
#[derive(Debug, Serialize)]
pub struct SimulatorConfigResponse {
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub was_added: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// ============================================================================
// Request types
// ============================================================================

/// Start capture request
#[derive(Debug, Deserialize, Default)]
pub struct StartCaptureRequest {
    #[serde(default)]
    pub camera_id: Option<String>,
}

/// Update settings request
#[derive(Debug, Deserialize, Default)]
pub struct UpdateSettingsRequest {
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
    #[serde(default)]
    pub save_raw_frames: Option<bool>,
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
    /// Region of interest for comet nucleus tracking (used in Comet stacking mode)
    #[serde(default)]
    pub comet_roi: Option<AlignmentRoi>,
    /// Region of interest for planetary alignment (used in Planetary stacking mode)
    #[serde(default)]
    pub planetary_roi: Option<AlignmentRoi>,
    /// Enable "Wanderer" mode
    #[serde(default)]
    pub wanderer_mode: Option<bool>,

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
}

/// Configure simulated camera request
#[derive(Debug, Deserialize)]
pub struct ConfigureSimulatorRequest {
    /// Path to directory containing image files
    pub directory: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_response_from_capture_settings() {
        let settings = CaptureSettings {
            exposure_us: 2_000_000,
            gain: 100,
            ..Default::default()
        };

        let response = SettingsResponse::from(&settings);
        assert_eq!(response.exposure_us, 2_000_000);
        assert_eq!(response.gain, 100);
    }

    #[test]
    fn test_capture_status_response_from_session() {
        let session = CaptureSession {
            frame_count: 10,
            stacked_count: 8,
            rejected_count: 2,
            ..Default::default()
        };

        let response = CaptureStatusResponse::from(&session);
        assert_eq!(response.frame_count, 10);
        assert_eq!(response.stacked_count, 8);
        assert_eq!(response.rejected_count, 2);
    }
}
