//! The guide camera's free-running loop.
//!
//! Deliberately not the four-thread imaging pipeline: a guide camera is never stacked,
//! so there is nothing to accumulate, no display copy to negotiate and no stage that can
//! fall behind. One thread does the whole job.
//!
//! It starts on connect rather than on Start Capture, because the two things it exists
//! for — plate solving and a look through the guide scope — are what you want *while*
//! framing, before any imaging session has begun.
//!
//! # The render gate
//!
//! Post-processing is identical to the main camera's, and runs only while somebody is
//! actually watching the guide stream ([`FrameStream::has_viewers`]). Nobody watching
//! means no background extraction, no stretch solve, no encode — the expensive two
//! thirds of a frame's cost — so a connected guide camera does not double the CPU bill
//! of a session that is only ever looking at the main image. Solving and raw saving are
//! *not* gated: both are the reason the loop is running at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, error, info, warn};

use super::analysis::{AnalysisContext, PreviewAnalysis};
use super::render_task::{encode_jpeg_tiers, ConversionCache};
use super::solving::{self, SolveSource};
use super::stage_config;
use super::storage;
use super::watchdog::{capture_frame_bounded, capture_watchdog_margin, CaptureOutcome};
use crate::camera::Camera;
use crate::disk_writer::{OpenSession, WritingSessionType};
use crate::camera::CameraStatus;
use crate::server::camera_session::ramp::RampState;
use crate::server::state::{
    AppState, CameraCaptureProfile, CameraOp, CameraRole, CaptureMode, CaptureSettings,
    ConnectedCameraInfo, RawSessionResume, RenderReadyFrame,
};

/// How long the loop waits before retrying after a recoverable capture error, so a
/// camera erroring instantly cannot spin a core.
const ERROR_BACKOFF: Duration = Duration::from_millis(500);

/// Longer wait after the camera rejected the capture config. Retrying that fast is
/// pointless — nothing changes until the user edits a setting — but the loop keeps
/// polling so it recovers the moment they do.
const REJECTED_CONFIG_BACKOFF: Duration = Duration::from_secs(2);

/// Start the guide loop for a freshly connected guide camera.
///
/// Non-blocking: the handle is checked out on a tokio task, because `take_for_capture`
/// may have to wait for the monitor to hand it back and `connect` must not block on that.
pub fn start(state: &Arc<AppState>, camera: &ConnectedCameraInfo) {
    // Published before the spawn, not inside it: a disconnect arriving while the task is
    // still queued would otherwise find no token, decide no loop was running, and close
    // the handle out from under it.
    let cancel = Arc::new(AtomicBool::new(false));
    *state
        .guide_cancel
        .lock()
        .expect("guide_cancel mutex poisoned") = Some(Arc::clone(&cancel));

    let state = Arc::clone(state);
    let camera = camera.clone();
    tokio::spawn(async move {
        if let Err(e) = spawn_loop(&state, &camera, cancel).await {
            error!(camera = %camera.info.name, error = %e, "Could not start the guide loop");
            state.clear_guide_cancel();
            state.send_error(format!(
                "Guide camera '{}' connected but its loop could not start: {}",
                camera.info.name, e
            ));
        }
    });
}

async fn spawn_loop(
    state: &Arc<AppState>,
    camera_info: &ConnectedCameraInfo,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let camera_name = camera_info.info.name.clone();

    // A disconnect that landed between `start` and here has already set the token; taking
    // the handle now would leave it checked out of a slot nobody is going to reclaim.
    if cancel.load(Ordering::SeqCst) {
        return Ok(());
    }

    let camera = crate::server::camera_session::lifecycle::take_for_capture(
        state,
        CameraRole::Guide,
        &camera_name,
    )
    .await
    .map_err(|e| e.to_string())?;

    state
        .set_camera_token(CameraRole::Guide, camera.cancel_token())
        .await;

    // Rejoin the folder a dropout interrupted, so one guide session stays in one
    // directory across a reconnect — the same reason `SessionResumePlan` carries the
    // imaging camera's.
    let resume = state.slot(CameraRole::Guide).raw_session.read().await.clone();

    let loop_state = Arc::clone(state);
    let loop_info = camera_info.clone();
    let rt = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("guide-task".into())
        .spawn(move || {
            // Set here rather than in `connect`: this is the first moment a loop
            // certainly exists, so a spawn that never got this far cannot leave the
            // imaging camera stood down for a solve source that is not there.
            loop_state.set_guide_loop_running(true);
            let camera = run(&loop_state, &loop_info, camera, &cancel, resume, &rt);
            loop_state.set_guide_loop_running(false);

            // Retire the token *before* handing the handle back. On the device-loss path
            // `return_from_capture(None)` reaches `finalize_disconnect`, which asks this
            // loop to stop — and a loop asking itself to stop would sit out the whole
            // wait budget for a handle it has already lost.
            loop_state.clear_guide_cancel();

            rt.block_on(crate::server::camera_session::lifecycle::return_from_capture(
                &loop_state,
                CameraRole::Guide,
                &loop_info.info.name,
                camera,
            ));
        })
        .map_err(|e| format!("failed to spawn the guide thread: {e}"))?;

    info!(camera = %camera_name, "Guide camera loop started");
    Ok(())
}

/// Ask the guide loop to stop and wait for it to hand the handle back.
///
/// Called before anything closes the handle. Returns once the slot holds a handle again
/// or the wait budget expires — a loop stuck inside a vendor call has already abandoned
/// its handle to the capture watchdog, and waiting longer would not produce one.
pub async fn stop(state: &Arc<AppState>) {
    // Cleared before the early return, not after it: once this function has been called
    // the loop is not running, and the flag has to say so whether or not there was a
    // token to signal. Solving goes back to the imaging camera immediately — a cooled
    // guide camera then warms up for minutes, and leaving the flag set through all of it
    // means neither camera may offer the solver a frame.
    state.set_guide_loop_running(false);
    let Some(cancel) = state.take_guide_cancel() else {
        return;
    };
    cancel.store(true, Ordering::SeqCst);
    // Cut short the exposure in flight, or the stop waits out a full guide sub.
    state.slot(CameraRole::Guide).cancel_exposure().await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if state.slot(CameraRole::Guide).holds_handle() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    debug!("Guide loop did not return its handle within the stop budget");
}

/// The loop body. Returns the handle unless a watchdog abandoned it.
fn run(
    state: &Arc<AppState>,
    camera_info: &ConnectedCameraInfo,
    mut camera: Box<dyn Camera>,
    cancel: &AtomicBool,
    resume: Option<RawSessionResume>,
    rt: &tokio::runtime::Handle,
) -> Option<Box<dyn Camera>> {
    debug!(camera = %camera_info.info.name, "Guide task started");

    let mut conversions = ConversionCache::default();
    // Outlives the loop for the same reason the render task's does: the background
    // model and image statistics describe the sky, not the frame.
    let mut analysis = PreviewAnalysis::new();
    let mut cfa_key = None;
    let mut cfa_pipeline = None;
    let mut disk = GuideDiskSession::new(resume);
    let mut frame_number: u64 = disk.first_frame_number() - 1;
    // The config error already reported, so the same one is not reported again.
    let mut rejected_config: Option<String> = None;
    let mut cooler = GuideCooler::default();
    let mut sensor = SensorReadout::default();

    while !cancel.load(Ordering::SeqCst) {
        let settings = rt.block_on(state.settings.read()).clone();
        let profile = settings.guide_camera.clone();
        let mut config = settings.to_capture_config_with(&profile, CameraRole::Guide);

        // Everything below runs between exposures, which is the only moment anything can
        // reach this camera: the loop owns its handle for the whole connection, so the
        // monitor thread — which drives these for the imaging camera — can never check
        // it out.
        for op in state.slot(CameraRole::Guide).drain_ops() {
            apply_op(camera.as_mut(), op, &camera_info.info.name);
        }
        sensor.sample(camera.as_ref(), state, camera_info, &profile, rt);
        if let Some(setpoint) =
            cooler.setpoint(&profile, sensor.temperature_c, std::time::Instant::now())
        {
            config.target_temp_c = Some(setpoint);
        }
        super::config_overrides::apply_best_raw_format(
            &mut config,
            &camera_info.info,
            &camera_info.info.name,
        );
        super::config_overrides::apply_cooler_support_override(
            &mut config,
            &camera_info.info,
            &camera_info.info.name,
        );
        super::config_overrides::apply_sensor_mode_support_override(
            &mut config,
            &camera_info.info,
            &camera_info.info.name,
        );

        frame_number += 1;
        let watchdog_timeout = Duration::from_micros(config.exposure_us)
            + capture_watchdog_margin(config.exposure_us, config.timeout);
        let (returned, result) =
            match capture_frame_bounded(camera, config, frame_number, watchdog_timeout, state) {
                CaptureOutcome::Completed(cam, result) => (cam, result),
                // The handle went with a detached thread that never returned. Nothing
                // left to hand back; the fault detector has already recorded it.
                CaptureOutcome::TimedOut => return None,
            };
        camera = returned;

        let raw_frame = match result {
            Ok(frame) => Arc::new(frame),
            Err(e) => {
                if matches!(e, crate::camera::CameraError::Cancelled) {
                    camera.cancel_token().store(false, Ordering::SeqCst);
                    continue;
                }
                if e.is_sdk_disconnected() {
                    error!(error = %e, "Guide camera disconnected during capture");
                    state.send_error(format!("Guide camera disconnected: {}", e));
                    return None;
                }
                if let crate::camera::CameraError::InvalidParameter { .. } = e {
                    // The camera rejected the config, so it will reject the identical
                    // one next frame too. Report it once and idle instead of filling
                    // the log at 2 Hz — the loop stays up because a settings edit is
                    // exactly how the user fixes this.
                    let message = e.to_string();
                    if rejected_config.as_deref() != Some(&message) {
                        error!(error = %e, "Guide camera rejected its capture settings");
                        state.send_error(format!(
                            "Guide camera '{}' rejected its settings: {}",
                            camera_info.info.name, e
                        ));
                        rejected_config = Some(message);
                    }
                    std::thread::sleep(REJECTED_CONFIG_BACKOFF);
                    continue;
                }
                warn!(error = %e, "Guide frame capture failed");
                std::thread::sleep(ERROR_BACKOFF);
                continue;
            }
        };
        rejected_config = None;

        // Above both gates below: an unwatched guide camera with no solve target is
        // still saving subs if the user asked it to.
        disk.write(state, &settings, &raw_frame, frame_number, camera_info, rt);

        let stream = Arc::clone(&state.guide_stream);
        let watched = stream.has_viewers();
        let solving_wanted = solving::plate_solve_available(state, SolveSource::Guide);
        if !watched && !solving_wanted {
            continue;
        }

        // Rebuilt only when the settings behind it change, exactly as the stacking task
        // does — building a `CfaPipeline` per frame is pure waste.
        let key = (settings.sensor_correction.clone(), settings.stacking_type);
        if cfa_key.as_ref() != Some(&key) {
            cfa_pipeline = Some(stage_config::build_cfa_pipeline(&settings));
            cfa_key = Some(key);
        }
        let algorithm = stage_config::debayer_algorithm(&settings);
        let frame = match stage_config::convert_captured_frame(
            &raw_frame,
            &camera_info.info,
            cfa_pipeline.as_ref().expect("cfa pipeline built above"),
            algorithm,
        ) {
            Ok(frame) => Arc::new(frame),
            Err(e) => {
                warn!(error = %e, "Guide frame conversion failed");
                continue;
            }
        };

        if solving_wanted {
            rt.spawn({
                let state = Arc::clone(state);
                let frame = Arc::clone(&frame);
                async move {
                    solving::try_plate_solve(&state, frame, SolveSource::Guide).await;
                }
            });
        }

        if !watched {
            continue;
        }

        render_and_publish(
            &stream,
            frame,
            &settings,
            &mut conversions,
            &mut analysis,
            rt,
        );
    }

    debug!(camera = %camera_info.info.name, "Guide task ended");
    Some(camera)
}

/// Run one queued hardware call against the handle.
fn apply_op(camera: &mut dyn Camera, op: CameraOp, camera_name: &str) {
    match op {
        CameraOp::SetDewHeater { enabled, power } => match camera.set_dew_heater(enabled, power) {
            Ok(()) => info!(camera = %camera_name, enabled, power, "Guide dew heater applied"),
            Err(e) => warn!(error = %e, "Guide dew heater change failed"),
        },
    }
}

/// How often the loop reads the sensor and broadcasts a status sample.
///
/// Matched to the monitor's `PHASE_POLL_INTERVAL` so the two cameras' readouts update
/// at the same rate, rather than the guide camera's tracking its exposure length.
const STATUS_INTERVAL: Duration = Duration::from_secs(2);

/// The guide camera's status sampling, which the monitor cannot do for it.
///
/// `status()` is called straight rather than through `monitor::FfiWorker`: that worker
/// exists to keep a stalled vendor call off a *shared* handle and off the tokio
/// runtime, and this loop is neither — it is a dedicated OS thread that already blocks
/// on `capture()` against the same device. A stall here wedges the guide loop exactly
/// as a stalled exposure does, and `guide_task::stop` already gives up on a wedged loop.
#[derive(Default)]
struct SensorReadout {
    last_sampled_at: Option<std::time::Instant>,
    /// Last temperature read, used to start the cooler ramp from where the sensor
    /// actually is rather than from its target.
    temperature_c: Option<f64>,
}

impl SensorReadout {
    fn sample(
        &mut self,
        camera: &dyn Camera,
        state: &Arc<AppState>,
        camera_info: &ConnectedCameraInfo,
        profile: &CameraCaptureProfile,
        rt: &tokio::runtime::Handle,
    ) {
        let now = std::time::Instant::now();
        let due = self
            .last_sampled_at
            .is_none_or(|last| now.duration_since(last) >= STATUS_INTERVAL);
        if !due {
            return;
        }
        self.last_sampled_at = Some(now);

        let status: CameraStatus = match camera.status() {
            Ok(status) => status,
            Err(e) => {
                debug!(error = %e, "Guide camera status read failed");
                return;
            }
        };
        self.temperature_c = Some(status.temperature_c);
        rt.block_on(state.update_camera_status(
            &camera_info.info.name,
            status,
            profile.target_temp_c,
        ));
    }
}

/// The guide camera's TEC setpoint, ramped by the loop that owns its handle.
///
/// Without this the per-frame config pushed the user's final target straight at the
/// sensor, so `RAMP_RATE_C_PER_MIN` — the 5 °C/min limit that exists to keep a cover
/// glass from condensing and a die from being thermally shocked — applied to the
/// imaging camera and not to this one.
#[derive(Default)]
struct GuideCooler {
    ramp: Option<RampState>,
    /// The cooler settings the current ramp was built for. A settings edit changes this
    /// and restarts the ramp from wherever the sensor has got to.
    goal: Option<(bool, Option<f64>, bool)>,
}

impl GuideCooler {
    /// The setpoint this frame's config should carry, or `None` to leave it alone.
    ///
    /// `None` covers the three cases with nothing to ramp: the cooler is off, no target
    /// is set, or fast mode is on — which is documented to snap to the setpoint and is
    /// the switch a user flips when they accept that.
    fn setpoint(
        &mut self,
        profile: &CameraCaptureProfile,
        sensor_temp_c: Option<f64>,
        now: std::time::Instant,
    ) -> Option<f64> {
        let goal = (
            profile.cooler_enabled,
            profile.target_temp_c,
            profile.cooler_fast_mode,
        );
        if self.goal != Some(goal) {
            self.goal = Some(goal);
            self.ramp = None;
        }

        let target = profile.target_temp_c?;
        if !profile.cooler_enabled || profile.cooler_fast_mode {
            self.ramp = None;
            return None;
        }

        let ramp = self.ramp.get_or_insert_with(|| {
            // Starting from the target rather than an unknown sensor temperature would
            // be a ramp that is already finished, i.e. no ramp at all.
            let start = sensor_temp_c.unwrap_or(target);
            RampState::new_from_current(start, target, now)
        });
        ramp.step(now);
        // The rounded value, not the logical one: a fractional setpoint that moves every
        // frame would make the backend re-issue `set_target_temperature` every frame,
        // which is exactly what `commanded_i64` exists to avoid.
        Some(ramp.commanded_i64() as f64)
    }
}

/// Run the same preview pipeline the main camera gets, then publish and encode.
///
/// Only reached with a viewer connected — see the module doc.
fn render_and_publish(
    stream: &Arc<crate::server::state::FrameStream>,
    frame: Arc<crate::frame::Frame>,
    settings: &CaptureSettings,
    conversions: &mut ConversionCache,
    analysis: &mut PreviewAnalysis,
    rt: &tokio::runtime::Handle,
) {
    let _span = tracing::info_span!("guide_render").entered();

    let mut display_frame = frame;
    let (pipeline_config, stretch_result) = match super::pipeline::process_preview_frame_with_analysis(
        Arc::make_mut(&mut display_frame),
        settings,
        // Every guide frame is a single sub — there is no stack behind it, which is the
        // same context live view runs in.
        AnalysisContext::ONE_SHOT,
        analysis,
    ) {
        Ok(res) => res,
        Err(e) => {
            warn!(error = %e, "Guide preview processing failed");
            return;
        }
    };

    let ready = Arc::new(RenderReadyFrame {
        linear_frame: display_frame,
        pipeline_config,
        stretch_result,
    });

    rt.block_on(stream.set_latest_raw_frame(Arc::clone(&ready)));
    let counter = stream.begin_frame();

    conversions.begin_frame();
    if stream.lossless_client_count() > 0 {
        let (max_w, max_h) = stream.lossless_target_box();
        match conversions.get(&ready, max_w, max_h) {
            Some(rgb) => {
                match crate::server::encoding::encode_rgb8_lz4_chunked_from_u8(
                    &rgb.0, rgb.1, rgb.2, 1,
                ) {
                    Ok(encoded) => rt.block_on(stream.set_latest_frame(encoded)),
                    Err(e) => warn!(error = %e, "Guide LZ4 encoding failed"),
                }
            }
            None => warn!("RGB8 conversion failed for the guide lossless stream"),
        }
    }

    encode_jpeg_tiers(stream, &ready, counter, conversions);
    stream.publish_frame();
}

/// The guide camera's own raw-frame session, opened lazily and closed when the switch
/// goes off, so toggling *Save raw frames → Guide camera* mid-session takes effect on
/// the next frame the way the imaging switches do.
struct GuideDiskSession {
    session: Option<OpenSession>,
    /// A directory a dropout left behind, rejoined by the first frame that needs one,
    /// together with the number that run had reached.
    resume: Option<RawSessionResume>,
}

impl GuideDiskSession {
    fn new(resume: Option<RawSessionResume>) -> Self {
        Self {
            session: None,
            resume,
        }
    }

    /// The number the first frame of this run should take.
    ///
    /// A resumed run continues its predecessor's numbering: the writer names files
    /// `frame_{:06}.fits`, so restarting at 1 wrote straight over the frames the
    /// interrupted run had already saved into the directory being rejoined.
    fn first_frame_number(&self) -> u64 {
        self.resume.as_ref().map_or(1, |r| r.next_frame.max(1))
    }

    fn write(
        &mut self,
        state: &Arc<AppState>,
        settings: &CaptureSettings,
        raw: &Arc<crate::camera::RawFrame>,
        frame_number: u64,
        camera_info: &ConnectedCameraInfo,
        rt: &tokio::runtime::Handle,
    ) {
        if !settings.saves_guide_raw_frames() {
            if self.session.take().is_some() {
                rt.block_on(async {
                    *state.slot(CameraRole::Guide).raw_session.write().await = None;
                });
            }
            return;
        }

        // The master flag is set from the main capture's `initialize_capture_session`,
        // which never runs when only a guide camera is connected.
        state.disk_writer.set_enabled(true);

        if self.session.is_none() {
            let opened = match self.resume.take() {
                Some(resume) => state
                    .disk_writer
                    .reopen_session(resume.dir, WritingSessionType::IndividualFrames),
                None => state.disk_writer.create_session(
                    WritingSessionType::IndividualFrames,
                    CaptureMode::Guide.session_dir_suffix(),
                ),
            };
            match opened {
                Ok(session) => {
                    info!(dir = ?session.dir, "Guide raw-frame session opened");
                    self.session = Some(session);
                }
                Err(e) => {
                    warn!(error = %e, "Could not open a guide raw-frame directory");
                    return;
                }
            }
        }

        // Parked before the frame is queued, not after the session is opened: what a
        // reconnect needs is the number to carry on from, and the only moment that is
        // known is the moment a frame claims one.
        if let Some(session) = self.session.as_ref() {
            let resume = RawSessionResume {
                dir: session.dir.clone(),
                next_frame: frame_number + 1,
            };
            rt.block_on(async {
                *state.slot(CameraRole::Guide).raw_session.write().await = Some(resume);
            });
        }

        let metadata = rt.block_on(storage::guide_frame_metadata(
            state,
            settings,
            camera_info,
            frame_number,
        ));
        if let Err(e) = state.disk_writer.queue_raw_frame_in(
            self.session.clone(),
            Arc::clone(raw),
            frame_number,
            metadata,
            camera_info.info.sensor_type,
            camera_info.info.bayer_pattern,
        ) {
            warn!(frame_number, error = %e, "Guide raw frame dropped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{
        CameraInfo, CameraResult, CaptureConfig, GainPresets, ImageFormat, RawFrame, SensorType,
    };
    use crate::server::state::{JpegTier, StreamKind, TierClientGuard};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex as StdMutex;

    /// Counts its own exposures and stops the loop after `frames`, so a test drives an
    /// exact number of iterations rather than racing a wall clock.
    struct CountingCamera {
        info: CameraInfo,
        cancel_flag: Arc<AtomicBool>,
        captured: Arc<AtomicUsize>,
        stop_after: usize,
        stop: Arc<AtomicBool>,
        /// What the loop drove the hardware with, per exposure, so a test can tell a
        /// queued call from a dropped one and a ramped setpoint from a snapped one.
        log: Arc<StdMutex<DriveLog>>,
        /// Sensor temperature `status()` reports, so a ramp has somewhere to start.
        temperature_c: f64,
    }

    /// What the guide loop actually asked of the camera.
    #[derive(Default, Debug)]
    struct DriveLog {
        dew_heater: Vec<(bool, i32)>,
        setpoints: Vec<Option<f64>>,
        status_reads: usize,
    }

    impl CountingCamera {
        fn new(stop_after: usize, stop: Arc<AtomicBool>) -> (Self, Arc<AtomicUsize>) {
            let (cam, captured, _log) = Self::with_log(stop_after, stop);
            (cam, captured)
        }

        fn with_log(
            stop_after: usize,
            stop: Arc<AtomicBool>,
        ) -> (Self, Arc<AtomicUsize>, Arc<StdMutex<DriveLog>>) {
            let captured = Arc::new(AtomicUsize::new(0));
            let log = Arc::new(StdMutex::new(DriveLog::default()));
            let info = CameraInfo {
                name: "Mock Guide Camera".to_string(),
                max_width: 32,
                max_height: 24,
                sensor_type: SensorType::Mono,
                supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
                has_cooler: true,
                has_dew_heater: true,
                min_temp_c: Some(-40.0),
                max_temp_c: Some(30.0),
                ..Default::default()
            };
            (
                Self {
                    info,
                    cancel_flag: Arc::new(AtomicBool::new(false)),
                    captured: Arc::clone(&captured),
                    stop_after,
                    stop,
                    log: Arc::clone(&log),
                    temperature_c: 20.0,
                },
                captured,
                log,
            )
        }
    }

    impl Camera for CountingCamera {
        fn info(&self) -> &CameraInfo {
            &self.info
        }

        fn gain_presets(&self) -> CameraResult<GainPresets> {
            Ok(GainPresets::default())
        }

        fn status(&self) -> CameraResult<crate::camera::CameraStatus> {
            self.log.lock().unwrap().status_reads += 1;
            Ok(crate::camera::CameraStatus {
                temperature_c: self.temperature_c,
                ..Default::default()
            })
        }

        fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
            Ok(())
        }

        fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
            Ok(())
        }

        fn set_dew_heater(&mut self, enabled: bool, power: i32) -> CameraResult<()> {
            self.log.lock().unwrap().dew_heater.push((enabled, power));
            Ok(())
        }

        fn capture(&mut self, config: &CaptureConfig) -> CameraResult<RawFrame> {
            self.log.lock().unwrap().setpoints.push(config.target_temp_c);
            let n = self.captured.fetch_add(1, Ordering::SeqCst) + 1;
            if n >= self.stop_after {
                self.stop.store(true, Ordering::SeqCst);
            }
            let pixels = (self.info.max_width * self.info.max_height) as usize;
            Ok(RawFrame {
                data: vec![7u8; pixels].into(),
                width: self.info.max_width,
                height: self.info.max_height,
                format: ImageFormat::Raw8,
            })
        }

        fn cancel(&self) {
            self.cancel_flag.store(true, Ordering::SeqCst);
        }

        fn cancel_token(&self) -> Arc<AtomicBool> {
            Arc::clone(&self.cancel_flag)
        }

        fn close(&mut self) -> CameraResult<()> {
            Ok(())
        }

        fn provider_name(&self) -> &'static str {
            "Mock"
        }
    }

    fn guide_camera_info() -> ConnectedCameraInfo {
        ConnectedCameraInfo {
            id: "mock_0".to_string(),
            provider: "Mock".to_string(),
            index: 0,
            role: CameraRole::Guide,
            info: CameraInfo {
                name: "Mock Guide Camera".to_string(),
                max_width: 32,
                max_height: 24,
                sensor_type: SensorType::Mono,
                supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
                // Matched to `CountingCamera`: `config_overrides` strips the cooler
                // fields from a config bound for a camera that says it has none.
                has_cooler: true,
                has_dew_heater: true,
                min_temp_c: Some(-40.0),
                max_temp_c: Some(30.0),
                ..Default::default()
            },
        }
    }

    /// Run the guide loop for `frames` exposures on a blocking thread, and report how
    /// many the camera actually delivered.
    async fn drive_guide_loop(state: &Arc<AppState>, frames: usize) -> usize {
        let stop = Arc::new(AtomicBool::new(false));
        let (camera, captured) = CountingCamera::new(frames, Arc::clone(&stop));
        let info = guide_camera_info();
        let state = Arc::clone(state);
        let rt = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            run(&state, &info, Box::new(camera), &stop, None, &rt);
        })
        .await
        .expect("guide loop panicked");

        captured.load(Ordering::SeqCst)
    }

    /// The requirement in one test: a guide camera nobody is looking at must not pay for
    /// the preview pipeline. `frame_counter` only advances inside the watched branch, so
    /// it staying at zero is proof the render and the encode never ran — while the camera
    /// really did expose the frames.
    #[tokio::test]
    async fn an_unwatched_guide_stream_is_never_rendered() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);

        let captured = drive_guide_loop(&state, 4).await;

        assert_eq!(captured, 4, "the camera should still have exposed frames");
        assert_eq!(
            state.guide_stream.frame_counter(),
            0,
            "the guide stream advanced a frame with nobody watching"
        );
        assert!(
            state.guide_stream.get_latest_raw_frame().await.is_none(),
            "an unwatched guide stream published a rendered frame"
        );
        for tier in JpegTier::all() {
            assert!(
                state.guide_stream.get_tier_jpeg(tier, 1).is_none(),
                "{tier:?} was encoded with nobody watching the guide stream"
            );
        }
    }

    /// The other half: with a viewer registered the same loop does render and publish,
    /// so the gate is throttling on demand rather than being permanently shut.
    #[tokio::test]
    async fn a_watched_guide_stream_renders_every_frame() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);

        let _viewer = TierClientGuard::new(
            Arc::clone(&state.guide_stream),
            StreamKind::Jpeg,
            JpegTier::Hd1080,
        );

        let captured = drive_guide_loop(&state, 3).await;

        assert_eq!(captured, 3);
        assert_eq!(
            state.guide_stream.frame_counter(),
            3,
            "a watched guide stream must publish every frame it captures"
        );
        assert!(state.guide_stream.get_latest_raw_frame().await.is_some());
    }

    /// A viewer on the guide stream must not make the *main* stream produce anything —
    /// the two are independent producers, and a guide frame advancing the main counter
    /// would invalidate the imaging camera's cached tiers on every guide exposure.
    #[tokio::test]
    async fn the_guide_loop_never_touches_the_main_stream() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);

        let _viewer = TierClientGuard::new(
            Arc::clone(&state.guide_stream),
            StreamKind::Jpeg,
            JpegTier::Hd1080,
        );
        drive_guide_loop(&state, 2).await;

        assert_eq!(state.main_stream.frame_counter(), 0);
        assert!(state.main_stream.get_latest_raw_frame().await.is_none());
    }

    /// Raw saving sits above both early exits: an unwatched guide camera with no solve
    /// target still writes the subs the user asked for.
    #[tokio::test]
    async fn guide_raw_frames_are_saved_even_when_nothing_is_watching() {
        let (state, disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        std::thread::spawn(move || disk_writer.run());

        state.settings.write().await.raw_frame_saving.guide = true;

        drive_guide_loop(&state, 3).await;

        let dir = guide_session_dir(&state)
            .await
            .expect("no guide raw-frame directory was opened");
        assert!(
            dir.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("-guide"),
            "guide frames went to {dir:?}, which does not name the guide mode"
        );

        // The writer runs on its own thread; give it a moment to drain the queue.
        for _ in 0..50 {
            let written = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
            if written >= 3 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("guide raw frames never reached {dir:?}");
    }

    /// A dropout rejoins the folder the interrupted session was filling, so the resumed
    /// run has to carry on its numbering: the writer names files `frame_{:06}.fits`, and
    /// restarting at 1 wrote straight over the frames already in there.
    #[tokio::test]
    async fn a_resumed_guide_session_appends_to_the_folder_it_rejoined() {
        let (state, disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        std::thread::spawn(move || disk_writer.run());

        state.settings.write().await.raw_frame_saving.guide = true;

        drive_guide_loop(&state, 3).await;
        let dir = guide_session_dir(&state)
            .await
            .expect("no guide raw-frame directory was opened");
        wait_for_files(&dir, 3).await;

        // Second run, resuming the way `spawn_loop` does — from what the slot parked.
        let resume = state
            .slot(CameraRole::Guide)
            .raw_session
            .read()
            .await
            .clone();
        assert_eq!(
            resume.as_ref().map(|r| r.next_frame),
            Some(4),
            "the interrupted run must park the number to carry on from"
        );

        let stop = Arc::new(AtomicBool::new(false));
        let (camera, _captured) = CountingCamera::new(2, Arc::clone(&stop));
        let info = guide_camera_info();
        {
            let state = Arc::clone(&state);
            let rt = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                run(&state, &info, Box::new(camera), &stop, resume, &rt);
            })
            .await
            .expect("guide loop panicked");
        }
        wait_for_files(&dir, 5).await;

        for n in 1..=5u64 {
            let path = dir.join(format!("frame_{n:06}.fits"));
            assert!(path.exists(), "{path:?} is missing — the resume overwrote it");
        }
    }

    /// Drive `frames` exposures and hand back everything the loop asked of the camera.
    async fn drive_and_log(state: &Arc<AppState>, frames: usize) -> Arc<StdMutex<DriveLog>> {
        let stop = Arc::new(AtomicBool::new(false));
        let (camera, _captured, log) = CountingCamera::with_log(frames, Arc::clone(&stop));
        let info = guide_camera_info();
        let state = Arc::clone(state);
        let rt = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            run(&state, &info, Box::new(camera), &stop, None, &rt);
        })
        .await
        .expect("guide loop panicked");

        log
    }

    /// `CaptureConfig` has no dew-heater field, so a queued call is the only way the
    /// switch can reach a camera whose handle the loop holds for the whole connection.
    #[tokio::test]
    async fn the_loop_runs_hardware_calls_queued_against_its_slot() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);

        state.slot(CameraRole::Guide).queue_op(CameraOp::SetDewHeater {
            enabled: true,
            power: 65,
        });

        let log = drive_and_log(&state, 2).await;

        assert_eq!(log.lock().unwrap().dew_heater, vec![(true, 65)]);
        assert!(
            state.slot(CameraRole::Guide).drain_ops().is_empty(),
            "the loop must consume what it applied"
        );
    }

    /// The monitor is the imaging camera's status source and cannot check out a handle
    /// the guide loop never returns, so the loop reports for itself — otherwise the
    /// guide camera's temperature readout never updates for the whole session.
    #[tokio::test]
    async fn the_loop_publishes_its_own_status_samples() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);

        let log = drive_and_log(&state, 2).await;

        assert!(
            log.lock().unwrap().status_reads >= 1,
            "the guide loop never read the sensor"
        );
        let status = state
            .get_camera_status("Mock Guide Camera")
            .await
            .expect("no status was broadcast for the guide camera");
        assert_eq!(status.temperature_c, 20.0);
    }

    /// The setpoint the loop commands has to walk toward the target at
    /// `RAMP_RATE_C_PER_MIN`, not jump to it. Pushing the final target per frame is how
    /// the 5 °C/min limit stopped applying to one of the two cameras on the rig.
    #[tokio::test]
    async fn the_cooler_setpoint_is_ramped_not_snapped() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);
        {
            let mut settings = state.settings.write().await;
            settings.guide_camera.cooler_enabled = true;
            settings.guide_camera.target_temp_c = Some(-15.0);
        }

        let log = drive_and_log(&state, 1).await;

        let first = log.lock().unwrap().setpoints[0];
        // The sensor reports 20 °C, so a ramp starts there. Under the test-time rate the
        // first step may already reach the target; what must never happen is the loop
        // commanding the target before it has looked at the sensor at all.
        assert!(
            first.is_some_and(|sp| (-15.0..=20.0).contains(&sp)),
            "expected a setpoint between the sensor and the target, got {first:?}"
        );
    }

    /// The arithmetic, at a controlled clock: `RAMP_RATE_C_PER_MIN` is shadowed in test
    /// builds, so the step is derived from the constant rather than hard-coded, and the
    /// interval is short enough that even the test rate cannot reach the target.
    #[test]
    fn the_ramp_walks_from_the_sensor_toward_the_target() {
        use crate::server::camera_session::RAMP_RATE_C_PER_MIN;

        let profile = CameraCaptureProfile {
            cooler_enabled: true,
            target_temp_c: Some(-15.0),
            ..Default::default()
        };
        let mut cooler = GuideCooler::default();
        let start = std::time::Instant::now();

        // First call installs the ramp at the sensor temperature; it has not moved yet.
        assert_eq!(cooler.setpoint(&profile, Some(20.0), start), Some(20.0));

        let dt = Duration::from_millis(100);
        let expected = 20.0 - dt.as_secs_f64() * RAMP_RATE_C_PER_MIN / 60.0;
        assert!(
            expected > -15.0,
            "the test interval must stay short of the target to prove a ramp"
        );
        let stepped = cooler
            .setpoint(&profile, Some(20.0), start + dt)
            .expect("a ramping cooler must command a setpoint");
        assert_eq!(stepped, expected.round());
        assert!(stepped < 20.0 && stepped > -15.0, "got {stepped}");
    }

    /// Moving the target restarts the ramp from where the sensor actually is, rather
    /// than continuing from a setpoint aimed somewhere else.
    #[test]
    fn a_new_target_restarts_the_ramp_at_the_sensor() {
        let mut profile = CameraCaptureProfile {
            cooler_enabled: true,
            target_temp_c: Some(-15.0),
            ..Default::default()
        };
        let mut cooler = GuideCooler::default();
        let t0 = std::time::Instant::now();
        cooler.setpoint(&profile, Some(20.0), t0);
        cooler.setpoint(&profile, Some(5.0), t0 + Duration::from_secs(60));

        profile.target_temp_c = Some(-5.0);
        assert_eq!(
            cooler.setpoint(&profile, Some(5.0), t0 + Duration::from_secs(61)),
            Some(5.0),
            "the ramp should have restarted at the sensor, not resumed"
        );
    }

    /// Nothing to ramp: the config's own target stands.
    #[test]
    fn a_cooler_with_nothing_to_ramp_leaves_the_config_alone() {
        let mut cooler = GuideCooler::default();
        let now = std::time::Instant::now();

        let off = CameraCaptureProfile {
            cooler_enabled: false,
            target_temp_c: Some(-15.0),
            ..Default::default()
        };
        assert_eq!(cooler.setpoint(&off, Some(20.0), now), None);

        let no_target = CameraCaptureProfile {
            cooler_enabled: true,
            target_temp_c: None,
            ..Default::default()
        };
        assert_eq!(cooler.setpoint(&no_target, Some(20.0), now), None);
    }

    /// Fast mode is the switch a user flips to accept a snap, and the UI warns while it
    /// is on — so the loop must leave that config's target alone.
    #[tokio::test]
    async fn fast_mode_leaves_the_target_alone() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);
        {
            let mut settings = state.settings.write().await;
            settings.guide_camera.cooler_enabled = true;
            settings.guide_camera.target_temp_c = Some(-15.0);
            settings.guide_camera.cooler_fast_mode = true;
        }

        let log = drive_and_log(&state, 1).await;

        assert_eq!(log.lock().unwrap().setpoints[0], Some(-15.0));
    }

    /// The directory the guide loop is currently filling, if any.
    async fn guide_session_dir(state: &Arc<AppState>) -> Option<std::path::PathBuf> {
        state
            .slot(CameraRole::Guide)
            .raw_session
            .read()
            .await
            .as_ref()
            .map(|r| r.dir.clone())
    }

    async fn wait_for_files(dir: &std::path::Path, want: usize) {
        for _ in 0..100 {
            if std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0) >= want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!(
            "only {} of {want} files reached {dir:?}",
            std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
        );
    }

    /// Off by default, and it must stay a separate decision from the imaging switches.
    #[tokio::test]
    async fn guide_raw_saving_is_off_unless_asked_for() {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);

        // Saving every imaging mode must not implicitly save guide frames.
        {
            let mut settings = state.settings.write().await;
            settings.raw_frame_saving.live_view = true;
            settings.raw_frame_saving.wanderer = true;
            settings.raw_frame_saving.stacking = true;
        }

        drive_guide_loop(&state, 2).await;

        assert!(
            guide_session_dir(&state).await.is_none(),
            "a guide session was opened without the guide switch"
        );
    }
}
