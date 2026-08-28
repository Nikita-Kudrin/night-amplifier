//! Capture service for managing capture sessions
//!
//! Encapsulates capture-related business logic including starting, stopping,
//! and monitoring capture sessions.

use std::sync::Arc;
use tracing::info;

use crate::server::capture::run_capture_loop;
use crate::server::error::{ApiError, ApiResult};
use crate::server::state::{AppState, CaptureState, SessionResumePlan};

/// Service for managing capture operations
pub struct CaptureService;

impl CaptureService {
    /// Start a capture session
    pub async fn start_capture(
        state: &Arc<AppState>,
        camera_id: Option<String>,
    ) -> ApiResult<String> {
        // Check if already capturing
        let current_state = state.capture_state().await;
        if current_state == CaptureState::Capturing || current_state == CaptureState::Starting {
            return Err(ApiError::CaptureInProgress);
        }

        // Determine which camera to use
        let camera_id = match camera_id {
            Some(id) => id,
            None => state
                .selected_camera
                .read()
                .await
                .clone()
                .ok_or(ApiError::NoCameraSelected)?,
        };

        // Verify camera is connected
        {
            let cameras = state.cameras.read().await;
            if !cameras.contains_key(&camera_id) {
                return Err(ApiError::CameraNotConnected(camera_id));
            }
        }

        // Reset state and start capture. A fresh start discards any stack a
        // previous session parked for a reconnect — only `resume_capture`
        // inherits one.
        state.reset_cancel();
        state.reset_session().await;
        state.clear_stacking_carryover();
        state.set_capture_state(CaptureState::Starting).await;

        info!(camera_id = %camera_id, "Starting capture session");

        // The resume plan is recorded by the capture loop once the disk session
        // exists — recording it here would capture the *previous* session's
        // directory, or none at all.
        Self::spawn_capture(state, camera_id.clone(), None);

        Ok(camera_id)
    }

    /// Restart the capture a device fault interrupted, in the mode it was
    /// running in and on top of the stack it had already accumulated.
    ///
    /// Deliberately not `start_capture`: that resets the session counters and
    /// opens a new raw-frame directory, which for a live-stacking session means
    /// throwing away the whole point of the last hour.
    pub async fn resume_capture(state: &Arc<AppState>, plan: &SessionResumePlan) -> ApiResult<()> {
        let current_state = state.capture_state().await;
        if current_state == CaptureState::Capturing || current_state == CaptureState::Starting {
            return Err(ApiError::CaptureInProgress);
        }
        {
            let cameras = state.cameras.read().await;
            if !cameras.contains_key(&plan.camera_id) {
                return Err(ApiError::CameraNotConnected(plan.camera_id.clone()));
            }
        }

        state.reset_cancel();
        state.set_capture_state(CaptureState::Starting).await;

        info!(camera_id = %plan.camera_id, "Resuming capture session");

        Self::spawn_capture(state, plan.camera_id.clone(), Some(plan.clone()));
        Ok(())
    }

    fn spawn_capture(state: &Arc<AppState>, camera_id: String, resume: Option<SessionResumePlan>) {
        let state = Arc::clone(state);
        tokio::spawn(async move {
            run_capture_loop(state, camera_id, resume).await;
        });
    }

    /// Stop the current capture session
    pub async fn stop_capture(state: &Arc<AppState>) -> bool {
        let current_state = state.capture_state().await;

        if current_state == CaptureState::Idle {
            return false;
        }

        state.request_cancel();
        state.set_capture_state(CaptureState::Stopping).await;

        // A deliberate stop is not something to recover from: drop the resume
        // plan and the parked stack rather than holding full-resolution
        // accumulators until the next session.
        *state.session_resume_plan.write().await = None;
        state.clear_stacking_carryover();

        // Clear Push-To target when capture is stopped
        let _ = super::PushToService::clear_target(state).await;

        info!("Capture session stopping");
        true
    }
}
