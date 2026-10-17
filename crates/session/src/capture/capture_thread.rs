//! The camera side of a capture session: the probe frame, then a dedicated OS thread
//! taking frames and handing them to the stacking and storage channels.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, warn};

use super::channel::{self, CapturedFrame, QueueDepth};
use super::config_overrides::*;
use super::drop_log::DropLog;
use super::stall::{handle_stall, StallSite, StallTracker, StallVerdict};
use super::storage;
use super::watchdog::*;
use crate::state::{AppState, CameraRole};
use night_amplifier_core::camera::{Camera, CameraError, CaptureConfig};
use night_amplifier_core::telemetry::metrics as telemetry_metrics;

/// The sending ends of the two capture channels, with the depth counters that shadow
/// them.
///
/// Grouped rather than passed as four more arguments: a sender and its counter are only
/// correct together — the counter has to be incremented before the send and given back
/// when the send did not happen — so keeping them apart invites exactly the desync
/// `QueueDepth` documents.
pub(crate) struct CaptureChannels {
    pub stacking_tx: mpsc::SyncSender<CapturedFrame>,
    pub storage_tx: mpsc::SyncSender<CapturedFrame>,
    pub stacking_depth: QueueDepth,
    pub storage_depth: QueueDepth,
    pub capacities: channel::PipelineCapacities,
}

/// A capture session's frame numbers: the probe frame and the capture thread claim them,
/// and the orchestrator records where they got to so a resume carries on from there.
#[derive(Debug, Clone)]
pub(crate) struct FrameNumbers(Arc<AtomicU64>);

impl FrameNumbers {
    /// Numbering whose first claimed frame is `first`.
    pub(crate) fn starting_at(first: u64) -> Self {
        Self(Arc::new(AtomicU64::new(first.max(1) - 1)))
    }

    pub(crate) fn claim(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The number the next [`Self::claim`] returns.
    pub(crate) fn peek(&self) -> u64 {
        self.0.load(Ordering::SeqCst) + 1
    }
}

/// Camera capture loop running on a dedicated OS thread.
///
/// Acquires frames from the camera and sends them (as `Arc<Frame>`) to the
/// stacking and storage channels. Uses `try_send` on the stacking channel
/// to avoid blocking when the pipeline can't keep up — frames are dropped
/// and counted. The storage channel uses `try_send` independently.
pub(crate) fn run_capture_task(
    state: Arc<AppState>,
    mut camera: Box<dyn night_amplifier_core::camera::Camera>,
    channels: CaptureChannels,
    numbers: FrameNumbers,
) -> Option<Box<dyn night_amplifier_core::camera::Camera>> {
    let CaptureChannels {
        stacking_tx,
        storage_tx,
        stacking_depth,
        storage_depth,
        capacities,
    } = channels;
    debug!("Capture task started");

    let mut last_status_at = Instant::now()
        .checked_sub(STATUS_POLL_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut camera_ok = true;
    let mut storage_drops = DropLog::default();
    let mut stalls = StallTracker::for_camera(&state, CameraRole::Main, &camera.info().name);

    loop {
        if state.is_cancelled() {
            break;
        }

        // Read settings snapshot for this frame
        let settings = state.settings_for_new_frame();
        let mut capture_config = settings.to_capture_config();

        let camera_info = state
            .camera_in_role(CameraRole::Main)
            .filter(|c| c.info.name == camera.info().name);
        let camera_info = match camera_info {
            Some(info) => info,
            None => {
                warn!("Camera info not found, stopping capture");
                break;
            }
        };

        apply_best_raw_format(&mut capture_config, &camera_info.info, &camera.info().name);
        apply_cooler_support_override(&mut capture_config, &camera_info.info, &camera.info().name);
        apply_sensor_mode_support_override(&mut capture_config, &camera_info.info);

        // Capture a frame (blocking FFI call, bounded so a stuck SDK call
        // can't freeze the pipeline indefinitely — see capture_frame_bounded).
        let watchdog_timeout = capture_watchdog_timeout(&capture_config, &camera_info.info);
        let (new_camera, capture_result) = match capture_frame_bounded(
            camera,
            capture_config,
            numbers.peek(),
            watchdog_timeout,
            &state,
            CameraRole::Main,
        ) {
            CaptureOutcome::Completed(cam, result) => (cam, result),
            // The handle is gone — moved into a detached thread that didn't
            // return in time. Nothing left to close() or return;
            // `stacking_tx`/`storage_tx` still drop normally on the way out.
            CaptureOutcome::TimedOut => return None,
        };
        camera = new_camera;

        let raw_frame = match capture_result {
            Ok(f) => f,
            Err(e) => {
                if let night_amplifier_core::camera::CameraError::Cancelled = e {
                    debug!(
                        "Capture cancelled (likely due to settings update), starting next frame"
                    );
                    camera
                        .cancel_token()
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    continue;
                }

                // Hard disconnect errors invalidate the handle — don't return it. No
                // message to the user here: recovery decides whether they need one.
                if e.is_sdk_disconnected() {
                    error!(error = %e, "Camera disconnected during capture");
                    camera_ok = false;
                    break;
                }

                if let night_amplifier_core::camera::CameraError::ExposureTimeout(budget) = e {
                    if let StallVerdict::Escalate(_) = handle_stall(
                        &mut stalls,
                        StallSite::Main,
                        &state,
                        &camera.info().name,
                        budget,
                    ) {
                        camera_ok = false;
                        break;
                    }
                    state.frame_rejected(settings.stacking, format!("Frame stalled after {budget:?}"));
                    continue;
                }

                warn!(error = %e, "Frame capture failed");
                state.frame_rejected(settings.stacking, format!("Capture failed: {}", e));
                if storage::should_stop_on_errors(&state) {
                    error!("Too many capture failures, stopping");
                    state.send_error("Too many capture failures, stopping".to_string());
                    break;
                }
                continue;
            }
        };

        stalls.frame_delivered();

        if state.is_cancelled() {
            break;
        }

        if camera.info().has_cooler && last_status_at.elapsed() >= STATUS_POLL_INTERVAL {
            camera = match poll_camera_status_bounded(
                camera,
                &state,
                CameraRole::Main,
                settings.target_temp_c,
            ) {
                StatusPollOutcome::Completed(camera) => {
                    last_status_at = Instant::now();
                    camera
                }
                StatusPollOutcome::TimedOut => return None,
            };
        }

        let frame_number = numbers.claim();
        // Counted before either send: the denominator of the drop rate is what the
        // camera produced, not what the pipeline managed to accept.
        state.frame_delivered();
        let arc_frame = Arc::new(raw_frame);

        // Send to stacking channel (non-blocking — drop frame if full)
        let stacking_msg = CapturedFrame {
            frame: Arc::clone(&arc_frame),
            frame_number,
            settings: settings.clone(),
            camera_info: camera_info.clone(),
        };
        // Counted before the send and given back on failure — see `QueueDepth`.
        stacking_depth.sent();
        if stacking_tx.try_send(stacking_msg).is_err() {
            stacking_depth.taken();
            state.frame_dropped();
            debug!(frame_number, "Frame dropped: stacking pipeline busy");
        }
        telemetry_metrics::record_pipeline_queue_depth(
            "capture_to_stacking",
            stacking_depth.pending() as u64,
            capacities.stacking as u64,
        );

        // Send to storage channel (non-blocking — independent dropping)
        if settings.saves_raw_frames() && state.disk_writer.is_enabled() {
            let storage_msg = CapturedFrame {
                frame: arc_frame,
                frame_number,
                settings,
                camera_info,
            };
            storage_depth.sent();
            if storage_tx.try_send(storage_msg).is_err() {
                storage_depth.taken();
                if let Some(dropped) = storage_drops.record() {
                    warn!(frame_number, dropped, "Raw frames dropped: storage pipeline busy");
                }
            }
            telemetry_metrics::record_pipeline_queue_depth(
                "capture_to_storage",
                storage_depth.pending() as u64,
                capacities.storage as u64,
            );
        }
    }

    if let Some(dropped) = storage_drops.flush() {
        warn!(
            dropped,
            "Raw frames dropped since the last report: storage pipeline busy"
        );
    }

    debug!("Capture task ended");
    // stacking_tx and storage_tx are dropped here, signaling downstream to exit.
    // Return the handle so the orchestrator can hand it back to the camera
    // session (or drop it on a hard disconnect).
    if camera_ok {
        return Some(camera);
    }
    release_faulted_handle(camera, &state, CameraRole::Main);
    None
}

/// The first frame of a session, through the same stall ladder as every later one: a
/// stall restarts the stream in place, and only a run of `STALL_ESCALATION` is handed
/// back, as the fault it is. After a (re)open this is the frame most likely to be slow —
/// config reapplied, stream started cold — and ending the session over one stall
/// discarded the capture recovery had just saved.
pub(super) fn capture_probe_frame(
    mut camera: Box<dyn Camera>,
    config: CaptureConfig,
    frame_number: u64,
    watchdog_timeout: Duration,
    state: &Arc<AppState>,
) -> CaptureOutcome {
    let mut stalls = StallTracker::for_camera(state, CameraRole::Main, &camera.info().name);
    loop {
        let outcome = capture_frame_bounded(
            camera,
            config.clone(),
            frame_number,
            watchdog_timeout,
            state,
            CameraRole::Main,
        );
        let (returned, budget) = match outcome {
            CaptureOutcome::Completed(returned, Err(CameraError::ExposureTimeout(budget))) => (returned, budget),
            other => {
                // A restart that brought the first frame back is evidence restarts work.
                if matches!(other, CaptureOutcome::Completed(_, Ok(_))) {
                    stalls.frame_delivered();
                }
                return other;
            }
        };
        let verdict =
            handle_stall(&mut stalls, StallSite::FirstFrame, state, &returned.info().name, budget);
        if let StallVerdict::Escalate(_) = verdict {
            return CaptureOutcome::Completed(returned, Err(CameraError::ExposureTimeout(budget)));
        }
        if state.is_cancelled() {
            return CaptureOutcome::Completed(returned, Err(CameraError::Cancelled));
        }
        camera = returned;
    }
}
