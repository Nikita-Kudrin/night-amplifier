use crate::camera::Camera;
use crate::server::camera_health::{self, FaultKind};
use crate::server::capture::channel::CapturedFrame;
use crate::server::state::AppState;
use crate::telemetry::metrics as telemetry_metrics;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tracing::{debug, error, warn};

/// Cadence for polling cooled-camera status from the capture thread.
pub(crate) const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Hard bound on a single `camera.status()` call (see `poll_camera_status_bounded`).
/// If it doesn't return within this, the handle is abandoned rather than left
/// to block frame delivery indefinitely — no vendor SDK call other than the
/// image-data read exposes a timeout of its own.
pub(crate) const STATUS_POLL_TIMEOUT: Duration = Duration::from_secs(3);

/// Added on top of a capture attempt's own `config.timeout + exposure` budget
/// to get `capture_frame_bounded`'s watchdog timeout — gives the backend's own
/// internal timeout-and-cleanup the first chance to fire before this external
/// last resort does. See `capture_frame_bounded`.
pub(crate) const CAPTURE_WATCHDOG_SLACK: Duration = Duration::from_secs(10);

/// Floor for `capture_watchdog_margin` at (near-)zero exposure — e.g. a 10ms
/// live-view frame. Set comfortably above the normal jitter ceiling observed
/// for healthy captures (up to ~1.6s) so it doesn't false-positive on
/// ordinary variance, while still catching a multi-second stall quickly
/// instead of only after the full long-exposure budget.
pub(crate) const CAPTURE_WATCHDOG_MIN_MARGIN: Duration = Duration::from_secs(5);

/// Exposure length at which `capture_watchdog_margin` finishes ramping up to
/// the full `config.timeout + CAPTURE_WATCHDOG_SLACK` ceiling. Set above
/// typical EAA live-stacking sub-exposure lengths (commonly single-digit to
/// ~20s), so most live-stacking sessions get meaningfully tighter, scaled
/// protection instead of the flat long-exposure budget — while exposures at
/// or beyond this (long deep-sky subs, the actual reason for a generous
/// ceiling) get full trust.
pub(crate) const CAPTURE_WATCHDOG_RAMP_EXPOSURE: Duration = Duration::from_secs(30);

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
pub(crate) fn capture_watchdog_margin(exposure_us: u64, config_timeout: Duration) -> Duration {
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

pub(crate) enum StatusPollOutcome {
    /// The call returned in time. The camera handle is returned so the
    /// capture loop can keep using it.
    Completed(Box<dyn crate::camera::Camera>),
    /// The call did not return within `STATUS_POLL_TIMEOUT`. The camera
    /// handle is gone for good — see the function doc for why.
    TimedOut,
}

/// Read the camera's live status, cache it, and broadcast a `CameraStatusUpdated`
/// event — bounded by `STATUS_POLL_TIMEOUT`. The camera handle is owned by exactly
/// one thread at a time (avoiding contention with vendor SDKs that require a single
/// handle per device); that thread is temporarily a detached watchdog thread for the
/// duration of this call, so a stuck read can't block frame delivery — no vendor SDK
/// call but the image-data read has its own timeout, so an unbounded USB hiccup in
/// `camera.status()` could otherwise block live view silently (observed: seconds to
/// indefinitely).
///
/// Waits up to `STATUS_POLL_TIMEOUT` on a channel; if it returns in time the handle
/// comes back normally, otherwise it's abandoned for good — no way to forcibly
/// cancel a stuck synchronous FFI call in Rust, so callers must treat `TimedOut` as
/// a real disconnect. Abandoning is only safe because of `camera::DeviceLease`: SDKs
/// close by device *index*, so a `Drop` running minutes later closes whichever
/// handle owns that index by then (killed a camera 80s after a successful reconnect
/// on 2026-08-22) — the lease makes a superseded handle's close a no-op.
///
/// Also tracks consecutive timeouts per camera to distinguish a USB hiccup from a
/// persistent fault — see `camera_health::PERSISTENT_FAULT_THRESHOLD`.
pub(crate) fn poll_camera_status_bounded(
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
            camera_health::clear_fault_streak(state, &camera_name);

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
            camera_health::record_fault(state, &camera_name, FaultKind::Timeout);
            state.send_error(camera_health::incident_message(
                &camera_name,
                FaultKind::Timeout,
            ));
            StatusPollOutcome::TimedOut
        }
    }
}

/// Outcome of a bounded `camera.capture()` call — see `capture_frame_bounded`.
pub(crate) enum CaptureOutcome {
    /// The call returned in time — successfully or with an error either way,
    /// so the caller can still see what `capture()` reported while getting
    /// the handle back to keep using.
    Completed(
        Box<dyn crate::camera::Camera>,
        crate::camera::CameraResult<crate::camera::RawFrame>,
    ),
    /// The call did not return within `watchdog_timeout`. The camera handle
    /// is gone for good — see `poll_camera_status_bounded`'s doc for why this
    /// is safe (every backend's handle type implements `Drop`) and why there
    /// is no alternative for a stuck synchronous FFI call.
    TimedOut,
}

/// Run `camera.capture(&config)` bounded by `watchdog_timeout`, the same way
/// `poll_camera_status_bounded` bounds `camera.status()`. Every backend's internal
/// capture loop already self-enforces a "total budget"
/// (`config.timeout + exposure duration`) *between* its blocking SDK calls
/// (confirmed identical across all five vendors), but can't fire if one of those
/// calls itself hangs — observed: a ~3-minute freeze inside PlayerOne's
/// `is_image_ready()` poll, unresponsive to Stop, before the SDK finally errored.
/// `watchdog_timeout` is set slightly above that internal budget for long exposures
/// (so the backend's own cleanup runs first, this is the last resort), and scaled
/// down via `capture_watchdog_margin` for short ones (live view, planetary), where
/// the full budget would be far too tolerant.
///
/// Caveat: if cancellation lands while `capture()` is already stuck and this
/// watchdog fires first, the session ends as a disconnect, not a clean stop — no way
/// to do better without real vendor-SDK cancellation, but still strictly better than
/// hanging indefinitely.
pub(crate) fn capture_frame_bounded(
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
            let span = tracing::info_span!(
                "camera_capture",
                frame_number,
                exposure_us = config.exposure_us,
                gain = config.gain,
                bin = config.bin,
                call_us = tracing::field::Empty,
                overhead_us = tracing::field::Empty,
            );
            let _entered = span.enter();
            let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::Capture);
            let started = std::time::Instant::now();
            let result = camera.capture(&config);

            // How long the vendor call blocked, vs. the exposure it was asked for.
            // Fields, not a child span: `Camera::capture` is one blocking vendor call
            // (on the continuous path, `get_video_data` handing back an
            // already-completed frame), so exposure and transfer aren't separable
            // from out here without instrumenting inside all five shims for a
            // boundary that doesn't exist in the mode live stacking uses.
            //
            // Purpose: `camera_capture` reported 131ms against a 100ms exposure in
            // production traces, with nothing saying whether the extra 31ms was a
            // slow link or a long exposure — matters on a Pi 5, where shared USB3
            // degrades first.
            //
            // `overhead_us` is **signed**: a saturating unsigned version reported `0`
            // on exactly the path it was added for (continuous mode's already-waiting
            // frame), so every sample saturated and the field answered nothing.
            // Negative now means the frame was already waiting; `call_us` carries the
            // raw measurement for any other arithmetic needed.
            let call_us = started.elapsed().as_micros().min(i64::MAX as u128) as i64;
            span.record("call_us", call_us);
            span.record(
                "overhead_us",
                call_us.saturating_sub(config.exposure_us as i64),
            );
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
            // A device-lost error is an answer, not a silence: the camera is
            // talking, but says its handle is dead. That is evidence of the
            // same fault the timeout branch counts, so it must not clear the
            // streak — it extends it.
            match &result {
                Err(e) if e.is_sdk_disconnected() => {
                    camera_health::record_fault(state, &camera_name, FaultKind::DeviceLost);
                }
                _ => camera_health::clear_fault_streak(state, &camera_name),
            }
            CaptureOutcome::Completed(camera, result)
        }
        Err(_) => {
            error!(
                camera_name = %camera_name,
                timeout = ?watchdog_timeout,
                "camera.capture() did not return in time — abandoning camera handle (suspected USB stall)"
            );
            camera_health::record_fault(state, &camera_name, FaultKind::Timeout);
            state.send_error(camera_health::incident_message(
                &camera_name,
                FaultKind::Timeout,
            ));
            CaptureOutcome::TimedOut
        }
    }
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use crate::server::camera_health::PERSISTENT_FAULT_THRESHOLD;
    use crate::server::events::ServerEvent;
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
        fn set_dew_heater(
            &mut self,
            _enabled: bool,
            _power: i32,
        ) -> crate::camera::CameraResult<()> {
            Ok(())
        }
        fn capture(
            &mut self,
            _config: &crate::camera::CaptureConfig,
        ) -> crate::camera::CameraResult<crate::camera::RawFrame> {
            if !self.capture_delay.is_zero() {
                std::thread::sleep(self.capture_delay);
            }
            Ok(crate::camera::RawFrame {
                data: vec![0; 4 * 4].into(),
                width: 4,
                height: 4,
                format: crate::camera::ImageFormat::Raw8,
            })
        }
        fn cancel(&self) {
            self.cancel_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
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
    pub(crate) async fn poll_camera_status_bounded_completes_when_fast() {
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
    pub(crate) async fn poll_camera_status_bounded_times_out_on_stuck_call() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let camera: Box<dyn crate::camera::Camera> = Box::new(TestCamera::new(
            STATUS_POLL_TIMEOUT + Duration::from_secs(5),
        ));
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
        let camera: Box<dyn crate::camera::Camera> = Box::new(TestCamera::new(
            STATUS_POLL_TIMEOUT + Duration::from_secs(5),
        ));
        let state = Arc::clone(state);
        let rt = rt.clone();
        tokio::task::spawn_blocking(move || poll_camera_status_bounded(camera, &state, None, &rt))
            .await
            .unwrap();
    }

    #[tokio::test]
    pub(crate) async fn poll_camera_status_bounded_escalates_after_persistent_timeouts() {
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
    pub(crate) async fn poll_camera_status_bounded_resets_streak_on_success() {
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
    pub(crate) fn capture_frame_bounded_completes_when_fast() {
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
    pub(crate) fn capture_frame_bounded_times_out_on_stuck_call() {
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
    pub(crate) fn capture_frame_bounded_escalates_after_persistent_timeouts() {
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
    pub(crate) fn capture_frame_bounded_resets_streak_on_success() {
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
    pub(crate) fn capture_watchdog_margin_floors_at_zero_exposure() {
        let margin = capture_watchdog_margin(0, Duration::from_secs(120));
        assert_eq!(margin, CAPTURE_WATCHDOG_MIN_MARGIN);
    }

    #[test]
    pub(crate) fn capture_watchdog_margin_is_tight_for_live_view_exposure() {
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
    pub(crate) fn capture_watchdog_margin_is_midpoint_at_half_ramp() {
        let config_timeout = Duration::from_secs(120);
        let max_margin = config_timeout + CAPTURE_WATCHDOG_SLACK;
        let half_ramp = CAPTURE_WATCHDOG_RAMP_EXPOSURE / 2;

        let margin = capture_watchdog_margin(half_ramp.as_micros() as u64, config_timeout);
        let expected = CAPTURE_WATCHDOG_MIN_MARGIN + (max_margin - CAPTURE_WATCHDOG_MIN_MARGIN) / 2;
        let diff = margin.as_secs_f64() - expected.as_secs_f64();
        assert!(diff.abs() < 0.01, "expected ~{expected:?}, got {margin:?}");
    }

    #[test]
    pub(crate) fn capture_watchdog_margin_reaches_and_holds_full_budget_past_ramp() {
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
    pub(crate) fn capture_watchdog_margin_ceiling_tracks_config_timeout() {
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
    pub(crate) fn capture_watchdog_margin_guards_against_inverted_range() {
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
