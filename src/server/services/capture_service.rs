//! Capture service for managing capture sessions
//!
//! Encapsulates capture-related business logic including starting, stopping,
//! and monitoring capture sessions.

use std::sync::Arc;
use tracing::info;

use crate::server::capture::{guide_task, run_capture_loop};
use crate::server::error::{ApiError, ApiResult};
use crate::server::state::{AppState, CameraRole, CaptureState, SessionResumePlan};

/// Service for managing capture operations
pub struct CaptureService;

impl CaptureService {
    /// Start the camera in `role`.
    ///
    /// The two roles run different things and the button addresses whichever camera the
    /// list has selected: the imaging camera drives the four-thread capture pipeline,
    /// the guide camera its own free-running loop. Only the imaging camera takes a
    /// `camera_id` — the guide slot holds at most one camera, so naming it adds nothing
    /// beyond a check that the client and the server agree on which one it is.
    pub async fn start(
        state: &Arc<AppState>,
        camera_id: Option<String>,
        role: CameraRole,
    ) -> ApiResult<String> {
        match role {
            CameraRole::Main => Self::start_capture(state, camera_id).await,
            CameraRole::Guide => Self::start_guide(state, camera_id).await,
        }
    }

    /// Stop the camera in `role`. Returns whether anything was running.
    pub async fn stop(state: &Arc<AppState>, role: CameraRole) -> bool {
        match role {
            CameraRole::Main => Self::stop_capture(state).await,
            CameraRole::Guide => Self::stop_guide(state).await,
        }
    }

    /// Start the guide camera's free-running loop.
    ///
    /// Ordinarily `connect` has already started it — framing and plate solving want it
    /// before any imaging session begins — so this is the restart after a deliberate
    /// Stop.
    async fn start_guide(state: &Arc<AppState>, camera_id: Option<String>) -> ApiResult<String> {
        let camera = state
            .camera_in_role(CameraRole::Guide)
            .await
            .ok_or(ApiError::NoGuideCameraConnected)?;

        // A client that named a camera gets told when it named the wrong one, rather
        // than silently starting the camera it did not ask for.
        if let Some(id) = camera_id {
            if id != camera.id {
                return match state.role_of(&id).await {
                    Some(role) => Err(ApiError::CameraRoleMismatch {
                        camera: state.connected_camera_name(&id).await.unwrap_or(id),
                        held: role.label(),
                        requested: CameraRole::Guide.label(),
                    }),
                    None => Err(ApiError::CameraNotConnected(id)),
                };
            }
        }

        if state.guide_loop_running() {
            return Err(ApiError::GuideAlreadyRunning);
        }

        info!(camera_id = %camera.id, "Starting the guide camera");
        guide_task::start(state, &camera);
        Ok(camera.id)
    }

    /// Stop the guide camera's loop, which also stops its raw-frame saving.
    async fn stop_guide(state: &Arc<AppState>) -> bool {
        if !state.guide_loop_running() {
            return false;
        }
        info!("Stopping the guide camera");
        guide_task::stop(state).await;

        // A deliberate stop ends the observation, the same call `stop_capture` makes on
        // the imaging camera's resume plan: drop the folder the loop was filling so a
        // later Start opens a fresh one instead of resuming the numbering into it.
        // Deliberately not inside `guide_task::stop` — the disconnect paths call that
        // too, and a dropout *must* keep the record so the reconnect rejoins one
        // session's frames into one folder.
        *state.slot(CameraRole::Guide).raw_session.write().await = None;

        // Nothing will refresh the guide preview until it is started again, so let go of
        // the last frame rather than leaving a viewer looking at a still image of a
        // camera that stopped.
        state.guide_stream.clear().await;
        true
    }

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

        // A capture always runs on the imaging camera. `selected_camera` is what the
        // settings panel is editing, which since roles exist can be the guide camera —
        // resolving through it would start a stacking session on the guide scope.
        let camera_id = match camera_id {
            Some(id) => id,
            None => state
                .camera_in_role(CameraRole::Main)
                .await
                .map(|info| info.id)
                .ok_or(ApiError::NoCameraSelected)?,
        };

        // Verify the named camera is connected, and is the imaging one.
        //
        // Both arms name the camera rather than its id: the id is what the client sent,
        // not something the reader recognises. A camera that is not connected has no
        // name to look up, so that arm falls back to the id — which is the honest
        // answer there, since nothing in the rig claims it.
        match state.role_of(&camera_id).await {
            Some(CameraRole::Main) => {}
            Some(role) => {
                let camera = state
                    .connected_camera_name(&camera_id)
                    .await
                    .unwrap_or(camera_id);
                return Err(ApiError::CaptureCameraIsNotMain {
                    camera,
                    held: role.label(),
                });
            }
            None => return Err(ApiError::CameraNotConnected(camera_id)),
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
