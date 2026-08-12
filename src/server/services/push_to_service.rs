//! Push-To Navigation Service
//!
//! Service layer for plate solving and telescope navigation guidance.

use std::sync::Arc;
use tokio::sync::RwLock;

use super::super::dto::{
    CatalogEntryResponse, CoordinateResponse, PushToDirectionResponse, PushToStatusResponse,
};
use super::super::events::ServerEvent;
use super::super::state::{AppState, TelescopeSettings};
use crate::push_to::{PushToError, PUSH_TO_PLUGIN};

/// Push-To navigation service
pub struct PushToService;

impl PushToService {
    /// Get the current Push-To status
    ///
    /// Read-only on purpose. `PushToState` mirrors the plugin so the stacking
    /// thread can gate plate solving without awaiting it, but a status poll is
    /// the wrong place to repair that mirror: `solving_in_progress` is a latch
    /// owned by `try_plate_solve`, and clearing it from here would let a second
    /// solve start under one already in flight. The mirror is maintained by the
    /// target mutations below and re-synced from `try_plate_solve`'s own
    /// authoritative read of the plugin.
    pub async fn get_status(_state: &AppState) -> PushToStatusResponse {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.get_status().await
        } else {
            PushToStatusResponse {
                solver_ready: false,
                is_solving: false,
                current_target: None,
                last_position: None,
                direction: None,
            }
        }
    }

    /// Cancel current plate solving process
    pub async fn cancel_solve(state: &AppState) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            let result = plugin.cancel_solve().await.map_err(|e| e.to_string());
            // Clear solving status on frontend immediately
            let _ = state
                .events
                .send(ServerEvent::position_solve_failed("Cancelled by user"));
            result
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Search the catalog
    pub async fn search_catalog(
        _state: &AppState,
        query: &str,
        limit: usize,
    ) -> Vec<CatalogEntryResponse> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
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
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.get_catalog_by_type(catalog_type_str).await
        } else {
            vec![]
        }
    }

    /// Set target by name
    pub async fn set_target_by_name(
        state: &AppState,
        name: &str,
    ) -> Result<CatalogEntryResponse, String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            let result = plugin.set_target_by_name(name).await?;
            state.set_push_to_has_target(true).await;
            let _ = state.events.send(ServerEvent::target_changed(
                result.name.clone(),
                Some(result.designation.clone()),
                result.ra_degrees,
                result.dec_degrees,
            ));
            Ok(result)
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Set target by coordinates
    pub async fn set_target_by_coords(
        state: &AppState,
        ra_degrees: f64,
        dec_degrees: f64,
    ) -> Result<CoordinateResponse, String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            let result = plugin.set_target_by_coords(ra_degrees, dec_degrees).await?;
            state.set_push_to_has_target(true).await;
            // For custom coordinates, name is usually the coordinate string
            let _ = state.events.send(ServerEvent::target_changed(
                Some(result.ra_string.clone() + " " + &result.dec_string),
                None,
                result.ra_degrees,
                result.dec_degrees,
            ));
            Ok(result)
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Clear the current target
    pub async fn clear_target(state: &AppState) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            let result = plugin.clear_target().await;
            // Only mirror a clear that actually happened — a failed clear leaves
            // the plugin holding the target, and claiming otherwise would stop
            // plate solving for a target that is still set.
            if result.is_ok() {
                state.set_push_to_has_target(false).await;
            }
            let _ = state.events.send(ServerEvent::target_cleared());
            result
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Get the push direction (if position and target are both set)
    pub async fn get_direction(_state: &AppState) -> Option<PushToDirectionResponse> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.get_direction().await
        } else {
            None
        }
    }

    /// Update the FOV hint for the solver
    pub async fn set_fov(_state: &AppState, fov_degrees: f32) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.set_fov(fov_degrees).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Update telescope settings on the solver for precise FOV calculation
    pub async fn set_telescope_settings(
        _state: &AppState,
        settings: TelescopeSettings,
    ) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.set_telescope_settings(settings).await
        } else {
            Ok(()) // No plugin available; not an error
        }
    }

    /// Load a solver database
    pub async fn load_database(_state: &AppState, path: &str) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.load_database(path).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }
}

/// Server-side mirror of the Push-To plugin, so the stacking thread can decide
/// whether a plate solve is worth preparing a frame for without awaiting the
/// plugin's own locks.
///
/// Both fields are caches, not the source of truth — the plugin is. They exist
/// only to keep `capture::solving::plate_solve_available` synchronous and cheap;
/// every consequential check is repeated against the plugin inside
/// `try_plate_solve`. Write `has_target` through
/// [`AppState::set_push_to_has_target`].
#[derive(Default)]
pub struct PushToState {
    /// Latch owned by `try_plate_solve`: set before a solve is spawned, cleared
    /// when it finishes. Nothing else may write it.
    pub solving_in_progress: bool,
    /// Whether the plugin currently holds a target. Written by the target
    /// mutations in [`PushToService`] and re-synced from `try_plate_solve`.
    pub has_target: bool,
}
