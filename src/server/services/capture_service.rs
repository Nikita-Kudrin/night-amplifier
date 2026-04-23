//! Capture service for managing capture sessions
//!
//! Encapsulates capture-related business logic including starting, stopping,
//! and monitoring capture sessions.

use std::sync::Arc;
use tracing::info;

use crate::server::capture::run_capture_loop;
use crate::server::error::{ApiError, ApiResult};
use crate::server::state::{AppState, CaptureState};

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

        // Reset state and start capture
        state.reset_cancel();
        state.reset_session().await;
        state.set_capture_state(CaptureState::Starting).await;

        info!(camera_id = %camera_id, "Starting capture session");

        // Spawn capture task using the capture module's run_capture_loop
        // which contains the full stacking pipeline
        let state_clone = Arc::clone(state);
        let camera_id_clone = camera_id.clone();
        tokio::spawn(async move {
            run_capture_loop(state_clone, camera_id_clone).await;
        });

        Ok(camera_id)
    }

    /// Stop the current capture session
    pub async fn stop_capture(state: &Arc<AppState>) -> bool {
        let current_state = state.capture_state().await;

        if current_state == CaptureState::Idle {
            return false;
        }

        state.request_cancel();
        state.set_capture_state(CaptureState::Stopping).await;

        // Clear Push-To target when capture is stopped
        let _ = super::PushToService::clear_target(state).await;

        info!("Capture session stopping");
        true
    }
}
