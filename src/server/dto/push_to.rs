//! Push-To navigation DTOs

use serde::Deserialize;

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

/// Largest page a catalog search returns: a one-letter query matches most of the ~130k entries,
/// and an unbounded `limit` would serialise all of them
pub const MAX_SEARCH_LIMIT: usize = 100;

impl SearchCatalogRequest {
    pub fn bounded_limit(&self) -> usize {
        self.limit.min(MAX_SEARCH_LIMIT)
    }
}

#[cfg(test)]
mod search_limit_tests {
    use super::*;

    fn request(limit: usize) -> SearchCatalogRequest {
        SearchCatalogRequest {
            query: "M4".to_string(),
            limit,
        }
    }

    #[test]
    fn a_requested_limit_is_kept_up_to_the_maximum() {
        assert_eq!(request(20).bounded_limit(), 20);
        assert_eq!(request(MAX_SEARCH_LIMIT).bounded_limit(), MAX_SEARCH_LIMIT);
        assert_eq!(request(100_000).bounded_limit(), MAX_SEARCH_LIMIT);
    }

    #[test]
    fn an_omitted_limit_is_the_default_page() {
        let request: SearchCatalogRequest = serde_json::from_str(r#"{"query": "M4"}"#).unwrap();
        assert_eq!(request.bounded_limit(), 20);
    }
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
