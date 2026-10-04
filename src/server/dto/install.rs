//! ASTAP and OpenNGC catalog installation DTOs

use serde::Deserialize;

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

/// Catalog installation request
#[derive(Debug, Deserialize)]
pub struct CatalogInstallRequest {
    /// Whether to also download the HYG star database (~15MB compressed)
    #[serde(default)]
    pub include_stars: bool,
}
