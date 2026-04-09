//! Camera service for managing camera connections
//!
//! Provides a clean interface for camera discovery, connection, and disconnection.

use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::camera::{CameraEntry, CameraRegistry};
use crate::server::error::{ApiError, ApiResult};
use crate::server::events::ServerEvent;
use crate::server::state::{AppState, CaptureState, ConnectedCameraInfo};
use crate::telemetry::metrics as telemetry_metrics;

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

    /// Connect to a camera
    pub async fn connect_camera(
        state: &Arc<AppState>,
        camera_id: &str,
    ) -> ApiResult<ConnectedCameraInfo> {
        // Check if already connected
        {
            let cameras = state.cameras.read().await;
            if let Some(info) = cameras.get(camera_id) {
                return Ok(info.clone());
            }
        }

        // Parse camera ID
        let (provider_name, index) = Self::parse_camera_id(camera_id)?;

        // Get current setting for simulated camera
        let use_simulated = state.settings.read().await.use_simulated_camera;

        // Open camera in blocking task
        let provider_name_clone = provider_name.to_string();
        let open_result = tokio::task::spawn_blocking(move || {
            let mut registry = CameraRegistry::new();

            // Register providers manually to respect the flag
            let _ = registry.register(crate::camera::PlayerOneProvider::new());
            let _ = registry.register(crate::camera::ZwoProvider::new());
            if use_simulated {
                let _ = registry.register(crate::camera::SimulatedProvider::new());
            }

            // Find matching provider name (case-insensitive)
            let provider_registry_name = registry
                .providers()
                .into_iter()
                .find(|name| name.to_lowercase() == provider_name_clone.to_lowercase())
                .map(|s| s.to_string())
                .unwrap_or_else(|| provider_name_clone.clone());

            match registry.open_camera(&provider_registry_name, index) {
                Ok(camera) => Ok((camera.info().clone(), provider_registry_name)),
                Err(e) => Err(e),
            }
        })
        .await;

        match open_result {
            Ok(Ok((info, provider_registry_name))) => {
                info!(
                    camera_id = %camera_id,
                    camera_name = %info.name,
                    provider = %provider_registry_name,
                    "Camera connected"
                );
                debug!(
                    camera_id = %camera_id,
                    specifications = ?info,
                    "Camera specifications"
                );

                let camera_info = ConnectedCameraInfo {
                    id: camera_id.to_string(),
                    provider: provider_registry_name,
                    index,
                    info: info.clone(),
                };

                // Store camera info
                {
                    let mut cameras = state.cameras.write().await;
                    cameras.insert(camera_id.to_string(), camera_info.clone());
                    telemetry_metrics::record_cameras_count(cameras.len() as u64);
                }

                // Set as selected if none selected
                {
                    let mut selected = state.selected_camera.write().await;
                    if selected.is_none() {
                        *selected = Some(camera_id.to_string());
                    }
                }

                // Broadcast event
                let _ = state.events.send(ServerEvent::camera_connected(&info.name));

                Ok(camera_info)
            }
            Ok(Err(e)) => {
                error!(camera_id = %camera_id, error = %e, "Failed to open camera");
                Err(ApiError::CameraOpenFailed(e.to_string()))
            }
            Err(e) => {
                error!(camera_id = %camera_id, error = %e, "Blocking task failed");
                Err(ApiError::Internal(e.to_string()))
            }
        }
    }

    /// Disconnect from a camera
    pub async fn disconnect_camera(state: &Arc<AppState>, camera_id: &str) -> ApiResult<String> {
        // Check if capturing with this camera
        let current_state = state.capture_state().await;
        if current_state == CaptureState::Capturing || current_state == CaptureState::Starting {
            let selected = state.selected_camera.read().await;
            if selected.as_ref() == Some(&camera_id.to_string()) {
                return Err(ApiError::CameraInUse);
            }
        }

        // Remove camera
        let camera_name = {
            let mut cameras = state.cameras.write().await;
            let result = cameras.remove(camera_id).map(|c| c.info.name);
            telemetry_metrics::record_cameras_count(cameras.len() as u64);
            result
        };

        match camera_name {
            Some(name) => {
                info!(camera_id = %camera_id, camera_name = %name, "Camera disconnected");

                // Clear selected if this was selected
                {
                    let mut selected = state.selected_camera.write().await;
                    if selected.as_ref() == Some(&camera_id.to_string()) {
                        *selected = None;
                    }
                }

                // Broadcast event
                let _ = state.events.send(ServerEvent::camera_disconnected(&name));

                Ok(name)
            }
            None => {
                warn!(camera_id = %camera_id, "Attempted to disconnect non-connected camera");
                Err(ApiError::CameraNotConnected(camera_id.to_string()))
            }
        }
    }

    /// Parse camera ID into provider name and index
    fn parse_camera_id(camera_id: &str) -> ApiResult<(&str, usize)> {
        let parts: Vec<&str> = camera_id.splitn(2, '_').collect();
        if parts.len() != 2 {
            return Err(ApiError::InvalidCameraIdFormat);
        }

        let provider_name = parts[0];
        let index: usize = parts[1].parse().map_err(|_| ApiError::InvalidCameraIndex)?;

        Ok((provider_name, index))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_camera_id_valid() {
        let result = CameraService::parse_camera_id("playerone_0");
        assert!(result.is_ok());
        let (provider, index) = result.unwrap();
        assert_eq!(provider, "playerone");
        assert_eq!(index, 0);
    }

    #[test]
    fn test_parse_camera_id_with_underscore_in_provider() {
        // splitn(2, '_') splits at the first underscore only
        // "player_one_0" -> ["player", "one_0"] where "one_0" is not a valid number
        let result = CameraService::parse_camera_id("player_one_0");
        assert!(matches!(result, Err(ApiError::InvalidCameraIndex)));
    }

    #[test]
    fn test_parse_camera_id_invalid_format() {
        let result = CameraService::parse_camera_id("invalidformat");
        assert!(matches!(result, Err(ApiError::InvalidCameraIdFormat)));
    }

    #[test]
    fn test_parse_camera_id_invalid_index() {
        let result = CameraService::parse_camera_id("provider_notanumber");
        assert!(matches!(result, Err(ApiError::InvalidCameraIndex)));
    }
}
