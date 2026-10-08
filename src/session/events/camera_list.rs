//! The camera listing a client sees: in `GET /api/cameras` and in `camera_discovered`
//! events, which is why it is defined beside the events rather than in `server::dto`.

use serde::Serialize;

use crate::camera::{CameraInfo, SensorMode};
use crate::session::state::CameraRole;

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
    pub phase: Option<super::CameraPhaseDto>,
    /// Seconds until a warm-up in progress is cut short, at the latest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_remaining_s: Option<u64>,
    pub info: CameraInfoResponse,
}
