//! Public API for camera connect/disconnect/handoff orchestration.
//!
//! This layer owns the `AppState.active_camera` handle and coordinates with
//! the monitor thread. `CameraService` and the capture loop delegate to
//! these functions rather than opening/closing handles directly.

use std::sync::Arc;
use tracing::{debug, error, info, warn};

use super::monitor;
use crate::camera::{Camera, CameraRegistry};
use crate::server::error::{ApiError, ApiResult};
use crate::server::events::ServerEvent;
use crate::server::state::{
    AppState, CameraPhase, CaptureState, ConnectedCameraInfo, MonitorCmd,
};
use crate::telemetry::metrics as telemetry_metrics;

/// Open a camera, store the handle long-term, and (optionally) begin
/// pre-cooling. Replaces the old `CameraService::connect_camera` behavior
/// that dropped the handle immediately after probing `CameraInfo`.
pub async fn connect(
    state: &Arc<AppState>,
    camera_id: &str,
) -> ApiResult<ConnectedCameraInfo> {
    // Already connected? Return the existing info — matches the prior
    // idempotent connect behavior.
    {
        let cameras = state.cameras.read().await;
        if let Some(info) = cameras.get(camera_id) {
            return Ok(info.clone());
        }
    }

    let (provider_name, index) = parse_camera_id(camera_id)?;
    let use_simulated = state.settings.read().await.use_simulated_camera;
    let provider_name = provider_name.to_string();

    // Open the camera on a blocking task so the FFI call doesn't occupy a
    // tokio worker. Returned: the handle plus the canonical provider name
    // (case-corrected) plus the CameraInfo.
    let open_result = tokio::task::spawn_blocking(move || -> Result<(Box<dyn Camera>, String), crate::camera::CameraError> {
        let mut registry = CameraRegistry::new();
        let _ = registry.register(crate::camera::PlayerOneProvider::new());
        let _ = registry.register(crate::camera::ZwoProvider::new());
        if use_simulated {
            let _ = registry.register(crate::camera::SimulatedProvider::new());
        }
        let provider_registry_name = registry
            .providers()
            .into_iter()
            .find(|name| name.to_lowercase() == provider_name.to_lowercase())
            .map(|s| s.to_string())
            .unwrap_or_else(|| provider_name.clone());

        let camera = registry.open_camera(&provider_registry_name, index)?;
        Ok((camera, provider_registry_name))
    })
    .await;

    let (mut camera, provider_registry_name) = match open_result {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            error!(camera_id = %camera_id, error = %e, "Failed to open camera");
            return Err(ApiError::CameraOpenFailed(e.to_string()));
        }
        Err(e) => {
            error!(camera_id = %camera_id, error = %e, "Blocking task failed");
            return Err(ApiError::Internal(e.to_string()));
        }
    };

    let info = camera.info().clone();
    let camera_name = info.name.clone();
    info!(
        camera_id = %camera_id,
        camera_name = %camera_name,
        provider = %provider_registry_name,
        "Camera opened and held in AppState"
    );
    debug!(
        camera_id = %camera_id,
        specifications = ?info,
        "Camera specifications"
    );

    // Decide initial phase: if the camera supports cooling and the user has
    // a target in settings, kick off precool right now. Otherwise Idle.
    let (initial_phase, cooler_applied) = {
        let settings = state.settings.read().await;
        if info.has_cooler && settings.cooler_enabled && settings.target_temp_c.is_some() {
            let target = settings.target_temp_c.unwrap();
            match apply_cooler(camera.as_mut(), true, Some(target)) {
                Ok(()) => (CameraPhase::Precooling, true),
                Err(e) => {
                    warn!(error = %e, "Failed to enable cooler on connect — falling back to Idle");
                    (CameraPhase::Idle, false)
                }
            }
        } else {
            (CameraPhase::Idle, false)
        }
    };

    let connected_info = ConnectedCameraInfo {
        id: camera_id.to_string(),
        provider: provider_registry_name,
        index,
        info,
    };

    // Store handle, metadata, selected, phase.
    {
        let mut cameras = state.cameras.write().await;
        cameras.insert(camera_id.to_string(), connected_info.clone());
        telemetry_metrics::record_cameras_count(cameras.len() as u64);
    }
    {
        let mut selected = state.selected_camera.write().await;
        if selected.is_none() {
            *selected = Some(camera_id.to_string());
        }
    }
    *state
        .active_camera
        .lock()
        .expect("active_camera mutex poisoned") = Some(camera);

    state.set_camera_phase(&camera_name, initial_phase).await;

    // Spawn the monitor thread. It will drive Precooling→Idle transition
    // and emit `CameraStatusUpdated` every 2s for any cooled camera.
    let tx = monitor::spawn(
        Arc::clone(state),
        camera_name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state
        .camera_monitor_tx
        .lock()
        .expect("camera_monitor_tx mutex poisoned") = Some(tx);

    let _ = state.events.send(ServerEvent::camera_connected(&camera_name));

    debug!(
        camera_id = %camera_id,
        phase = ?initial_phase,
        cooler_applied,
        "Camera session started"
    );

    Ok(connected_info)
}

/// Disconnect (or begin warmup prior to disconnect) for a camera.
pub async fn disconnect(state: &Arc<AppState>, camera_id: &str) -> ApiResult<String> {
    // Can't disconnect mid-capture — user must stop capture first.
    let current_capture_state = state.capture_state().await;
    if current_capture_state == CaptureState::Capturing
        || current_capture_state == CaptureState::Starting
    {
        let selected = state.selected_camera.read().await;
        if selected.as_ref() == Some(&camera_id.to_string()) {
            return Err(ApiError::CameraInUse);
        }
    }

    let camera_name = {
        let cameras = state.cameras.read().await;
        cameras.get(camera_id).map(|c| c.info.name.clone())
    };
    let Some(camera_name) = camera_name else {
        warn!(camera_id = %camera_id, "Attempted to disconnect non-connected camera");
        return Err(ApiError::CameraNotConnected(camera_id.to_string()));
    };

    let phase = state.camera_phase(&camera_name).await;

    // Already warming up — idempotent no-op.
    if phase == CameraPhase::WarmingUp {
        info!(camera_id = %camera_id, "Disconnect requested but already warming up");
        return Ok(camera_name);
    }

    // Decide whether to warm up: if the user had cooling enabled in settings
    // (current intent) OR the last status sample reported cooler_on, ramp
    // the TEC down before closing the handle. Relying on settings alone is
    // important because the monitor may not have polled yet on fresh connects.
    let cooler_enabled_in_settings = state.settings.read().await.cooler_enabled;
    let cooler_reported_on = state
        .get_camera_status(&camera_name)
        .await
        .map(|s| s.cooler_on)
        .unwrap_or(false);
    let needs_warmup = cooler_enabled_in_settings || cooler_reported_on;

    if needs_warmup {
        // Start warmup; monitor thread will close handle + emit
        // CameraDisconnected when the sensor reaches WARMUP_THRESHOLD_C.
        state
            .set_camera_phase(&camera_name, CameraPhase::WarmingUp)
            .await;
        send_monitor_cmd(state, MonitorCmd::StartWarmup);
        info!(camera_id = %camera_id, "Warmup initiated; disconnect will complete asynchronously");
        Ok(camera_name)
    } else {
        // No cooler active — close immediately.
        finalize_disconnect(state, &camera_name).await;
        Ok(camera_name)
    }
}

/// Take the camera handle for a capture session. Cancels any in-progress
/// warmup and transitions the phase to `Capturing`.
pub async fn take_for_capture(
    state: &Arc<AppState>,
    camera_name: &str,
) -> Result<Box<dyn Camera>, ApiError> {
    let phase = state.camera_phase(camera_name).await;

    if phase == CameraPhase::WarmingUp {
        // User started capture mid-warmup — cancel, re-enable cooler per
        // current settings before handoff.
        debug!(camera_name, "Cancelling warmup: capture requested");
        send_monitor_cmd(state, MonitorCmd::CancelWarmup);
        let settings = state.settings.read().await;
        if settings.cooler_enabled {
            let target = settings.target_temp_c;
            drop(settings);
            if let Some(cam) = state
                .active_camera
                .lock()
                .expect("active_camera mutex poisoned")
                .as_mut()
            {
                let _ = apply_cooler(cam.as_mut(), true, target);
            }
        }
    }

    // Tell the monitor to pause; it will observe `Capturing` phase and skip
    // its polling loop. This avoids contention with capture's own calls.
    send_monitor_cmd(state, MonitorCmd::HandOffToCapture);

    let camera = state
        .active_camera
        .lock()
        .expect("active_camera mutex poisoned")
        .take()
        .ok_or_else(|| {
            ApiError::Internal(format!(
                "Camera '{}' has no active handle to take for capture",
                camera_name
            ))
        })?;

    state
        .set_camera_phase(camera_name, CameraPhase::Capturing)
        .await;

    Ok(camera)
}

/// Return the handle after a capture session ends. If the capture thread
/// lost the handle (e.g., panicked), we transition straight to Disconnected.
pub async fn return_from_capture(
    state: &Arc<AppState>,
    camera_name: &str,
    camera: Option<Box<dyn Camera>>,
) {
    match camera {
        Some(cam) => {
            *state
                .active_camera
                .lock()
                .expect("active_camera mutex poisoned") = Some(cam);

            // Decide phase: if cooling is enabled and we're not yet near target,
            // precooling; otherwise idle. We use the last cached status as a
            // cheap proxy — the monitor will correct it on the next poll.
            let settings = state.settings.read().await;
            let target = settings.target_temp_c;
            let cooler_enabled = settings.cooler_enabled;
            drop(settings);

            let next_phase = if cooler_enabled && target.is_some() {
                match state.get_camera_status(camera_name).await {
                    Some(status)
                        if (status.temperature_c - target.unwrap()).abs()
                            <= super::PRECOOL_TOLERANCE_C =>
                    {
                        CameraPhase::Idle
                    }
                    _ => CameraPhase::Precooling,
                }
            } else {
                CameraPhase::Idle
            };

            state.set_camera_phase(camera_name, next_phase).await;
            send_monitor_cmd(state, MonitorCmd::ResumeAfterCapture);
        }
        None => {
            // Capture thread crashed or returned without the handle.
            warn!(camera_name, "Capture ended without returning handle; cleaning up");
            finalize_disconnect(state, camera_name).await;
        }
    }
}

/// Close the handle, drop state, broadcast `CameraDisconnected`, and
/// transition phase to `Disconnected`. Used by both immediate-disconnect
/// (no warmup) and warmup-completion paths.
pub async fn finalize_disconnect(state: &Arc<AppState>, camera_name: &str) {
    // Shut down the monitor thread first.
    send_monitor_cmd(state, MonitorCmd::Shutdown);
    {
        let mut tx_guard = state
            .camera_monitor_tx
            .lock()
            .expect("camera_monitor_tx mutex poisoned");
        *tx_guard = None;
    }

    // Close and drop the handle.
    if let Some(mut cam) = state
        .active_camera
        .lock()
        .expect("active_camera mutex poisoned")
        .take()
    {
        if let Err(e) = cam.close() {
            warn!(error = %e, "camera.close() failed — dropping anyway");
        }
    }

    // Drop metadata and status, clear selected.
    let removed_id = {
        let mut cameras = state.cameras.write().await;
        let id = cameras
            .iter()
            .find(|(_, v)| v.info.name == camera_name)
            .map(|(k, _)| k.clone());
        if let Some(ref id) = id {
            cameras.remove(id);
            telemetry_metrics::record_cameras_count(cameras.len() as u64);
        }
        id
    };
    if let Some(ref id) = removed_id {
        let mut selected = state.selected_camera.write().await;
        if selected.as_ref() == Some(id) {
            *selected = None;
        }
    }
    {
        let mut statuses = state.latest_camera_status.write().await;
        statuses.remove(camera_name);
    }

    state
        .set_camera_phase(camera_name, CameraPhase::Disconnected)
        .await;
    let _ = state.events.send(ServerEvent::camera_disconnected(camera_name));

    info!(camera_name, "Camera disconnected");
}

/// Convenience: apply cooler enable + target temperature under the handle mutex.
/// The caller must already hold the mutex guard.
pub(super) fn apply_cooler(
    camera: &mut dyn Camera,
    enabled: bool,
    target_temp_c: Option<f64>,
) -> crate::camera::CameraResult<()> {
    if let Some(t) = target_temp_c {
        camera.set_target_temperature(t)?;
    }
    camera.set_cooler(enabled)?;
    Ok(())
}

/// Send a monitor command, swallowing failures if the monitor has exited.
fn send_monitor_cmd(state: &Arc<AppState>, cmd: MonitorCmd) {
    if let Some(tx) = state
        .camera_monitor_tx
        .lock()
        .expect("camera_monitor_tx mutex poisoned")
        .as_ref()
    {
        let _ = tx.send(cmd);
    }
}

/// Parse camera ID into provider name and index (e.g. "playerone_0" → ("playerone", 0)).
pub(crate) fn parse_camera_id(camera_id: &str) -> ApiResult<(&str, usize)> {
    let parts: Vec<&str> = camera_id.splitn(2, '_').collect();
    if parts.len() != 2 {
        return Err(ApiError::InvalidCameraIdFormat);
    }
    let index: usize = parts[1].parse().map_err(|_| ApiError::InvalidCameraIndex)?;
    Ok((parts[0], index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_camera_id_valid() {
        let (provider, index) = parse_camera_id("playerone_0").unwrap();
        assert_eq!(provider, "playerone");
        assert_eq!(index, 0);
    }

    #[test]
    fn parse_camera_id_invalid_format() {
        assert!(matches!(
            parse_camera_id("invalidformat"),
            Err(ApiError::InvalidCameraIdFormat)
        ));
    }

    #[test]
    fn parse_camera_id_invalid_index() {
        assert!(matches!(
            parse_camera_id("provider_notanumber"),
            Err(ApiError::InvalidCameraIndex)
        ));
    }
}
