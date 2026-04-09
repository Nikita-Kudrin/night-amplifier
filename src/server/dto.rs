//! Data Transfer Objects (DTOs) for REST API
//!
//! This module contains request and response types used by the REST API handlers.

use serde::{Deserialize, Serialize};

use super::state::{CaptureSession, CaptureSettings, EyepieceSettings};
use crate::background::BackgroundExtractionAlgorithm;
use crate::camera::CameraInfo;
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
    pub bit_depth: u8,
    pub min_exposure_us: u64,
    pub max_exposure_us: u64,
    pub min_gain: i32,
    pub max_gain: i32,
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
            bit_depth: info.bit_depth,
            min_exposure_us: info.min_exposure_us,
            max_exposure_us: info.max_exposure_us,
            min_gain: info.min_gain,
            max_gain: info.max_gain,
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

// Note: StackingTypeInfo is used directly from state.rs for API responses
// It includes capability information (uses_star_registration, supports_live_stacking, etc.)
// See StackingType::info() for the full structure

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
}

/// Configure simulated camera request
#[derive(Debug, Deserialize)]
pub struct ConfigureSimulatorRequest {
    /// Path to directory containing image files
    pub directory: String,
}

// ============================================================================
// Push-To Navigation DTOs
// ============================================================================

/// Push-To position response (from plate solve)
#[derive(Debug, Serialize)]
pub struct PushToPositionResponse {
    /// Right Ascension in degrees
    pub ra_degrees: f64,
    /// Declination in degrees
    pub dec_degrees: f64,
    /// RA as formatted string (HH:MM:SS)
    pub ra_string: String,
    /// Dec as formatted string (±DD:MM:SS)
    pub dec_string: String,
    /// Field rotation in degrees
    pub rotation_deg: f64,
    /// Estimated FOV in degrees
    pub fov_deg: f64,
    /// Number of stars matched
    pub stars_matched: usize,
    /// Solve confidence (0-1)
    pub confidence: f64,
    /// Time taken to solve (ms)
    pub solve_time_ms: u64,
}

/// Push-To direction response
#[derive(Debug, Serialize)]
pub struct PushToDirectionResponse {
    /// Angle to push in degrees (0 = north, 90 = east)
    pub angle_deg: f64,
    /// Angular distance to target in degrees
    pub distance_deg: f64,
    /// Whether within fine-adjustment range (<1 degree)
    pub is_close: bool,
    /// Direction hint (N, NE, E, SE, S, SW, W, NW, OK)
    pub direction_hint: String,
    /// Full direction description
    pub direction_full: String,
    /// Current position (if solved)
    pub current_position: Option<CoordinateResponse>,
    /// Target position
    pub target: Option<CoordinateResponse>,
}

/// Coordinate response (simplified)
#[derive(Debug, Serialize)]
pub struct CoordinateResponse {
    pub ra_degrees: f64,
    pub dec_degrees: f64,
    pub ra_string: String,
    pub dec_string: String,
}

/// Catalog entry response
#[derive(Debug, Serialize)]
pub struct CatalogEntryResponse {
    pub designation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub common_name: Option<String>,
    pub catalog_type: String,
    pub ra_degrees: f64,
    pub dec_degrees: f64,
    pub ra_string: String,
    pub dec_string: String,
    pub object_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magnitude: Option<f32>,
    pub constellation: String,
}

/// Push-To status response
#[derive(Debug, Serialize)]
pub struct PushToStatusResponse {
    /// Whether the solver database is loaded
    pub solver_ready: bool,
    /// Whether a plate solve is currently in progress
    pub is_solving: bool,
    /// Current target (if set)
    pub current_target: Option<CatalogEntryResponse>,
    /// Last solved position (if available)
    pub last_position: Option<CoordinateResponse>,
    /// Push direction to target (if both position and target are set)
    pub direction: Option<PushToDirectionResponse>,
}

/// Set target request
#[derive(Debug, Deserialize)]
pub struct SetTargetRequest {
    /// Target name (e.g., "M31", "NGC 7000", "Andromeda Galaxy")
    #[serde(default)]
    pub name: Option<String>,
    /// Or set by coordinates
    #[serde(default)]
    pub ra_degrees: Option<f64>,
    #[serde(default)]
    pub dec_degrees: Option<f64>,
}

/// Search catalog request
#[derive(Debug, Deserialize)]
pub struct SearchCatalogRequest {
    /// Search query
    pub query: String,
    /// Maximum results to return
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}

fn default_search_limit() -> usize {
    20
}

/// Push-To configuration request
#[derive(Debug, Deserialize)]
pub struct PushToConfigRequest {
    /// Field of view hint in degrees
    #[serde(default)]
    pub fov_degrees: Option<f32>,
    /// Path to solver database
    #[serde(default)]
    pub database_path: Option<String>,
}

// ============================================================================
// ASTAP Installation DTOs
// ============================================================================

/// ASTAP installation status response
#[derive(Debug, Serialize)]
pub struct AstapStatusResponse {
    /// Whether ASTAP CLI binary is installed and executable
    pub binary_installed: bool,
    /// Path to the ASTAP binary (if installed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    /// Whether a star database is installed
    pub database_installed: bool,
    /// Path to the database directory (if installed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_path: Option<String>,
    /// Which database is installed (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_type: Option<String>,
    /// Whether the system is ready for plate solving
    pub ready: bool,
}

/// Available database types for installation
#[derive(Debug, Serialize)]
pub struct DatabaseTypeResponse {
    /// Database identifier (D80, G05, W08)
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// FOV range string (e.g., "0.2°-15°")
    pub fov_range: String,
    /// Approximate download size (e.g., "~3GB")
    pub size: String,
}

/// ASTAP installation request
#[derive(Debug, Deserialize)]
pub struct AstapInstallRequest {
    /// Which database to install (D80, G05, W08)
    /// Defaults to D80 if not specified
    #[serde(default = "default_database_type")]
    pub database_type: String,
}

fn default_database_type() -> String {
    "D80".to_string()
}

// ============================================================================
// OpenNGC Catalog Installation DTOs
// ============================================================================

/// Catalog installation status response
#[derive(Debug, Serialize)]
pub struct CatalogStatusResponse {
    /// Whether the catalog is installed
    pub installed: bool,
    /// Path to the catalog directory (if installed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_path: Option<String>,
    /// Whether NGC.csv exists
    pub ngc_file_exists: bool,
    /// Whether addendum.csv exists
    pub addendum_file_exists: bool,
    /// Number of objects loaded (if catalog was parsed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_count: Option<usize>,
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
