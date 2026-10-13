//! Camera service for managing camera connections
//!
//! Provides a clean interface for camera discovery, connection, and disconnection.
//! The heavy lifting (holding the long-lived handle, pre-cool, warmup) lives
//! in `crate::camera::lifecycle`. This service is the thin
//! HTTP-facing surface plus camera discovery.

use std::sync::{Arc, Mutex};

use night_amplifier_core::camera::identity::{self, DeviceIdentity};
use night_amplifier_core::camera::{CameraEntry, CameraInfo};
use crate::camera::lifecycle;
use crate::error::{ApiError, ApiResult};
use crate::events::{CameraInfoResponse, CameraListEntry, ServerEvent};
use crate::state::{AppState, CameraPhase, CameraRole, ConnectedCameraInfo, ViewSelection};

/// Service for managing camera operations
pub struct CameraService;

impl CameraService {
    /// Every camera: the connected ones, and what a fresh enumeration of each provider and
    /// the INDI server finds. Seconds of USB work (QHY and ZWO open every idle device), so
    /// only for a first load or an explicit refresh — see [`Self::list_known_cameras`].
    pub async fn list_cameras(state: &Arc<AppState>) -> Vec<CameraListItem> {
        let use_simulated = state.settings.snapshot().use_simulated_camera;
        let devices = Self::discover_cameras(state, use_simulated).await;
        state.discovered_cameras.record_devices(use_simulated, devices.clone());
        Self::spawn_indi_discovery(state);
        Self::assemble(state, devices)
    }

    /// As [`Self::list_cameras`], from what the last enumeration found: what a client
    /// re-reads after a sync. A sync on every reconnect and lag enumerated the USB bus each
    /// time, for every open page, often in the middle of a capture. Enumerates only when
    /// nothing has yet, or the simulator setting changed what there is to find.
    pub async fn list_known_cameras(state: &Arc<AppState>) -> Vec<CameraListItem> {
        let use_simulated = state.settings.snapshot().use_simulated_camera;
        match state.discovered_cameras.devices(use_simulated) {
            Some(devices) => Self::assemble(state, devices),
            None => Self::list_cameras(state).await,
        }
    }

    /// The connected cameras, then every listed device and INDI camera not among them.
    fn assemble(state: &AppState, devices: Vec<CameraEntry>) -> Vec<CameraListItem> {
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

        let connected = state.roster.connected();
        for entry in devices {
            if connected.iter().any(|camera| is_same_device(camera, &entry)) {
                continue;
            }
            cameras_list.push(CameraListItem {
                id: identity::camera_id(&entry.provider, entry.index, entry.info.serial.as_deref()),
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

        cameras_list.extend(
            state
                .discovered_cameras
                .indi()
                .into_iter()
                .filter(|camera| !state.roster.contains(&camera.id)),
        );
        cameras_list
    }

    /// Ask the INDI server for its cameras in the background — it takes seconds to answer —
    /// and announce the ones the last discovery did not find. A server that does not answer
    /// lists none: its cameras stayed listed after it had gone.
    fn spawn_indi_discovery(state: &Arc<AppState>) {
        let settings = state.settings.snapshot();
        let (host, port) = (settings.indi_server_host.clone(), settings.indi_server_port);
        drop(settings);
        if host.is_empty() {
            state.discovered_cameras.replace_indi(Vec::new());
            return;
        }

        let state = Arc::clone(state);
        tokio::spawn(async move {
            let provider = night_amplifier_core::camera::IndiProvider::new(host, port);
            let found = match provider.list_cameras_async().await {
                Ok(cameras) => cameras.into_iter().map(|camera| indi_list_item(&camera)).collect(),
                Err(e) => {
                    tracing::debug!(error = %e, "INDI discovery failed; listing no INDI cameras");
                    Vec::new()
                }
            };
            for camera in state.discovered_cameras.replace_indi(found) {
                if !state.roster.contains(&camera.id) {
                    let _ = state.events.send(ServerEvent::camera_discovered(camera.entry()));
                }
            }
        });
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

    /// The observer dismissed `role`'s "could not bring it back" warning. Returns whether
    /// there was one; every client hears `camera_recovery_cleared` if so.
    pub fn dismiss_recovery_failure(state: &AppState, role: CameraRole) -> bool {
        state.recovery_status.dismiss_failure(role)
    }

    /// Show `role`'s camera on the eyepiece pages; a change reaches every client as an event.
    pub fn select_viewed_camera(state: &AppState, role: CameraRole) -> ApiResult<()> {
        match state
            .viewed_camera
            .select(role, |role| state.roster.delivers_frames(role))
        {
            ViewSelection::Changed | ViewSelection::Unchanged => Ok(()),
            ViewSelection::Refused => Err(ApiError::NoGuideCameraToView),
        }
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

/// What the last enumeration found: every provider's devices (connected ones included, so
/// a camera disconnected since is listed again), keyed by the simulator setting it ran
/// under, and the INDI server's cameras, which every list carries — a list without them
/// dropped the INDI cameras its `camera_discovered` events had added.
#[derive(Default)]
pub struct DiscoveredCameras {
    devices: Mutex<Option<(bool, Vec<CameraEntry>)>>,
    indi: Mutex<Vec<CameraListItem>>,
}

impl DiscoveredCameras {
    fn record_devices(&self, use_simulated: bool, devices: Vec<CameraEntry>) {
        *self.devices.lock().unwrap_or_else(|e| e.into_inner()) = Some((use_simulated, devices));
    }

    /// The devices the last enumeration under `use_simulated` found; `None` before one.
    fn devices(&self, use_simulated: bool) -> Option<Vec<CameraEntry>> {
        let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .as_ref()
            .filter(|(simulated, _)| *simulated == use_simulated)
            .map(|(_, devices)| devices.clone())
    }

    fn indi_lock(&self) -> std::sync::MutexGuard<'_, Vec<CameraListItem>> {
        self.indi.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn indi(&self) -> Vec<CameraListItem> {
        self.indi_lock().clone()
    }

    /// Replace the INDI cameras with `found`, returning the ones it did not hold before.
    fn replace_indi(&self, found: Vec<CameraListItem>) -> Vec<CameraListItem> {
        let mut known = self.indi_lock();
        let new = found
            .iter()
            .filter(|camera| !known.iter().any(|old| old.id == camera.id))
            .cloned()
            .collect();
        *known = found;
        new
    }
}

fn indi_list_item(camera: &CameraInfo) -> CameraListItem {
    const PROVIDER: &str = "indi";
    CameraListItem {
        id: format!("{PROVIDER}_{}", camera.id),
        name: camera.name.clone(),
        connected: false,
        provider: Some(PROVIDER.to_string()),
        index: Some(camera.id as usize),
        role: None,
        phase: None,
        warmup_remaining: None,
        info: camera.clone(),
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
    pub info: CameraInfo,
}

impl CameraListItem {
    /// How a client sees it, in `GET /api/cameras` and `camera_discovered`.
    pub fn entry(self) -> CameraListEntry {
        CameraListEntry {
            info: CameraInfoResponse::from_info(&self.info, &self.id),
            id: self.id,
            name: self.name,
            connected: self.connected,
            provider: self.provider,
            index: self.index,
            role: self.role,
            phase: self.phase.map(Into::into),
            warmup_remaining_s: self.warmup_remaining.map(|left| left.as_secs()),
        }
    }
}

#[cfg(test)]
#[path = "camera_service_tests.rs"]
mod tests;
