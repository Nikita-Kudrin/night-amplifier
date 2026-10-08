//! Camera service for managing camera connections
//!
//! Provides a clean interface for camera discovery, connection, and disconnection.
//! The heavy lifting (holding the long-lived handle, pre-cool, warmup) lives
//! in `crate::camera::lifecycle`. This service is the thin
//! HTTP-facing surface plus camera discovery.

use std::sync::Arc;

use night_amplifier_core::camera::identity::{self, DeviceIdentity};
use night_amplifier_core::camera::CameraEntry;
use crate::camera::lifecycle;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CameraPhase, CameraRole, ConnectedCameraInfo};

/// Service for managing camera operations
pub struct CameraService;

impl CameraService {
    /// List all available cameras (connected + discovered)
    pub async fn list_cameras(state: &Arc<AppState>) -> Vec<CameraListItem> {
        let mut cameras_list = Vec::new();

        for (cam_info, phase) in state.roster.connected_with_phases() {
            cameras_list.push(CameraListItem {
                id: cam_info.id,
                name: cam_info.info.name.clone(),
                connected: true,
                provider: Some(cam_info.provider),
                index: Some(cam_info.index),
                role: Some(cam_info.role),
                phase: Some(phase),
                warmup_remaining: state.slot(cam_info.role).warmup_remaining(),
                info: cam_info.info,
            });
        }

        // Get current setting for simulated camera
        let use_simulated = state.settings.snapshot().use_simulated_camera;

        for entry in Self::discover_cameras(state, use_simulated).await {
            let id = identity::camera_id(&entry.provider, entry.index, entry.info.serial.as_deref());

            if state.roster.connected().iter().any(|camera| is_same_device(camera, &entry)) {
                continue;
            }

            cameras_list.push(CameraListItem {
                id,
                name: entry.info.name.clone(),
                connected: false,
                provider: Some(entry.provider),
                index: Some(entry.index),
                role: None,
                phase: None,
                warmup_remaining: None,
                info: entry.info,
            });
        }

        // Get INDI settings
        let settings = state.settings.snapshot();
        let indi_host = settings.indi_server_host.clone();
        let indi_port = settings.indi_server_port;
        drop(settings);

        // Spawn INDI discovery in the background
        if !indi_host.is_empty() {
            let event_sender = state.events.clone();
            let state_arc = state.clone();

            tokio::spawn(async move {
                let provider = night_amplifier_core::camera::IndiProvider::new(indi_host, indi_port);
                if let Ok(cameras) = provider.list_cameras_async().await {
                    for cam in cameras {
                        let provider_name = "indi";
                        let id = format!("{}_{}", provider_name, cam.id);

                        if state_arc.roster.contains(&id) {
                            continue;
                        }

                        let entry = crate::events::CameraListEntry {
                            id: id.clone(),
                            name: cam.name.clone(),
                            connected: false,
                            provider: Some(provider_name.to_string()),
                            index: Some(cam.id as usize),
                            role: None,
                            phase: None,
                            warmup_remaining_s: None,
                            info: crate::events::CameraInfoResponse::from_info(&cam, &id),
                        };
                        let _ = event_sender
                            .send(crate::events::ServerEvent::camera_discovered(entry));
                    }
                }
            });
        }

        cameras_list
    }

    /// Cap on one provider's enumeration: QHY and ZWO open every idle device to read its
    /// capabilities, a few seconds each.
    #[cfg(not(test))]
    const DISCOVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
    #[cfg(test)]
    const DISCOVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

    /// Every provider's cameras, in catalog order. Each provider enumerates on its own blocking
    /// thread under [`Self::DISCOVERY_TIMEOUT`], so one hung SDK neither holds up nor hides the
    /// others. A provider whose earlier call is still in flight is waited for and, when still
    /// stuck, skipped: a wedged device costs one parked thread, not one per refresh.
    async fn discover_cameras(state: &AppState, use_simulated: bool) -> Vec<CameraEntry> {
        let catalog = Arc::clone(&state.device_catalog);
        let listings: Vec<_> = catalog
            .provider_names(use_simulated)
            .into_iter()
            .map(|provider| {
                let catalog = Arc::clone(&catalog);
                let calls = state.discovery_calls_for(&provider);
                async move {
                    if !calls.wait_drained(Self::DISCOVERY_TIMEOUT).await {
                        tracing::warn!(
                            %provider,
                            "Camera discovery skipped: its previous enumeration is still inside the SDK"
                        );
                        return Vec::new();
                    }
                    let listing = provider.clone();
                    let listed = calls
                        .run_bounded(Self::DISCOVERY_TIMEOUT, move || {
                            catalog.list(&listing, use_simulated)
                        })
                        .await;
                    match listed {
                        Ok(Ok(entries)) => entries,
                        Ok(Err(e)) => {
                            tracing::debug!(%provider, error = %e, "Camera discovery failed");
                            Vec::new()
                        }
                        Err(e) => {
                            tracing::warn!(
                                %provider,
                                error = ?e,
                                timeout = ?Self::DISCOVERY_TIMEOUT,
                                "Camera discovery did not return"
                            );
                            Vec::new()
                        }
                    }
                }
            })
            .collect();
        futures_util::future::join_all(listings)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// Get information about a specific connected camera
    pub async fn get_camera_info(
        state: &AppState,
        camera_id: &str,
    ) -> ApiResult<ConnectedCameraInfo> {
        state
            .roster
            .get(camera_id)
            .ok_or_else(|| ApiError::CameraNotFound(camera_id.to_string()))
    }

    /// Connect to a camera in `role` (delegates to lifecycle — opens the handle and
    /// begins pre-cool when applicable).
    pub async fn connect_camera(
        state: &Arc<AppState>,
        camera_id: &str,
        role: CameraRole,
    ) -> ApiResult<ConnectedCameraInfo> {
        lifecycle::connect(state, camera_id, role).await
    }

    /// Disconnect from a camera (delegates to lifecycle — stops a running capture, and
    /// warms a cooled camera up first unless `warmup` says to skip it).
    pub async fn disconnect_camera(
        state: &Arc<AppState>,
        camera_id: &str,
        warmup: lifecycle::WarmupPolicy,
    ) -> ApiResult<lifecycle::DisconnectOutcome> {
        lifecycle::disconnect(state, camera_id, warmup).await
    }
}

/// Whether a discovered entry is a camera that is already connected.
///
/// By serial when both sides have one. Otherwise by where the device is listed *now*:
/// a connected camera's entry keeps the id it was connected under, and a recovered one
/// whose device moved keeps an index id that names another device's position — matching
/// on it hid that other camera and offered the connected one again. The entry's `index`
/// is where it was last opened. Not by `CameraInfo::id`, which some providers fill
/// differently when listing than when opening.
fn is_same_device(connected: &ConnectedCameraInfo, entry: &CameraEntry) -> bool {
    if !connected.provider.eq_ignore_ascii_case(&entry.provider) {
        return false;
    }
    let (ours, theirs) = (DeviceIdentity::of(&connected.info), DeviceIdentity::of(&entry.info));
    match (&ours.serial, &theirs.serial) {
        (Some(_), Some(_)) => ours.matches(&theirs),
        _ => connected.index == entry.index && ours.name == theirs.name,
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
    /// The position this camera occupies, or `None` while it is merely discovered.
    pub role: Option<CameraRole>,
    /// The connected camera's lifecycle phase.
    pub phase: Option<CameraPhase>,
    /// Time until a warm-up in progress is cut short, at the latest.
    pub warmup_remaining: Option<std::time::Duration>,
    pub info: night_amplifier_core::camera::CameraInfo,
}

#[cfg(test)]
#[path = "camera_service_tests.rs"]
mod tests;
