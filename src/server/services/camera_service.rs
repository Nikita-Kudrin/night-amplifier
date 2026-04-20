//! Camera service for managing camera connections
//!
//! Provides a clean interface for camera discovery, connection, and disconnection.
//! The heavy lifting (holding the long-lived handle, pre-cool, warmup) lives
//! in `crate::server::camera_session::lifecycle`. This service is the thin
//! HTTP-facing surface plus camera discovery.

use std::sync::Arc;

use crate::camera::{CameraEntry, CameraRegistry};
use crate::server::camera_session::lifecycle;
use crate::server::error::{ApiError, ApiResult};
use crate::server::state::{AppState, ConnectedCameraInfo};

/// Service for managing camera operations
pub struct CameraService;

impl CameraService {
    /// List all available cameras (connected + discovered)
    pub async fn list_cameras(state: &AppState) -> Vec<CameraListItem> {
        let mut cameras_list = Vec::new();

        // Add already connected cameras
        {
            let connected = state.cameras.read().await;
            for (id, cam_info) in connected.iter() {
                cameras_list.push(CameraListItem {
                    id: id.clone(),
                    name: cam_info.info.name.clone(),
                    connected: true,
                    provider: Some(cam_info.provider.clone()),
                    index: Some(cam_info.index),
                    info: cam_info.info.clone(),
                });
            }
        }

        // Get current setting for simulated camera
        let use_simulated = state.settings.read().await.use_simulated_camera;

        // Discover available cameras
        let discovered = Self::discover_cameras(use_simulated).await;
        if let Ok(entries) = discovered {
            for entry in entries {
                let id = format!("{}_{}", entry.provider.to_lowercase(), entry.index);

                // Skip if already connected
                let connected = state.cameras.read().await;
                if connected.contains_key(&id) {
                    continue;
                }
                drop(connected);

                cameras_list.push(CameraListItem {
                    id,
                    name: entry.info.name.clone(),
                    connected: false,
                    provider: Some(entry.provider),
                    index: Some(entry.index),
                    info: entry.info,
                });
            }
        }

        cameras_list
    }

    /// Discover cameras using the registry (runs in blocking task)
    async fn discover_cameras(
        use_simulated: bool,
    ) -> Result<Vec<CameraEntry>, crate::camera::CameraError> {
        tokio::task::spawn_blocking(move || {
            std::panic::catch_unwind(move || {
                let mut registry = CameraRegistry::new();

                // Manual registration to allow filtering simulated cameras
                let _ = registry.register(crate::camera::PlayerOneProvider::new());
                let _ = registry.register(crate::camera::ZwoProvider::new());

                if use_simulated {
                    let _ = registry.register(crate::camera::SimulatedProvider::new());
                }

                registry.list_all_cameras()
            })
            .unwrap_or(Err(crate::camera::CameraError::NoCamerasFound))
        })
        .await
        .unwrap_or(Err(crate::camera::CameraError::NoCamerasFound))
    }

    /// Get information about a specific connected camera
    pub async fn get_camera_info(
        state: &AppState,
        camera_id: &str,
    ) -> ApiResult<ConnectedCameraInfo> {
        let cameras = state.cameras.read().await;
        cameras
            .get(camera_id)
            .cloned()
            .ok_or_else(|| ApiError::CameraNotFound(camera_id.to_string()))
    }

    /// Connect to a camera (delegates to lifecycle — opens the handle and
    /// begins pre-cool when applicable).
    pub async fn connect_camera(
        state: &Arc<AppState>,
        camera_id: &str,
    ) -> ApiResult<ConnectedCameraInfo> {
        lifecycle::connect(state, camera_id).await
    }

    /// Disconnect from a camera (delegates to lifecycle — triggers warmup
    /// asynchronously if the cooler was running).
    pub async fn disconnect_camera(state: &Arc<AppState>, camera_id: &str) -> ApiResult<String> {
        lifecycle::disconnect(state, camera_id).await
    }
}

/// Camera list item returned by list_cameras
#[derive(Debug, Clone)]
pub struct CameraListItem {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub provider: Option<String>,
    pub index: Option<usize>,
    pub info: crate::camera::CameraInfo,
}
