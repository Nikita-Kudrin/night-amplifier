//! Push-To navigation DTOs

use serde::{Deserialize, Serialize};

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
