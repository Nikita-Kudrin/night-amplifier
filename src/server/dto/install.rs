//! ASTAP and OpenNGC catalog installation DTOs

use serde::{Deserialize, Serialize};

/// ASTAP installation status response
#[derive(Debug, Serialize)]
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

/// ASTAP installation request
#[derive(Debug, Deserialize)]
pub struct AstapInstallRequest {
    /// Which databases to install (D80, G05, W08)
    #[serde(default)]
    pub database_types: Vec<String>,
    /// Legacy single database field for backward compatibility
    #[serde(default)]
    pub database_type: Option<String>,
}

impl AstapInstallRequest {
    /// Normalize the request into a list of database types
    pub fn into_database_types(self) -> Vec<String> {
        if !self.database_types.is_empty() {
            self.database_types
        } else if let Some(dt) = self.database_type {
            vec![dt]
        } else {
            vec!["D80".to_string()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installed_database_info_fov_fields_present() {
        // Verify the DTO struct exposes min_fov_deg and max_fov_deg fields.
        let info = InstalledDatabaseInfo {
            id: "D80".to_string(),
            database_path: "/astap/d80_database".to_string(),
            min_fov_deg: 0.15,
            max_fov_deg: 6.0,
        };
        assert_eq!(info.id, "D80");
        assert_eq!(info.min_fov_deg, 0.15);
        assert_eq!(info.max_fov_deg, 6.0);
    }

    #[test]
    fn test_database_type_response_fov_fields_present() {
        let resp = DatabaseTypeResponse {
            id: "D80".to_string(),
            description: "General Purpose".to_string(),
            min_fov_deg: 0.15,
            max_fov_deg: 6.0,
            size: "~1.3GB".to_string(),
            installed: false,
        };
        assert_eq!(resp.min_fov_deg, 0.15);
        assert_eq!(resp.max_fov_deg, 6.0);
    }
}

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
