//! Push-To Navigation Service
//!
//! Service layer for plate solving and telescope navigation guidance.

use std::sync::Arc;
use tokio::sync::RwLock;

use super::super::dto::{
    CatalogEntryResponse, CoordinateResponse, PushToDirectionResponse, PushToStatusResponse,
};
use super::super::state::AppState;
use crate::push_to::{PushToError, PUSH_TO_PLUGIN};

/// Push-To navigation service
pub struct PushToService;

impl PushToService {
    /// Get the current Push-To status
    pub async fn get_status(_state: &AppState) -> PushToStatusResponse {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.get_status().await
        } else {
            PushToStatusResponse {
                solver_ready: false,
                current_target: None,
                last_position: None,
                direction: None,
            }
        }
    }

    /// Search the catalog
    pub async fn search_catalog(
        _state: &AppState,
        query: &str,
        limit: usize,
    ) -> Vec<CatalogEntryResponse> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.search_catalog(query, limit).await
        } else {
            vec![]
        }
    }

    /// Get all catalog entries of a specific type
    pub async fn get_catalog_by_type(
        _state: &AppState,
        catalog_type_str: &str,
    ) -> Vec<CatalogEntryResponse> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.get_catalog_by_type(catalog_type_str).await
        } else {
            vec![]
        }
    }

    /// Set target by name
    pub async fn set_target_by_name(
        _state: &AppState,
        name: &str,
    ) -> Result<CatalogEntryResponse, String> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.set_target_by_name(name).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Set target by coordinates
    pub async fn set_target_by_coords(
        _state: &AppState,
        ra_degrees: f64,
        dec_degrees: f64,
    ) -> Result<CoordinateResponse, String> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.set_target_by_coords(ra_degrees, dec_degrees).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Clear the current target
    pub async fn clear_target(_state: &AppState) -> Result<(), String> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.clear_target().await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Get the push direction (if position and target are both set)
    pub async fn get_direction(_state: &AppState) -> Option<PushToDirectionResponse> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.get_direction().await
        } else {
            None
        }
    }

    /// Update the FOV hint for the solver
    pub async fn set_fov(_state: &AppState, fov_degrees: f32) -> Result<(), String> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.set_fov(fov_degrees).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Load a solver database
    pub async fn load_database(_state: &AppState, path: &str) -> Result<(), String> {
        if let Some(plugin) = PUSH_TO_PLUGIN.get() {
            plugin.load_database(path).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }
}

/// Dummy state for compatibility with AppState
pub struct PushToState {
    pub solving_in_progress: bool,
}

impl Default for PushToState {
    fn default() -> Self {
        Self {
            solving_in_progress: false,
        }
    }
}
