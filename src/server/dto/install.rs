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
