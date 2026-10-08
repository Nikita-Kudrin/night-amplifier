//! Push-To's values: what the plugin traits take and return. The `*Response` names are
//! the plugin's answers, kept because Pro's own `CatalogEntry`, `AstapStatus` and the like
//! are its richer internal types; the server serves them as they are.

use serde::Serialize;

/// Telescope and camera parameters for FOV calculation
///
/// `PartialEq` so callers can act on a telescope block that actually *changed*
/// rather than one that was merely present in the request. Every field feeds the
/// FOV, so any difference matters and a whole-struct comparison is the right test.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
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

/// Push-To position response (from plate solve)
#[derive(Debug, Clone, Serialize)]
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
    /// Stars ASTAP found in the image, as it reported them in its own log.
    ///
    /// Not a *matched* count — neither the WCS nor the INI carries one. `None` means
    /// ASTAP never said, which is not the same claim as an empty field.
    pub stars_detected: Option<usize>,
    /// Solve confidence (0-1)
    pub confidence: f64,
    /// Time taken to solve (ms)
    pub solve_time_ms: u64,
}

/// Push-To direction response
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct PushToDirectionResponse {
    /// Angle to push in degrees, in the image frame:
    /// 0 = screen up, 90 = screen right, rotation is clockwise.
    /// The plate-solved camera rotation and parity are already applied, so
    /// this can be used directly as an SVG/CSS rotation for a chevron that
    /// points "up" at 0°. For a celestial-frame label use `direction_hint`.
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
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct CoordinateResponse {
    pub ra_degrees: f64,
    pub dec_degrees: f64,
    pub ra_string: String,
    pub dec_string: String,
}

/// Catalog entry response
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct CatalogEntryResponse {
    pub designation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub catalog_type: String,
    pub ra_degrees: f64,
    pub dec_degrees: f64,
    pub ra_string: String,
    pub dec_string: String,
    pub object_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub magnitude: Option<f32>,
    pub constellation: String,
    /// Messier number ("M4") when the object is in the Messier catalog
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messier: Option<String>,
    /// The alias or identifier a search matched ("C 69"), when neither designation nor name did
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_name: Option<String>,
}

/// Push-To status response
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
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

/// ASTAP installation status response
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct AstapStatusResponse {
    /// Whether ASTAP CLI binary is installed and executable
    pub binary_installed: bool,
    /// Path to the ASTAP binary (if installed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    /// Whether at least one star database is installed
    pub database_installed: bool,
    /// Path to the primary database directory (if installed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_path: Option<String>,
    /// Primary installed database type (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_type: Option<String>,
    /// All installed databases with their paths
    pub installed_databases: Vec<InstalledDatabaseInfo>,
    /// Whether the system is ready for plate solving
    pub ready: bool,
}

/// Information about a single installed database
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct InstalledDatabaseInfo {
    /// Database identifier (D80, G05, W08)
    pub id: String,
    /// Path to this database's directory
    pub database_path: String,
    /// Minimum FOV in degrees this database supports
    pub min_fov_deg: f32,
    /// Maximum FOV in degrees this database supports
    pub max_fov_deg: f32,
}

/// Available database types for installation
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct DatabaseTypeResponse {
    /// Database identifier (D80, G05, W08)
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Minimum FOV in degrees this database supports
    pub min_fov_deg: f32,
    /// Maximum FOV in degrees this database supports
    pub max_fov_deg: f32,
    /// Approximate download size (e.g., "~3GB")
    pub size: String,
    /// Whether this database is already installed
    pub installed: bool,
}

/// Catalog installation status response
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
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
    /// Whether hyg_stars.csv exists
    pub hyg_file_exists: bool,
    /// Number of objects loaded (if catalog was parsed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_count: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinate() -> CoordinateResponse {
        CoordinateResponse {
            ra_degrees: 10.5,
            dec_degrees: -20.25,
            ra_string: "00h42m".into(),
            dec_string: "-20°15'".into(),
        }
    }

    fn entry() -> CatalogEntryResponse {
        CatalogEntryResponse {
            designation: "NGC 224".into(),
            name: Some("Andromeda".into()),
            catalog_type: "NGC".into(),
            ra_degrees: 10.68,
            dec_degrees: 41.27,
            ra_string: "a".into(),
            dec_string: "b".into(),
            object_type: "Galaxy".into(),
            magnitude: Some(3.4),
            constellation: "And".into(),
            messier: Some("M31".into()),
            matched_name: None,
        }
    }

    fn json(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap()
    }

    /// The REST answers the plugin's values become, captured while they were still
    /// server DTOs: moving them into the domain must not move a byte of the wire.
    #[test]
    fn the_answers_keep_their_wire_shape() {
        let direction = PushToDirectionResponse {
            angle_deg: 12.0,
            distance_deg: 3.5,
            is_close: false,
            direction_hint: "NE".into(),
            direction_full: "North-East".into(),
            current_position: Some(coordinate()),
            target: Some(coordinate()),
        };
        let status = PushToStatusResponse {
            solver_ready: true,
            is_solving: false,
            current_target: Some(entry()),
            last_position: Some(coordinate()),
            direction: Some(direction),
        };
        assert_eq!(
            json(&status),
            r#"{"solver_ready":true,"is_solving":false,"current_target":{"designation":"NGC 224","name":"Andromeda","catalog_type":"NGC","ra_degrees":10.68,"dec_degrees":41.27,"ra_string":"a","dec_string":"b","object_type":"Galaxy","magnitude":3.4,"constellation":"And","messier":"M31"},"last_position":{"ra_degrees":10.5,"dec_degrees":-20.25,"ra_string":"00h42m","dec_string":"-20°15'"},"direction":{"angle_deg":12.0,"distance_deg":3.5,"is_close":false,"direction_hint":"NE","direction_full":"North-East","current_position":{"ra_degrees":10.5,"dec_degrees":-20.25,"ra_string":"00h42m","dec_string":"-20°15'"},"target":{"ra_degrees":10.5,"dec_degrees":-20.25,"ra_string":"00h42m","dec_string":"-20°15'"}}}"#
        );

        let position = PushToPositionResponse {
            ra_degrees: 1.0,
            dec_degrees: 2.0,
            ra_string: "r".into(),
            dec_string: "d".into(),
            rotation_deg: 3.0,
            fov_deg: 0.5,
            stars_detected: None,
            confidence: 0.9,
            solve_time_ms: 1200,
        };
        assert_eq!(
            json(&position),
            r#"{"ra_degrees":1.0,"dec_degrees":2.0,"ra_string":"r","dec_string":"d","rotation_deg":3.0,"fov_deg":0.5,"stars_detected":null,"confidence":0.9,"solve_time_ms":1200}"#
        );

        let astap = AstapStatusResponse {
            binary_installed: true,
            binary_path: Some("/a".into()),
            database_installed: true,
            database_path: None,
            database_type: Some("D80".into()),
            installed_databases: vec![InstalledDatabaseInfo {
                id: "D80".into(),
                database_path: "/d".into(),
                min_fov_deg: 0.15,
                max_fov_deg: 6.0,
            }],
            ready: true,
        };
        assert_eq!(
            json(&astap),
            r#"{"binary_installed":true,"binary_path":"/a","database_installed":true,"database_type":"D80","installed_databases":[{"id":"D80","database_path":"/d","min_fov_deg":0.15,"max_fov_deg":6.0}],"ready":true}"#
        );

        let database = DatabaseTypeResponse {
            id: "G05".into(),
            description: "Wide".into(),
            min_fov_deg: 0.5,
            max_fov_deg: 20.0,
            size: "~1GB".into(),
            installed: false,
        };
        assert_eq!(
            json(&database),
            r#"{"id":"G05","description":"Wide","min_fov_deg":0.5,"max_fov_deg":20.0,"size":"~1GB","installed":false}"#
        );

        let catalog = CatalogStatusResponse {
            installed: true,
            catalog_path: None,
            ngc_file_exists: true,
            addendum_file_exists: false,
            hyg_file_exists: true,
            object_count: Some(5),
        };
        assert_eq!(
            json(&catalog),
            r#"{"installed":true,"ngc_file_exists":true,"addendum_file_exists":false,"hyg_file_exists":true,"object_count":5}"#
        );
    }
}
