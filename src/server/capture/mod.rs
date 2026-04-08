//! Capture loop implementation
//!
//! This module contains the main capture loop that runs in a background task
//! when capture is started via the API.

mod context;
mod pipeline;
mod solving;
mod storage;

use std::sync::Arc;
use tracing::{debug, error, info, warn};

use super::encoding::encode_rgb8_lz4;
use super::state::{AppState, CaptureState, StackingType};

use crate::stacking::CometContext;
pub use context::{PlanetaryStackingContext, StackingContext};

/// The main capture loop that runs in a background task
pub async fn run_capture_loop(state: Arc<AppState>, camera_id: String) {
    use crate::camera::CameraRegistry;

    // Transition to capturing state
    state.set_capture_state(CaptureState::Capturing).await;

    debug!(camera_id = %camera_id, "Capture loop starting");

    // Get camera info for opening
    let camera_info = match storage::get_camera_info(&state, &camera_id).await {
        Some(info) => info,
        None => {
            error!(camera_id = %camera_id, "Camera not found in capture loop");
            state.send_error("Camera not found".to_string());
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    // Initialize capture session
    if let Err(e) = storage::initialize_capture_session(&state).await {
        error!(error = %e, "Failed to initialize capture session");
        state.send_error(e);
        state.set_capture_state(CaptureState::Idle).await;
        return;
    }

    // Track queue warning state to avoid spamming events
    let mut queue_warning_active = false;

    // Open camera for capture
    let mut registry = CameraRegistry::new();
    registry.register_defaults();

    let mut camera = match registry.open_camera(&camera_info.provider, camera_info.index) {
        Ok(cam) => {
            debug!(
                camera_id = %camera_id,
                provider = %camera_info.provider,
                "Camera opened for capture"
            );
            cam
        }
        Err(e) => {
            error!(camera_id = %camera_id, error = %e, "Failed to open camera for capture");
            state.send_error(format!("Failed to open camera: {}", e));
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    // Register active camera cancel token in state
    state.set_active_camera_token(camera.cancel_token()).await;

    // Stacking contexts - initialized on first frame when stacking is enabled
    let mut stacking_ctx: Option<StackingContext> = None;
    let mut comet_ctx: Option<Box<dyn CometContext>> = None;
    let mut planetary_ctx: Option<PlanetaryStackingContext> = None;
    let mut stacking_failed = false;
    // Track previous stacking state to detect toggles
    let mut was_stacking_enabled = false;
    // Track stacking type to detect mode changes
    let mut last_stacking_type = StackingType::DeepSky;

    loop {
        // Check for cancellation
        if state.is_cancelled() {
            break;
        }

        // Get current settings
        let settings = state.settings.read().await.clone();
        let capture_config = settings.to_capture_config();

        // Detect when stacking is toggled on or stacking type changes - reset context
        let stacking_enabled = settings.stacking && settings.stacking_type.supports_stacking();
        let stacking_type_changed = settings.stacking_type != last_stacking_type;

        if (stacking_enabled && !was_stacking_enabled)
            || (stacking_enabled && stacking_type_changed)
        {
            // Stacking was just enabled or mode changed, reset contexts and counters
            stacking_ctx = None;
            comet_ctx = None;
            planetary_ctx = None;
            stacking_failed = false;
            state.reset_counters().await;
            info!(
                stacking_type = ?settings.stacking_type,
                "Live stacking enabled/changed, resetting context and counters"
            );
        }
        was_stacking_enabled = stacking_enabled;
        last_stacking_type = settings.stacking_type;

        // Capture a frame
        let frame = match camera.capture(&capture_config) {
            Ok(f) => f,
            Err(e) => {
                if let crate::camera::CameraError::Cancelled = e {
                    debug!("Capture cancelled (likely due to settings update), starting next frame");
                    // Reset cancel flag so next capture isn't immediately cancelled
                    camera.cancel_token().store(false, std::sync::atomic::Ordering::SeqCst);
                    continue;
                }

                warn!(camera_id = %camera_id, error = %e, "Frame capture failed");
                state.frame_rejected(format!("Capture failed: {}", e)).await;

                // If we get too many consecutive errors, stop
                if storage::should_stop_on_errors(&state).await {
                    error!(camera_id = %camera_id, "Too many capture failures, stopping");
                    state.send_error("Too many capture failures, stopping".to_string());
                    break;
                }
                continue;
            }
        };

        if state.is_cancelled() {
            break;
        }


        // Process frame
                // Get frame number for this capture
                let frame_number = {
                    let session = state.session.read().await;
                    session.frame_count + 1
                };

                // Save raw frame to disk if enabled (only in stacking mode, not live view or wanderer)
                let is_stacking_mode = settings.stacking && !settings.wanderer_mode;
                if settings.save_raw_frames && is_stacking_mode && state.disk_writer.is_enabled() {
                    queue_warning_active = storage::save_frame_to_disk(
                        &state,
                        &frame,
                        frame_number,
                        &settings,
                        &camera_info,
                        queue_warning_active,
                    )
                    .await;
                }

                // Process frame through stacking pipeline if enabled
                let mut registration_succeeded = true;
                let mut display_frame = if stacking_enabled && !stacking_failed {
                    // Check if frame dimensions match existing context (e.g. after binning change)
                    let dimension_mismatch = if let Some(ctx) = stacking_ctx.as_ref() {
                        frame.width() != ctx.width()
                            || frame.height() != ctx.height()
                            || frame.channels() != ctx.channels()
                    } else if let Some(ctx) = comet_ctx.as_ref() {
                        frame.width() != ctx.width()
                            || frame.height() != ctx.height()
                            || frame.channels() != ctx.channels()
                    } else if let Some(ctx) = planetary_ctx.as_ref() {
                        frame.width() != ctx.width()
                            || frame.height() != ctx.height()
                            || frame.channels() != ctx.channels()
                    } else {
                        false
                    };

                    if dimension_mismatch {
                        info!("Frame dimensions changed (likely due to binning change), resetting stack");
                        stacking_ctx = None;
                        comet_ctx = None;
                        planetary_ctx = None;
                        state.reset_counters().await;
                    }

                    debug!(
                        stacking = settings.stacking,
                        stacking_type = ?settings.stacking_type,
                        "Processing frame through stacking pipeline"
                    );
                    let (res_frame, matched) = match settings.stacking_type {
                        StackingType::Comet => {
                            crate::server::capture::pipeline::process_frame_with_comet_stacking(
                                &frame,
                                &settings,
                                &mut comet_ctx,
                                &mut stacking_failed,
                            )
                            .await
                        }
                        StackingType::Planetary => {
                            crate::server::capture::pipeline::process_frame_with_planetary_stacking(
                                &frame,
                                &settings,
                                &mut planetary_ctx,
                                &mut stacking_failed,
                            )
                            .await
                        }
                        _ => {
                            // DeepSky and future types that use star registration
                            crate::server::capture::pipeline::process_frame_with_stacking(
                                &frame,
                                &settings,
                                &mut stacking_ctx,
                                &mut stacking_failed,
                            )
                            .await
                        }
                    };
                    registration_succeeded = matched;
                    res_frame
                } else {
                    debug!(
                        stacking = settings.stacking,
                        stacking_type = ?settings.stacking_type,
                        stacking_failed = stacking_failed,
                        "Stacking disabled or failed, using raw frame"
                    );
                    registration_succeeded = false;
                    frame.clone()
                };

                // If stacking is enabled but registration failed, fallback to raw frame for live view
                // so the user doesn't see a frozen image.
                if stacking_enabled && !registration_succeeded {
                    debug!("Registration failed, falling back to raw frame for live view");
                    display_frame = frame.clone();
                }

                // Wanderer mode: reset stack if movement detected (registration failed)
                if settings.wanderer_mode && stacking_enabled && !registration_succeeded {
                    info!(
                        "Wanderer mode: movement detected (registration failed), resetting stack"
                    );
                    stacking_ctx = None;
                    comet_ctx = None;
                    planetary_ctx = None;
                    state.reset_counters().await;
                    // In wanderer mode, show raw frame when moving
                    display_frame = frame.clone();
                }

                // Track whether this frame was successfully stacked
                let was_stacked = if stacking_enabled {
                    match settings.stacking_type {
                        StackingType::Comet => comet_ctx
                            .as_ref()
                            .map(|ctx| ctx.frame_count() > 0)
                            .unwrap_or(false),
                        StackingType::Planetary => planetary_ctx
                            .as_ref()
                            .map(|ctx| ctx.frame_count() > 0)
                            .unwrap_or(false),
                        _ => stacking_ctx
                            .as_ref()
                            .map(|ctx| ctx.frame_count() > 0)
                            .unwrap_or(false),
                    }
                } else {
                    true
                };

                // Trigger plate solving if target is set and not already solving
                solving::try_plate_solve(&state, &display_frame).await;

                // Process frame through unified render pipeline
                let mut preview_frame = display_frame;
                if let Err(e) = crate::server::capture::pipeline::process_preview_frame(
                    &mut preview_frame,
                    &settings,
                ) {
                    state.send_error(format!("Preview processing failed: {}", e));
                    continue;
                }

                // Encode frame as RGB8+LZ4 for streaming
                let encode_result = {
                    let _encode_span = tracing::info_span!("encode_rgb8_lz4").entered();
                    encode_rgb8_lz4(&preview_frame)
                };
                match encode_result {
                    Ok(encoded_data) => {
                        state.set_latest_frame(encoded_data).await;
                        state.frame_captured(was_stacked).await;
                    }
                    Err(e) => {
                        state
                            .frame_rejected(format!("RGB8+LZ4 encoding failed: {}", e))
                            .await;
                    }
                }

        // Check for cancellation after heavy processing
        if state.is_cancelled() {
            break;
        }

        // Small delay between frames if not cancelled
        if !state.is_cancelled() {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    }

    // Close camera
    let _ = camera.close();
    state.clear_active_camera_token().await;

    // Save stacked result if applicable
    let stacked_frame = stacking_ctx
        .as_ref()
        .and_then(|ctx| ctx.compute().ok())
        .or_else(|| comet_ctx.as_ref().and_then(|ctx| ctx.compute().ok()))
        .or_else(|| planetary_ctx.as_ref().and_then(|ctx| ctx.compute().ok()));
    storage::save_stacked_result(&state, stacked_frame, &camera_info).await;

    // End capture session
    state.disk_writer.end_session().await;

    info!(camera_id = %camera_id, "Capture loop ended");

    // Clean up
    state.set_capture_state(CaptureState::Idle).await;
}
