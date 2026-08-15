//! Decoupled asynchronous capture pipeline
//!
//! The capture loop is decomposed into four independent tasks connected by
//! bounded MPSC channels:
//!
//! - **CaptureTask** (dedicated thread) — acquires frames from the camera
//! - **StorageTask** (dedicated thread) — saves raw frames to disk
//! - **StackingTask** (dedicated thread) — registration + accumulation
//! - **RenderTask** (dedicated thread) — preview rendering + encoding
//!
//! `Arc<Frame>` provides zero-copy frame sharing between channels.
//! Channel capacities are calculated from a 2 GB memory budget divided by
//! the actual frame size.
//!
//! Each spawned OS thread receives a `tokio::runtime::Handle` captured from
//! the async orchestrator, so it can call `handle.block_on()` for async
//! state access and `handle.spawn()` for fire-and-forget async work.

pub mod channel;
mod context;
mod pipeline;
mod render_task;
mod solving;
mod stacking_task;
mod storage;

use render_task::run_render_task;
use stacking_task::run_stacking_task;

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use super::encoding::encode_rgb8_lz4;
use super::events::ServerEvent;
use super::state::{AppState, CaptureState, StackingType};

use crate::frame::Frame;
use crate::stacking::CometContext;
use crate::telemetry::metrics as telemetry_metrics;
pub use context::{PlanetaryStackingContext, StackingContext};

use channel::{max_queue_capacity, CapturedFrame, StackedFrame};

/// Cadence for polling cooled-camera status from the capture thread.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Hard bound on a single `camera.status()` call (see `poll_camera_status_bounded`).
/// If it doesn't return within this, the handle is abandoned rather than left
/// to block frame delivery indefinitely — no vendor SDK call other than the
/// image-data read exposes a timeout of its own.
const STATUS_POLL_TIMEOUT: Duration = Duration::from_secs(3);

/// Consecutive watchdog timeouts against the same camera before escalating
/// from an ordinary disconnect to a distinct "persistently unresponsive"
/// signal (`ServerEvent::CameraPersistentlyUnresponsive`) — see
/// `AppState.consecutive_watchdog_timeouts`.
const PERSISTENT_FAULT_THRESHOLD: u32 = 3;

/// Added on top of a capture attempt's own `config.timeout + exposure` budget
/// to get `capture_frame_bounded`'s watchdog timeout — gives the backend's own
/// internal timeout-and-cleanup the first chance to fire before this external
/// last resort does. See `capture_frame_bounded`.
const CAPTURE_WATCHDOG_SLACK: Duration = Duration::from_secs(10);

/// Floor for `capture_watchdog_margin` at (near-)zero exposure — e.g. a 10ms
/// live-view frame. Set comfortably above the normal jitter ceiling observed
/// for healthy captures (up to ~1.6s) so it doesn't false-positive on
/// ordinary variance, while still catching a multi-second stall quickly
/// instead of only after the full long-exposure budget.
const CAPTURE_WATCHDOG_MIN_MARGIN: Duration = Duration::from_secs(5);

/// Exposure length at which `capture_watchdog_margin` finishes ramping up to
/// the full `config.timeout + CAPTURE_WATCHDOG_SLACK` ceiling. Set above
/// typical EAA live-stacking sub-exposure lengths (commonly single-digit to
/// ~20s), so most live-stacking sessions get meaningfully tighter, scaled
/// protection instead of the flat long-exposure budget — while exposures at
/// or beyond this (long deep-sky subs, the actual reason for a generous
/// ceiling) get full trust.
const CAPTURE_WATCHDOG_RAMP_EXPOSURE: Duration = Duration::from_secs(30);

/// Watchdog margin added on top of the exposure itself to get
/// `capture_frame_bounded`'s timeout — scaled down for short exposures so a
/// stall gets caught in seconds during live view instead of only after the
/// full ~130s long-exposure budget, while long deep-sky exposures keep their
/// existing tolerance unchanged. Pure/deterministic so it's directly
/// unit-testable without threads or real time.
///
/// Ramps linearly from `CAPTURE_WATCHDOG_MIN_MARGIN` (at zero exposure) to
/// `config_timeout + CAPTURE_WATCHDOG_SLACK` (at or beyond
/// `CAPTURE_WATCHDOG_RAMP_EXPOSURE`), where it holds flat.
fn capture_watchdog_margin(exposure_us: u64, config_timeout: Duration) -> Duration {
    // `.max(...)` guards against a degenerate/misconfigured tiny
    // `config_timeout` ever producing a ceiling below the floor.
    let max_margin = (config_timeout + CAPTURE_WATCHDOG_SLACK).max(CAPTURE_WATCHDOG_MIN_MARGIN);
    let exposure = Duration::from_micros(exposure_us);
    if exposure >= CAPTURE_WATCHDOG_RAMP_EXPOSURE {
        return max_margin;
    }
    let fraction = exposure.as_secs_f64() / CAPTURE_WATCHDOG_RAMP_EXPOSURE.as_secs_f64();
    let min = CAPTURE_WATCHDOG_MIN_MARGIN.as_secs_f64();
    let max = max_margin.as_secs_f64();
    Duration::from_secs_f64(min + (max - min) * fraction)
}

/// Clear a camera's consecutive-watchdog-timeout streak. Called whenever any
/// bounded SDK call — status or capture — returns within its budget, since
/// that proves the camera is currently responding regardless of which
/// specific watchdog was in play.
fn clear_watchdog_timeout_streak(state: &Arc<AppState>, camera_name: &str) {
    state
        .consecutive_watchdog_timeouts
        .lock()
        .expect("consecutive_watchdog_timeouts mutex poisoned")
        .remove(camera_name);
}

/// Record a watchdog timeout for a camera — a status-poll timeout and a
/// capture timeout are equally strong evidence of the same underlying
/// hardware/USB fault, so both feed this one counter. Once
/// `PERSISTENT_FAULT_THRESHOLD` consecutive timeouts have accumulated,
/// escalates with a distinct event on top of whatever per-incident error the
/// caller already sends.
fn record_watchdog_timeout(state: &Arc<AppState>, camera_name: &str) {
    let consecutive_timeouts = {
        let mut counts = state
            .consecutive_watchdog_timeouts
            .lock()
            .expect("consecutive_watchdog_timeouts mutex poisoned");
        let count = counts.entry(camera_name.to_string()).or_insert(0);
        *count += 1;
        *count
    };
    if consecutive_timeouts >= PERSISTENT_FAULT_THRESHOLD {
        error!(
            camera_name = %camera_name,
            consecutive_timeouts,
            "Camera appears persistently unresponsive across repeated reconnects"
        );
        let _ = state.events.send(ServerEvent::camera_persistently_unresponsive(
            camera_name.to_string(),
            consecutive_timeouts,
        ));
    }
}

/// Override the capture format with the best raw format advertised by the
/// camera (`Raw16` preferred, `Raw8` as fallback). Leaves the config untouched
/// if neither is advertised, letting the provider surface a clear SDK error.
fn apply_best_raw_format(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
    camera_name: &str,
) {
    if let Some(format) = crate::camera::ImageFormat::best_raw_format(&info.supported_formats) {
        if config.format != format {
            debug!(
                camera = %camera_name,
                selected = ?format,
                requested = ?config.format,
                supported = ?info.supported_formats,
                "Adjusted capture format to best available raw format"
            );
            config.format = format;
        }
    } else {
        warn!(
            camera = %camera_name,
            supported = ?info.supported_formats,
            "Camera advertises neither Raw16 nor Raw8 — capture may fail"
        );
    }
}

/// Drop cooler-related fields when the camera has no cooler. Saved settings
/// may carry `cooler_enabled = true` from a previous cooled camera; without
/// this override `CaptureConfig::validate` would reject the config and
/// capture would fail before the first frame.
fn apply_cooler_support_override(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
    camera_name: &str,
) {
    if info.has_cooler {
        return;
    }
    if config.cooler_enabled || config.target_temp_c.is_some() {
        debug!(
            camera = %camera_name,
            "Camera has no cooler; clearing cooler_enabled / target_temp_c from capture config"
        );
        config.cooler_enabled = false;
        config.target_temp_c = None;
    }
}

/// Drop `sensor_mode` when the camera doesn't advertise sensor modes.
/// `CaptureSettings::to_capture_config` fills `sensor_mode` unconditionally
/// from the explicit override or from `stacking_type.desired_sensor_mode()`
/// — neither is aware of the active camera's capabilities. Without this
/// override, `CaptureConfig::validate` rejects the request with
/// `ParameterNotSupported("sensor_mode")` for any camera that reports an
/// empty `sensor_modes` list (e.g. Player One uncooled planetary models).
fn apply_sensor_mode_support_override(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
    camera_name: &str,
) {
    if !info.sensor_modes.is_empty() {
        return;
    }
    if config.sensor_mode.is_some() {
        debug!(
            camera = %camera_name,
            "Camera advertises no sensor modes; clearing sensor_mode from capture config"
        );
        config.sensor_mode = None;
    }
}

/// The main capture orchestrator.
///
/// Takes the long-lived camera handle from `AppState` (opened at connect
/// time), creates bounded channels, spawns four independent worker threads,
/// and awaits their completion. On shutdown, returns the handle to
/// `AppState` so the monitor thread can resume — unless the capture task
/// lost the handle due to a hard error, in which case the lifecycle layer
/// finalizes a disconnect.
pub async fn run_capture_loop(state: Arc<AppState>, camera_id: String) {
    use crate::server::camera_session::lifecycle;

    // Transition to capturing state
    state.set_capture_state(CaptureState::Capturing).await;

    debug!(camera_id = %camera_id, "Capture pipeline starting");

    // Capture the tokio runtime handle — this will be passed to all spawned
    // OS threads so they can call handle.block_on() and handle.spawn().
    let rt_handle = tokio::runtime::Handle::current();

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

    // Take the handle from AppState (held by camera_session since connect).
    // This cancels any in-progress warmup and flips the phase to Capturing.
    let camera_name = camera_info.info.name.clone();
    let mut camera = match lifecycle::take_for_capture(&state, &camera_name).await {
        Ok(cam) => {
            debug!(
                camera_id = %camera_id,
                provider = %camera_info.provider,
                "Camera handle taken for capture"
            );
            cam
        }
        Err(e) => {
            error!(camera_id = %camera_id, error = %e, "Failed to take camera handle for capture");
            state.send_error(format!("Failed to take camera handle: {}", e));
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    // Register active camera cancel token in state
    state.set_active_camera_token(camera.cancel_token()).await;

    // Capture a probe frame to determine dimensions and channel capacities
    let settings = state.settings.read().await.clone();
    let mut capture_config = settings.to_capture_config();
    apply_best_raw_format(&mut capture_config, &camera_info.info, &camera_name);
    apply_cooler_support_override(&mut capture_config, &camera_info.info, &camera_name);
    apply_sensor_mode_support_override(&mut capture_config, &camera_info.info, &camera_name);
    let probe_frame = match camera.capture(&capture_config) {
        Ok(f) => f,
        Err(e) => {
            error!(error = %e, "Failed to capture probe frame for pipeline setup");
            state.send_error(format!("Failed to capture initial frame: {}", e));
            state.clear_active_camera_token().await;
            lifecycle::return_from_capture(&state, &camera_name, Some(camera)).await;
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    let frame_memory = probe_frame.memory_size();
    let channel_capacity = max_queue_capacity(frame_memory);
    info!(
        frame_memory_bytes = frame_memory,
        channel_capacity = channel_capacity,
        width = probe_frame.width(),
        height = probe_frame.height(),
        channels = probe_frame.channels(),
        "Pipeline channel capacity calculated"
    );

    // Create bounded channels
    let (stacking_tx, stacking_rx) = mpsc::sync_channel::<CapturedFrame>(channel_capacity);
    let (storage_tx, storage_rx) = mpsc::sync_channel::<CapturedFrame>(channel_capacity);
    let (render_tx, render_rx) = mpsc::sync_channel::<StackedFrame>(channel_capacity);

    // Send the probe frame as the first frame through the pipeline
    let first_frame = Arc::new(probe_frame);
    let first_msg = CapturedFrame {
        frame: Arc::clone(&first_frame),
        frame_number: 1,
        settings: settings.clone(),
        camera_info: camera_info.clone(),
    };
    let first_msg_storage = CapturedFrame {
        frame: first_frame,
        frame_number: 1,
        settings: settings.clone(),
        camera_info: camera_info.clone(),
    };
    let _ = stacking_tx.send(first_msg);
    let _ = storage_tx.send(first_msg_storage);

    // Spawn worker threads — each gets a clone of the tokio Handle
    let state_capture = Arc::clone(&state);
    let state_stacking = Arc::clone(&state);
    let state_render = Arc::clone(&state);
    let state_storage = Arc::clone(&state);

    let rt_capture = rt_handle.clone();
    let rt_stacking = rt_handle.clone();
    let rt_render = rt_handle.clone();
    let rt_storage = rt_handle.clone();

    let capture_handle = std::thread::Builder::new()
        .name("capture-task".into())
        .spawn(move || run_capture_task(state_capture, camera, stacking_tx, storage_tx, rt_capture))
        .expect("Failed to spawn capture thread");

    let stacking_handle = std::thread::Builder::new()
        .name("stacking-task".into())
        .spawn(move || {
            run_stacking_task(state_stacking, stacking_rx, render_tx, rt_stacking);
        })
        .expect("Failed to spawn stacking thread");

    let render_handle = std::thread::Builder::new()
        .name("render-task".into())
        .spawn(move || {
            run_render_task(state_render, render_rx, rt_render);
        })
        .expect("Failed to spawn render thread");

    let storage_handle = std::thread::Builder::new()
        .name("storage-task".into())
        .spawn(move || {
            storage::run_storage_task(state_storage, storage_rx, rt_storage);
        })
        .expect("Failed to spawn storage thread");

    // Wait for all threads to complete (blocking join wrapped in spawn_blocking
    // to avoid blocking the tokio runtime). The capture task returns the
    // handle so it can be returned to the session; downstream threads
    // produce no output.
    let returned_camera = tokio::task::spawn_blocking(move || {
        let cam = match capture_handle.join() {
            Ok(cam) => cam,
            Err(e) => {
                error!("Capture thread panicked: {:?}", e);
                None
            }
        };
        // Once capture is done (senders dropped), downstream threads will drain and exit
        if let Err(e) = storage_handle.join() {
            error!("Storage thread panicked: {:?}", e);
        }
        if let Err(e) = stacking_handle.join() {
            error!("Stacking thread panicked: {:?}", e);
        }
        if let Err(e) = render_handle.join() {
            error!("Render thread panicked: {:?}", e);
        }
        cam
    })
    .await
    .unwrap_or(None);

    // End capture session
    state.disk_writer.end_session();

    info!(camera_id = %camera_id, "Capture pipeline ended");

    // Return the camera handle to the session (or finalize disconnect if lost).
    state.clear_active_camera_token().await;
    lifecycle::return_from_capture(&state, &camera_name, returned_camera).await;
    state.set_capture_state(CaptureState::Idle).await;
}

// =============================================================================
// CaptureTask
// =============================================================================

/// Camera capture loop running on a dedicated OS thread.
///
/// Acquires frames from the camera and sends them (as `Arc<Frame>`) to the
/// stacking and storage channels. Uses `try_send` on the stacking channel
/// to avoid blocking when the pipeline can't keep up — frames are dropped
/// and counted. The storage channel uses `try_send` independently.
fn run_capture_task(
    state: Arc<AppState>,
    mut camera: Box<dyn crate::camera::Camera>,
    stacking_tx: mpsc::SyncSender<CapturedFrame>,
    storage_tx: mpsc::SyncSender<CapturedFrame>,
    rt: tokio::runtime::Handle,
) -> Option<Box<dyn crate::camera::Camera>> {
    debug!("Capture task started");

    // Frame numbering continues from 1 (probe frame was #1)
    let mut frame_number: u64 = 1;
    let mut last_status_at = Instant::now()
        .checked_sub(STATUS_POLL_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut camera_ok = true;

    loop {
        if state.is_cancelled() {
            break;
        }

        // Read settings snapshot for this frame
        let settings = rt.block_on(state.settings.read()).clone();
        let mut capture_config = settings.to_capture_config();

        // Get camera info
        let camera_info = {
            let cameras = rt.block_on(state.cameras.read());
            cameras
                .values()
                .find(|c| c.info.name == camera.info().name)
                .cloned()
        };
        let camera_info = match camera_info {
            Some(info) => info,
            None => {
                warn!("Camera info not found, stopping capture");
                break;
            }
        };

        apply_best_raw_format(&mut capture_config, &camera_info.info, &camera.info().name);
        apply_cooler_support_override(&mut capture_config, &camera_info.info, &camera.info().name);
        apply_sensor_mode_support_override(
            &mut capture_config,
            &camera_info.info,
            &camera.info().name,
        );

        // Capture a frame (blocking FFI call, bounded so a stuck SDK call
        // can't freeze the pipeline indefinitely — see capture_frame_bounded).
        let watchdog_timeout = Duration::from_micros(capture_config.exposure_us)
            + capture_watchdog_margin(capture_config.exposure_us, capture_config.timeout);
        let (new_camera, capture_result) = match capture_frame_bounded(
            camera,
            capture_config,
            frame_number + 1,
            watchdog_timeout,
            &state,
        ) {
            CaptureOutcome::Completed(cam, result) => (cam, result),
            // The handle is gone — moved into a detached thread that didn't
            // return in time. Nothing left to close() or return;
            // `stacking_tx`/`storage_tx` still drop normally on the way out.
            CaptureOutcome::TimedOut => return None,
        };
        camera = new_camera;

        let frame = match capture_result {
            Ok(f) => f,
            Err(e) => {
                if let crate::camera::CameraError::Cancelled = e {
                    debug!(
                        "Capture cancelled (likely due to settings update), starting next frame"
                    );
                    camera
                        .cancel_token()
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    continue;
                }

                // Hard disconnect errors invalidate the handle — don't return it.
                if let crate::camera::CameraError::Disconnected = e {
                    error!(error = %e, "Camera disconnected during capture");
                    state.send_error(format!("Camera disconnected: {}", e));
                    camera_ok = false;
                    break;
                }

                warn!(error = %e, "Frame capture failed");
                rt.block_on(state.frame_rejected(format!("Capture failed: {}", e)));
                if rt.block_on(storage::should_stop_on_errors(&state)) {
                    error!("Too many capture failures, stopping");
                    state.send_error("Too many capture failures, stopping".to_string());
                    break;
                }
                continue;
            }
        };

        if state.is_cancelled() {
            break;
        }

        if camera.info().has_cooler && last_status_at.elapsed() >= STATUS_POLL_INTERVAL {
            camera = match poll_camera_status_bounded(camera, &state, settings.target_temp_c, &rt) {
                StatusPollOutcome::Completed(camera) => {
                    last_status_at = Instant::now();
                    camera
                }
                // The handle is gone — moved into a detached thread that
                // didn't return in time. Nothing left to close() or return;
                // `stacking_tx`/`storage_tx` still drop normally on the way out.
                StatusPollOutcome::TimedOut => return None,
            };
        }

        frame_number += 1;
        let arc_frame = Arc::new(frame);

        // Send to stacking channel (non-blocking — drop frame if full)
        let stacking_msg = CapturedFrame {
            frame: Arc::clone(&arc_frame),
            frame_number,
            settings: settings.clone(),
            camera_info: camera_info.clone(),
        };
        if stacking_tx.try_send(stacking_msg).is_err() {
            state.frame_dropped();
            debug!(frame_number, "Frame dropped: stacking pipeline busy");
        }

        // Send to storage channel (non-blocking — independent dropping)
        let is_stacking_mode = settings.stacking && !settings.wanderer_mode;
        if settings.save_raw_frames && is_stacking_mode && state.disk_writer.is_enabled() {
            let storage_msg = CapturedFrame {
                frame: arc_frame,
                frame_number,
                settings,
                camera_info,
            };
            if storage_tx.try_send(storage_msg).is_err() {
                warn!(frame_number, "Raw frame dropped: storage pipeline busy");
            }
        }
    }

    debug!("Capture task ended");
    // stacking_tx and storage_tx are dropped here, signaling downstream to exit.
    // Return the handle so the orchestrator can hand it back to the camera
    // session (or drop it on a hard disconnect).
    if camera_ok {
        Some(camera)
    } else {
        let _ = camera.close();
        None
    }
}

/// Outcome of a bounded `camera.status()` call — see `poll_camera_status_bounded`.
enum StatusPollOutcome {
    /// The call returned in time. The camera handle is returned so the
    /// capture loop can keep using it.
    Completed(Box<dyn crate::camera::Camera>),
    /// The call did not return within `STATUS_POLL_TIMEOUT`. The camera
    /// handle is gone for good — see the function doc for why.
    TimedOut,
}

/// Read the camera's live status, cache it, and broadcast a `CameraStatusUpdated`
/// event — bounded by `STATUS_POLL_TIMEOUT`.
///
/// The camera handle is owned by exactly one thread at a time — never touched
/// concurrently, which is what avoids contention with vendor SDKs that require
/// a single handle per device. Historically that one thread was always the
/// capture thread itself; now it's temporarily a detached watchdog thread
/// instead, for exactly the duration of this one call, so a stuck read can't
/// block frame delivery. No vendor SDK call other than the image-data read
/// exposes a timeout of its own, so without this bound a USB-level hiccup
/// inside `camera.status()` could block the entire live view silently, for as
/// long as the underlying call took to return (observed: several seconds to
/// indefinitely).
///
/// The call runs on that detached helper thread while this function waits up
/// to `STATUS_POLL_TIMEOUT` on a channel. If it returns in time, the handle
/// comes back and capture continues normally. If not, the handle is abandoned
/// for good: every backend's underlying type implements `Drop`, so the SDK
/// resource is still released whenever that thread eventually unwinds — there
/// is no way to forcibly cancel a stuck synchronous FFI call in Rust, so
/// "abandon and disconnect" is the safe alternative to "wait forever." The
/// caller must treat `TimedOut` the same as a real disconnect.
///
/// Also tracks consecutive timeouts per camera (`AppState.consecutive_watchdog_timeouts`)
/// to distinguish an isolated USB hiccup from a persistent hardware fault — see
/// `PERSISTENT_FAULT_THRESHOLD` and `ServerEvent::CameraPersistentlyUnresponsive`.
fn poll_camera_status_bounded(
    camera: Box<dyn crate::camera::Camera>,
    state: &Arc<AppState>,
    target_temp_c: Option<f64>,
    rt: &tokio::runtime::Handle,
) -> StatusPollOutcome {
    let (tx, rx) = mpsc::channel();
    let camera_name = camera.info().name.clone();
    // `std::thread::spawn` does not carry over the calling thread's tracing
    // context, so `camera_status_poll` would otherwise show up as a root span
    // with no relation to whatever surrounds this call — capture and re-enter
    // it explicitly.
    let parent_span = tracing::Span::current();

    if let Err(e) = std::thread::Builder::new()
        .name("status-poll-watchdog".into())
        .spawn(move || {
            let _parent_guard = parent_span.enter();
            let _span = tracing::info_span!("camera_status_poll").entered();
            let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::StatusPoll);
            let start = Instant::now();
            let result = camera.status();
            let _ = tx.send((camera, result, start.elapsed()));
        })
    {
        // `camera` was moved into the closure above and is gone with it — an
        // OS-level thread-spawn failure is rare enough that treating it the
        // same as a timeout (abandon the handle, disconnect) is simplest.
        error!(camera_name = %camera_name, error = %e, "Failed to spawn status-poll watchdog thread");
        return StatusPollOutcome::TimedOut;
    }

    match rx.recv_timeout(STATUS_POLL_TIMEOUT) {
        Ok((camera, result, elapsed)) => {
            // A response arrived within budget — whatever it says, the camera
            // is currently communicating, so any prior timeout streak no
            // longer indicates an active fault.
            clear_watchdog_timeout_streak(state, &camera_name);

            if elapsed > Duration::from_millis(500) {
                warn!(
                    camera_name = %camera_name,
                    elapsed_ms = elapsed.as_millis(),
                    "camera.status() was slow"
                );
            }
            match result {
                Ok(status) => {
                    rt.block_on(state.update_camera_status(&camera_name, status, target_temp_c));
                }
                Err(e) => debug!(error = %e, "Failed to read camera status"),
            }
            StatusPollOutcome::Completed(camera)
        }
        Err(_) => {
            error!(
                camera_name = %camera_name,
                timeout = ?STATUS_POLL_TIMEOUT,
                "camera.status() did not return in time — abandoning camera handle (suspected USB stall)"
            );
            record_watchdog_timeout(state, &camera_name);
            state.send_error(format!(
                "Camera '{}' stopped responding (status read timed out) — disconnecting",
                camera_name
            ));
            StatusPollOutcome::TimedOut
        }
    }
}

/// Outcome of a bounded `camera.capture()` call — see `capture_frame_bounded`.
enum CaptureOutcome {
    /// The call returned in time — successfully or with an error either way,
    /// so the caller can still see what `capture()` reported while getting
    /// the handle back to keep using.
    Completed(
        Box<dyn crate::camera::Camera>,
        crate::camera::CameraResult<Frame>,
    ),
    /// The call did not return within `watchdog_timeout`. The camera handle
    /// is gone for good — see `poll_camera_status_bounded`'s doc for why this
    /// is safe (every backend's handle type implements `Drop`) and why there
    /// is no alternative for a stuck synchronous FFI call.
    TimedOut,
}

/// Run `camera.capture(&config)` bounded by `watchdog_timeout`, the same way
/// `poll_camera_status_bounded` bounds `camera.status()`.
///
/// Every backend's own internal capture loop already computes a "total
/// budget" of `config.timeout + exposure duration` and self-enforces it
/// *between* the individual blocking SDK calls that make up one capture
/// attempt (confirmed identical across PlayerOne/ZWO/SVBony/QHY/ToupTek). That
/// self-check cannot fire if one of those individual calls itself hangs — a
/// USB-level stall inside, say, PlayerOne's `is_image_ready()` poll blocks the
/// whole loop indefinitely, exactly as observed in the field: a ~3 minute
/// freeze, unresponsive to a Stop click partway through, before the SDK
/// finally reported `POA_ERROR_INVALID_ID`. For a long exposure,
/// `watchdog_timeout` is set to slightly more than that same internal budget,
/// so the backend's own graceful timeout-and-cleanup gets the first chance to
/// run and this watchdog only acts as the last resort when even that gets
/// bypassed. For a short exposure (live view, planetary) that full budget is
/// far too tolerant — a multi-second stall is already abnormal long before
/// ~130s — so the caller scales `watchdog_timeout` down via
/// `capture_watchdog_margin` instead of always using the full budget.
///
/// Caveat: if cancellation is requested while `capture()` is already stuck,
/// and this watchdog fires before the backend's own cancel-flag check would
/// have noticed, the session ends as a disconnect rather than a clean stop.
/// There is no way to do better without the vendor SDK supporting real
/// cancellation of an in-flight call — still strictly better than hanging
/// indefinitely.
fn capture_frame_bounded(
    camera: Box<dyn crate::camera::Camera>,
    config: crate::camera::CaptureConfig,
    frame_number: u64,
    watchdog_timeout: Duration,
    state: &Arc<AppState>,
) -> CaptureOutcome {
    let (tx, rx) = mpsc::channel();
    let camera_name = camera.info().name.clone();
    let parent_span = tracing::Span::current();

    if let Err(e) = std::thread::Builder::new()
        .name("capture-watchdog".into())
        .spawn(move || {
            let mut camera = camera;
            let _parent_guard = parent_span.enter();
            let _span = tracing::info_span!(
                "camera_capture",
                frame_number,
                exposure_us = config.exposure_us,
                gain = config.gain,
                bin = config.bin,
            )
            .entered();
            let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::Capture);
            let result = camera.capture(&config);
            let _ = tx.send((camera, result));
        })
    {
        // `camera` was moved into the closure above and is gone with it —
        // same reasoning as poll_camera_status_bounded's spawn-failure branch.
        error!(camera_name = %camera_name, error = %e, "Failed to spawn capture watchdog thread");
        return CaptureOutcome::TimedOut;
    }

    match rx.recv_timeout(watchdog_timeout) {
        Ok((camera, result)) => {
            // A response arrived within budget — regardless of whether
            // `result` itself is Ok or Err, the camera is currently
            // communicating, so any prior timeout streak no longer applies.
            clear_watchdog_timeout_streak(state, &camera_name);
            CaptureOutcome::Completed(camera, result)
        }
        Err(_) => {
            error!(
                camera_name = %camera_name,
                timeout = ?watchdog_timeout,
                "camera.capture() did not return in time — abandoning camera handle (suspected USB stall)"
            );
            record_watchdog_timeout(state, &camera_name);
            state.send_error(format!(
                "Camera '{}' stopped responding (capture timed out) — disconnecting",
                camera_name
            ));
            CaptureOutcome::TimedOut
        }
    }
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use crate::server::state::AppState;
    use std::sync::atomic::AtomicBool;

    /// Minimal `Camera` double whose `status()`/`capture()` can each
    /// independently simulate a stuck SDK call.
    struct TestCamera {
        info: crate::camera::CameraInfo,
        status_delay: Duration,
        capture_delay: Duration,
        cancel_flag: Arc<AtomicBool>,
    }

    impl TestCamera {
        fn new(status_delay: Duration) -> Self {
            Self::with_delays(status_delay, Duration::ZERO)
        }

        fn with_capture_delay(capture_delay: Duration) -> Self {
            Self::with_delays(Duration::ZERO, capture_delay)
        }

        fn with_delays(status_delay: Duration, capture_delay: Duration) -> Self {
            Self {
                info: crate::camera::CameraInfo {
                    name: "Test Cam".to_string(),
                    ..Default::default()
                },
                status_delay,
                capture_delay,
                cancel_flag: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl crate::camera::Camera for TestCamera {
        fn info(&self) -> &crate::camera::CameraInfo {
            &self.info
        }
        fn gain_presets(&self) -> crate::camera::CameraResult<crate::camera::GainPresets> {
            Ok(crate::camera::GainPresets::default())
        }
        fn status(&self) -> crate::camera::CameraResult<crate::camera::CameraStatus> {
            if !self.status_delay.is_zero() {
                std::thread::sleep(self.status_delay);
            }
            Ok(crate::camera::CameraStatus::default())
        }
        fn set_target_temperature(&mut self, _temp_c: f64) -> crate::camera::CameraResult<()> {
            Ok(())
        }
        fn set_cooler(&mut self, _enabled: bool) -> crate::camera::CameraResult<()> {
            Ok(())
        }
        fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> crate::camera::CameraResult<()> {
            Ok(())
        }
        fn capture(
            &mut self,
            _config: &crate::camera::CaptureConfig,
        ) -> crate::camera::CameraResult<crate::frame::Frame> {
            if !self.capture_delay.is_zero() {
                std::thread::sleep(self.capture_delay);
            }
            crate::frame::Frame::zeros(4, 4, 1)
                .map_err(|e| crate::camera::CameraError::ImageReadFailed(e.to_string()))
        }
        fn cancel(&self) {
            self.cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        fn cancel_token(&self) -> Arc<AtomicBool> {
            Arc::clone(&self.cancel_flag)
        }
        fn close(&mut self) -> crate::camera::CameraResult<()> {
            Ok(())
        }
        fn provider_name(&self) -> &'static str {
            "Test"
        }
    }

    #[tokio::test]
    async fn poll_camera_status_bounded_completes_when_fast() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let camera: Box<dyn crate::camera::Camera> = Box::new(TestCamera::new(Duration::ZERO));
        let rt = tokio::runtime::Handle::current();

        let outcome = tokio::task::spawn_blocking({
            let state = Arc::clone(&state);
            move || poll_camera_status_bounded(camera, &state, None, &rt)
        })
        .await
        .unwrap();

        assert!(
            matches!(outcome, StatusPollOutcome::Completed(_)),
            "expected Completed for a fast status() call"
        );
    }

    /// The whole point of the watchdog: a stuck `status()` call must not block
    /// the caller past `STATUS_POLL_TIMEOUT`, even though the underlying call
    /// (and its thread) keeps running in the background afterward.
    #[tokio::test]
    async fn poll_camera_status_bounded_times_out_on_stuck_call() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let camera: Box<dyn crate::camera::Camera> =
            Box::new(TestCamera::new(STATUS_POLL_TIMEOUT + Duration::from_secs(5)));
        let rt = tokio::runtime::Handle::current();

        let start = Instant::now();
        let outcome = tokio::task::spawn_blocking({
            let state = Arc::clone(&state);
            move || poll_camera_status_bounded(camera, &state, None, &rt)
        })
        .await
        .unwrap();
        let elapsed = start.elapsed();

        assert!(matches!(outcome, StatusPollOutcome::TimedOut));
        assert!(
            elapsed < STATUS_POLL_TIMEOUT + Duration::from_secs(2),
            "poll_camera_status_bounded should return around STATUS_POLL_TIMEOUT, \
             not wait for the stuck call; took {:?}",
            elapsed
        );
    }

    /// Run one bounded status poll against a camera that never responds in
    /// time, returning once the call has been dispatched (it will show up as
    /// a `TimedOut` outcome, same as the dedicated timeout test above).
    async fn stuck_poll(state: &Arc<AppState>, rt: &tokio::runtime::Handle) {
        let camera: Box<dyn crate::camera::Camera> =
            Box::new(TestCamera::new(STATUS_POLL_TIMEOUT + Duration::from_secs(5)));
        let state = Arc::clone(state);
        let rt = rt.clone();
        tokio::task::spawn_blocking(move || poll_camera_status_bounded(camera, &state, None, &rt))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn poll_camera_status_bounded_escalates_after_persistent_timeouts() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let rt = tokio::runtime::Handle::current();

        for i in 1..=PERSISTENT_FAULT_THRESHOLD {
            let mut subscriber = state.subscribe_events();
            stuck_poll(&state, &rt).await;

            let mut saw_persistent = false;
            while let Ok(event) = subscriber.try_recv() {
                if let ServerEvent::CameraPersistentlyUnresponsive {
                    consecutive_timeouts,
                    ..
                } = event
                {
                    assert_eq!(
                        consecutive_timeouts, i,
                        "escalation event should report the current streak length"
                    );
                    saw_persistent = true;
                }
            }
            assert_eq!(
                saw_persistent,
                i >= PERSISTENT_FAULT_THRESHOLD,
                "persistent-unresponsive event should only fire from the threshold-th \
                 consecutive timeout onward (iteration {i})"
            );
        }
    }

    #[tokio::test]
    async fn poll_camera_status_bounded_resets_streak_on_success() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let rt = tokio::runtime::Handle::current();

        // One timeout short of the threshold.
        for _ in 0..(PERSISTENT_FAULT_THRESHOLD - 1) {
            stuck_poll(&state, &rt).await;
        }

        // A fast, successful poll should clear the streak.
        let fast_camera: Box<dyn crate::camera::Camera> = Box::new(TestCamera::new(Duration::ZERO));
        let outcome = {
            let state = Arc::clone(&state);
            let rt = rt.clone();
            tokio::task::spawn_blocking(move || {
                poll_camera_status_bounded(fast_camera, &state, None, &rt)
            })
            .await
            .unwrap()
        };
        assert!(matches!(outcome, StatusPollOutcome::Completed(_)));

        // One more timeout after the reset must look like "1 consecutive,"
        // not continue the earlier streak — must not escalate.
        let mut subscriber = state.subscribe_events();
        stuck_poll(&state, &rt).await;

        let mut saw_persistent = false;
        while let Ok(event) = subscriber.try_recv() {
            if matches!(event, ServerEvent::CameraPersistentlyUnresponsive { .. }) {
                saw_persistent = true;
            }
        }
        assert!(
            !saw_persistent,
            "a successful poll in between should have reset the timeout streak"
        );
    }

    /// Watchdog timeout used across the `capture_frame_bounded` tests below —
    /// short so the "stuck" tests stay fast, distinct from any production
    /// constant since the real timeout is computed dynamically per-attempt.
    const TEST_CAPTURE_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(3);

    #[test]
    fn capture_frame_bounded_completes_when_fast() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let camera: Box<dyn crate::camera::Camera> =
            Box::new(TestCamera::with_capture_delay(Duration::ZERO));

        let outcome = capture_frame_bounded(
            camera,
            crate::camera::CaptureConfig::default(),
            1,
            TEST_CAPTURE_WATCHDOG_TIMEOUT,
            &state,
        );

        assert!(
            matches!(outcome, CaptureOutcome::Completed(_, Ok(_))),
            "expected a completed, successful capture for a fast camera"
        );
    }

    /// The whole point of the watchdog: a stuck `capture()` call must not
    /// block the caller past its timeout, even though the underlying call
    /// (and its thread) keeps running in the background afterward.
    #[test]
    fn capture_frame_bounded_times_out_on_stuck_call() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let camera: Box<dyn crate::camera::Camera> = Box::new(TestCamera::with_capture_delay(
            TEST_CAPTURE_WATCHDOG_TIMEOUT + Duration::from_secs(5),
        ));

        let start = Instant::now();
        let outcome = capture_frame_bounded(
            camera,
            crate::camera::CaptureConfig::default(),
            1,
            TEST_CAPTURE_WATCHDOG_TIMEOUT,
            &state,
        );
        let elapsed = start.elapsed();

        assert!(matches!(outcome, CaptureOutcome::TimedOut));
        assert!(
            elapsed < TEST_CAPTURE_WATCHDOG_TIMEOUT + Duration::from_secs(2),
            "capture_frame_bounded should return around its watchdog timeout, \
             not wait for the stuck call; took {:?}",
            elapsed
        );
    }

    fn stuck_capture(state: &Arc<AppState>) {
        let camera: Box<dyn crate::camera::Camera> = Box::new(TestCamera::with_capture_delay(
            TEST_CAPTURE_WATCHDOG_TIMEOUT + Duration::from_secs(5),
        ));
        capture_frame_bounded(
            camera,
            crate::camera::CaptureConfig::default(),
            1,
            TEST_CAPTURE_WATCHDOG_TIMEOUT,
            state,
        );
    }

    /// Capture timeouts feed the *same* persistent-fault counter as status
    /// timeouts (`PERSISTENT_FAULT_THRESHOLD` is shared) — a stuck capture is
    /// equally strong evidence of a hardware/USB fault.
    #[test]
    fn capture_frame_bounded_escalates_after_persistent_timeouts() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        for i in 1..=PERSISTENT_FAULT_THRESHOLD {
            let mut subscriber = state.subscribe_events();
            stuck_capture(&state);

            let mut saw_persistent = false;
            while let Ok(event) = subscriber.try_recv() {
                if let ServerEvent::CameraPersistentlyUnresponsive {
                    consecutive_timeouts,
                    ..
                } = event
                {
                    assert_eq!(
                        consecutive_timeouts, i,
                        "escalation event should report the current streak length"
                    );
                    saw_persistent = true;
                }
            }
            assert_eq!(
                saw_persistent,
                i >= PERSISTENT_FAULT_THRESHOLD,
                "persistent-unresponsive event should only fire from the threshold-th \
                 consecutive timeout onward (iteration {i})"
            );
        }
    }

    #[test]
    fn capture_frame_bounded_resets_streak_on_success() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        // One timeout short of the threshold.
        for _ in 0..(PERSISTENT_FAULT_THRESHOLD - 1) {
            stuck_capture(&state);
        }

        // A fast, successful capture should clear the streak.
        let fast_camera: Box<dyn crate::camera::Camera> =
            Box::new(TestCamera::with_capture_delay(Duration::ZERO));
        let outcome = capture_frame_bounded(
            fast_camera,
            crate::camera::CaptureConfig::default(),
            1,
            TEST_CAPTURE_WATCHDOG_TIMEOUT,
            &state,
        );
        assert!(matches!(outcome, CaptureOutcome::Completed(_, Ok(_))));

        // One more timeout after the reset must look like "1 consecutive,"
        // not continue the earlier streak — must not escalate.
        let mut subscriber = state.subscribe_events();
        stuck_capture(&state);

        let mut saw_persistent = false;
        while let Ok(event) = subscriber.try_recv() {
            if matches!(event, ServerEvent::CameraPersistentlyUnresponsive { .. }) {
                saw_persistent = true;
            }
        }
        assert!(
            !saw_persistent,
            "a successful capture in between should have reset the timeout streak"
        );
    }

    #[test]
    fn capture_watchdog_margin_floors_at_zero_exposure() {
        let margin = capture_watchdog_margin(0, Duration::from_secs(120));
        assert_eq!(margin, CAPTURE_WATCHDOG_MIN_MARGIN);
    }

    #[test]
    fn capture_watchdog_margin_is_tight_for_live_view_exposure() {
        // 10ms — the actual live-view exposure from the field incident.
        let margin = capture_watchdog_margin(10_000, Duration::from_secs(120));
        assert!(
            margin > CAPTURE_WATCHDOG_MIN_MARGIN
                && margin < CAPTURE_WATCHDOG_MIN_MARGIN + Duration::from_millis(100),
            "expected a margin just barely above the floor for a 10ms exposure, got {margin:?}"
        );
        // The whole point: this must be short enough that a repeat of the
        // 6.96s field incident would actually trip it.
        assert!(
            Duration::from_micros(10_000) + margin < Duration::from_secs_f64(6.96),
            "a 10ms-exposure watchdog timeout of {:?} would not have caught the 6.96s incident",
            Duration::from_micros(10_000) + margin
        );
    }

    #[test]
    fn capture_watchdog_margin_is_midpoint_at_half_ramp() {
        let config_timeout = Duration::from_secs(120);
        let max_margin = config_timeout + CAPTURE_WATCHDOG_SLACK;
        let half_ramp = CAPTURE_WATCHDOG_RAMP_EXPOSURE / 2;

        let margin = capture_watchdog_margin(half_ramp.as_micros() as u64, config_timeout);
        let expected = CAPTURE_WATCHDOG_MIN_MARGIN + (max_margin - CAPTURE_WATCHDOG_MIN_MARGIN) / 2;
        let diff = margin.as_secs_f64() - expected.as_secs_f64();
        assert!(diff.abs() < 0.01, "expected ~{expected:?}, got {margin:?}");
    }

    #[test]
    fn capture_watchdog_margin_reaches_and_holds_full_budget_past_ramp() {
        let config_timeout = Duration::from_secs(120);
        let max_margin = config_timeout + CAPTURE_WATCHDOG_SLACK;

        let at_ramp = capture_watchdog_margin(
            CAPTURE_WATCHDOG_RAMP_EXPOSURE.as_micros() as u64,
            config_timeout,
        );
        assert_eq!(at_ramp, max_margin);

        // A long deep-sky sub (5 minutes) must get exactly the same, unchanged
        // budget as before this change — no regression for long exposures.
        let long_exposure = capture_watchdog_margin(300_000_000, config_timeout);
        assert_eq!(long_exposure, max_margin);
    }

    #[test]
    fn capture_watchdog_margin_ceiling_tracks_config_timeout() {
        // The ceiling must follow a non-default config_timeout, not a
        // hardcoded constant.
        let config_timeout = Duration::from_secs(60);
        let margin = capture_watchdog_margin(
            CAPTURE_WATCHDOG_RAMP_EXPOSURE.as_micros() as u64,
            config_timeout,
        );
        assert_eq!(margin, config_timeout + CAPTURE_WATCHDOG_SLACK);
    }

    #[test]
    fn capture_watchdog_margin_guards_against_inverted_range() {
        // A pathologically small config_timeout must not produce a ceiling
        // below the floor.
        let config_timeout = Duration::from_secs(1);
        let margin = capture_watchdog_margin(
            CAPTURE_WATCHDOG_RAMP_EXPOSURE.as_micros() as u64,
            config_timeout,
        );
        assert!(margin >= CAPTURE_WATCHDOG_MIN_MARGIN);
    }
}
