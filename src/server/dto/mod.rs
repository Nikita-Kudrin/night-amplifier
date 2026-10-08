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
    CameraCaptureProfile, CameraRole, CaptureSettings, CaptureState, EyepieceSettings,
    RawFrameSaving, Resolution, SessionStats,
};
use crate::background::BackgroundExtractionAlgorithm;
use crate::camera::{CameraInfo, DualSamplingMode, SensorMode};
use crate::cfa::SensorCorrectionSettings;
use crate::planetary::AlignmentRoi;
use crate::push_to::TelescopeSettings;
use crate::render::denoise::DenoiseSettings;
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
    pub debug_logging: bool,
}

#[derive(Debug, Serialize)]
pub struct DeepSkyCapabilities {
    pub advanced_rejection: bool,
    pub rbf_background: bool,
    pub saturation_boost: bool,
    /// The spatial denoisers and every control over them — Denoise, Colour Mottle,
    /// Background Grain, Structure strength. Without them the Noise Reduction section is
    /// locked and the render is the plain one.
    pub denoise: bool,
    /// The AI denoiser behind the "AI denoising" switch; locked without it.
    pub ai_denoise: bool,
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

impl CaptureStatusResponse {
    /// `exposure_us` and `gain` are the imaging camera's settings; nothing records a
    /// `last_error`, so it is always `None`.
    pub fn new(state: CaptureState, stats: &SessionStats, settings: &CaptureSettings) -> Self {
        let counts = stats.counts();
        Self {
            state: format!("{:?}", state),
            frame_count: counts.frames,
            stacked_count: counts.stacked,
            rejected_count: counts.rejected,
            last_error: None,
            started_at: stats.started_at(),
            exposure_us: settings.exposure_us,
            gain: settings.gain,
        }
    }
}

/// Camera sensor mode DTO (dual sampling mode slot)
#[derive(Debug, Clone, Serialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct CameraInfoResponse {
    pub id: String,
    pub name: String,
    pub max_width: u32,
    pub max_height: u32,
    pub pixel_size_x_um: f64,
    pub pixel_size_y_um: f64,
    pub sensor_type: String,
    pub has_cooler: bool,
    pub has_dew_heater: bool,
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
            has_dew_heater: info.has_dew_heater,
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

/// What `GET`/`POST /api/settings` answer: every setting, in `settings.json`'s shape.
///
/// Except the Focus/Finder snapshot: it is the server's record of what to restore, and a
/// client that could read it would be tempted to write it.
#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    #[serde(flatten)]
    pub settings: CaptureSettings,
}

impl From<&CaptureSettings> for SettingsResponse {
    fn from(settings: &CaptureSettings) -> Self {
        Self {
            settings: CaptureSettings {
                focus_mode_snapshot: None,
                ..settings.clone()
            },
        }
    }
}

/// Camera list entry
#[derive(Debug, Clone, Serialize)]
pub struct CameraListEntry {
    pub id: String,
    pub name: String,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// Which position this camera occupies, or `None` if it is not connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CameraRole>,
    /// Lifecycle phase of a connected camera. The client's phase map is otherwise built
    /// from `camera_phase_changed` events alone, which a page opened later never saw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<crate::server::events::CameraPhaseDto>,
    /// Seconds until a warm-up in progress is cut short, at the latest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_remaining_s: Option<u64>,
    pub info: CameraInfoResponse,
}

/// Body of `POST /api/cameras/{id}/connect`.
///
/// Absent (or an empty body) means the imaging camera, so a client that has never heard
/// of roles keeps working.
#[derive(Debug, Default, Deserialize)]
pub struct ConnectCameraRequest {
    #[serde(default)]
    pub role: Option<CameraRole>,
}

/// Body of `POST /api/cameras/{id}/disconnect`. Absent means an ordinary Disconnect.
#[derive(Debug, Default, Deserialize)]
pub struct DisconnectCameraRequest {
    /// Close at once instead of warming a cooled camera up first.
    #[serde(default)]
    pub skip_warmup: bool,
}

/// Answer to a Disconnect.
#[derive(Debug, Serialize)]
pub struct DisconnectResponse {
    pub message: String,
    pub camera_id: String,
    /// The camera is warming up and disconnects on its own once warm.
    pub warming_up: bool,
    /// Seconds until a warm-up is cut short, at the latest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_remaining_s: Option<u64>,
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

/// Body of `POST /api/capture/start`.
///
/// `role` says which camera the button was pressed for, mirroring
/// [`ConnectCameraRequest`]: the panel edits whichever camera is selected in the list,
/// so Start has to address the same one. Absent means the imaging camera, so a client
/// that has never heard of roles keeps working.
#[derive(Debug, Deserialize, Default)]
pub struct StartCaptureRequest {
    #[serde(default)]
    pub camera_id: Option<String>,
    #[serde(default)]
    pub role: Option<CameraRole>,
}

/// Body of `POST /api/capture/stop`. See [`StartCaptureRequest`] for `role`.
#[derive(Debug, Deserialize, Default)]
pub struct StopCaptureRequest {
    #[serde(default)]
    pub role: Option<CameraRole>,
}

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
        assert_eq!(response.settings.exposure_us, 2_000_000);
        assert_eq!(response.settings.gain, 100);
    }

    /// The keys the frontend reads, pinned: the answer is `CaptureSettings` serialised,
    /// so a serde attribute added for `settings.json` would otherwise change it unseen.
    /// Optional values and empty maps are left out rather than sent as `null`.
    #[test]
    fn the_settings_answer_keeps_its_wire_shape() {
        const ALWAYS: &[&str] = &[
            "auto_reconnect", "auto_resume_capture", "auto_stretch", "auto_stretch_intensity",
            "background_extraction_algorithm", "background_subtraction", "bin",
            "cooler_enabled", "cooler_fast_mode", "denoise", "dew_heater_enabled",
            "dew_heater_power", "eula_accepted", "exposure_us", "eyepiece", "focus_mode",
            "force_focus_image_now", "gain", "guide_camera", "indi_server_host",
            "indi_server_port", "offset", "planetary_auto_tracking",
            "planetary_multi_point_alignment", "preview_resolution", "raw_frame_saving",
            "rejection_method", "rejection_sigma", "saturation_boost",
            "saturation_boost_strength", "save_stacked_image", "sensor_correction",
            "show_focus_image", "simulated_preload_images", "stacking", "stacking_type",
            "streaming_resolution", "stretch_aggressiveness", "telescope",
            "use_simulated_camera", "wanderer_mode", "weighting_preset",
        ];
        const WHEN_SET: &[&str] = &[
            "camera_profiles", "camera_telescope_profiles", "comet_roi", "last_camera_name",
            "planetary_roi", "sensor_mode_override", "target_temp_c",
        ];
        let keys = |settings: &CaptureSettings| -> Vec<String> {
            let value = serde_json::to_value(SettingsResponse::from(settings)).unwrap();
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };

        assert_eq!(keys(&CaptureSettings::default()), ALWAYS);

        let roi = AlignmentRoi { x: 1, y: 2, width: 30, height: 40 };
        let mut everything = CaptureSettings {
            comet_roi: Some(roi),
            planetary_roi: Some(roi),
            last_camera_name: Some("Ares-C PRO".into()),
            target_temp_c: Some(-10.0),
            sensor_mode_override: Some(DualSamplingMode::Normal),
            ..Default::default()
        };
        everything.camera_telescope_profiles.insert("Ares-C PRO".into(), Default::default());
        everything.camera_profiles.insert("PlayerOne/Ares-C PRO".into(), Default::default());
        crate::server::state::focus_mode::set(&mut everything, true, &crate::plugins::Plugins::none());

        let mut expected: Vec<&str> = ALWAYS.iter().chain(WHEN_SET).copied().collect();
        expected.sort();
        assert_eq!(keys(&everything), expected, "and never the Focus/Finder snapshot");
        assert!(everything.focus_mode_snapshot.is_some(), "the snapshot was there to leak");
    }

    /// The counters as the stats hold them, and the exposure and gain the imaging camera
    /// is set to — the answer once carried a default 1 s and gain 0 nothing ever updated.
    #[test]
    fn the_capture_status_reports_the_counters_and_the_cameras_settings() {
        let stats = SessionStats::default();
        for stacked in [true, true, false] {
            stats.frame_captured(stacked, true);
        }
        let settings = CaptureSettings {
            exposure_us: 30_000_000,
            gain: 120,
            ..CaptureSettings::default()
        };

        let response = CaptureStatusResponse::new(CaptureState::Capturing, &stats, &settings);
        assert_eq!(response.state, "Capturing");
        assert_eq!((response.frame_count, response.stacked_count, response.rejected_count), (3, 2, 1));
        assert_eq!((response.exposure_us, response.gain), (30_000_000, 120));
        assert_eq!(response.last_error, None);
    }
}
