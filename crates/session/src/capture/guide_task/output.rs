//! Where a guide frame goes besides the solver: the live view and the eyepiece pages,
//! while somebody watches, and the guide camera's own raw-frame session.

use std::sync::Arc;
use tracing::{info, warn};

use crate::capture::analysis::{AnalysisContext, PreviewAnalysis};
use crate::capture::stream_encoding::{
    encode_watched, ConversionCache, FailureReports, StreamResolutions, StreamTarget,
};
use crate::capture::{pipeline, stage_config, storage};
use crate::state::{
    AppState, CameraRole, CaptureMode, CaptureSettings, ConnectedCameraInfo, RawSessionResume,
};
use night_amplifier_core::disk_writer::{OpenSession, WritingSessionType};
use night_amplifier_core::render::display::RenderReadyFrame;

/// Run the same preview pipeline the main camera gets, then publish and encode.
///
/// Only reached with a viewer connected — see the module doc.
pub(super) fn render_and_publish(
    state: &AppState,
    frame: Arc<night_amplifier_core::frame::Frame>,
    settings: &CaptureSettings,
    conversions: &mut ConversionCache,
    failures: &mut FailureReports,
    analysis: &mut PreviewAnalysis,
) {
    let _span = tracing::info_span!("guide_render").entered();

    let settings = stage_config::guide_render_settings(settings.clone());
    let mut display_frame = frame;
    let rendered = match pipeline::process_preview_frame_with_analysis(
        Arc::make_mut(&mut display_frame),
        &settings,
        &state.plugins,
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
        pipeline_config: rendered.pipeline_config,
        stretch_result: rendered.stretch_result,
        // A guide frame is a single sub: nothing accumulated it, so there is no
        // per-pixel noise to report.
        noise: None,
    });

    let stream = &state.guide_stream;
    stream.set_latest_raw_frame(Arc::clone(&ready));
    // The live resolutions, not `settings`: that snapshot predates the exposure.
    let resolutions = StreamResolutions::of(&state.settings.snapshot());
    let counter = stream.begin_frame();

    let target = StreamTarget::guide(stream);
    for error in encode_watched(target, &ready, counter, resolutions, conversions, failures) {
        warn!(error = %error, "Guide stream encoding failed");
    }
    stream.publish_frame();
}

/// The guide camera's own raw-frame session, opened lazily and closed when the switch
/// goes off, so toggling *Save raw frames → Guide camera* mid-session takes effect on
/// the next frame the way the imaging switches do.
pub(super) struct GuideDiskSession {
    session: Option<OpenSession>,
    /// A directory a dropout left behind, rejoined by the first frame that needs one,
    /// together with the number that run had reached.
    resume: Option<RawSessionResume>,
    /// Opening failed and the observer was told; every frame retries, quietly.
    open_failing: bool,
}

impl GuideDiskSession {
    pub(super) fn new(resume: Option<RawSessionResume>) -> Self {
        Self {
            session: None,
            resume,
            open_failing: false,
        }
    }

    /// The number the first frame of this run should take.
    ///
    /// A resumed run continues its predecessor's numbering: the writer names files
    /// `frame_{:06}.fits`, so restarting at 1 wrote straight over the frames the
    /// interrupted run had already saved into the directory being rejoined.
    pub(super) fn first_frame_number(&self) -> u64 {
        self.resume.as_ref().map_or(1, |r| r.next_frame.max(1))
    }

    pub(super) fn write(
        &mut self,
        state: &Arc<AppState>,
        settings: &CaptureSettings,
        raw: &Arc<night_amplifier_core::camera::RawFrame>,
        frame_number: u64,
        camera_info: &ConnectedCameraInfo,
    ) {
        if !settings.saves_guide_raw_frames() {
            if self.session.take().is_some() {
                state.slot(CameraRole::Guide).set_raw_session(None);
            }
            return;
        }

        // The master flag is set from the main capture's `initialize_capture_session`,
        // which never runs when only a guide camera is connected.
        state.disk_writer.set_enabled(true);

        if self.session.is_none() {
            // The resume stays parked until it opens: taken eagerly, one failed attempt
            // sent every retry to a fresh folder instead of the one being rejoined.
            let opened = match self.resume.as_ref() {
                Some(resume) => state
                    .disk_writer
                    .reopen_session(resume.dir.clone(), WritingSessionType::IndividualFrames),
                None => state.disk_writer.create_session(
                    WritingSessionType::IndividualFrames,
                    CaptureMode::Guide.session_dir_suffix(),
                ),
            };
            match opened {
                Ok(session) => {
                    info!(dir = ?session.dir, "Guide raw-frame session opened");
                    self.resume = None;
                    self.open_failing = false;
                    self.session = Some(session);
                }
                Err(e) => {
                    if !std::mem::replace(&mut self.open_failing, true) {
                        warn!(error = %e, "Could not open a guide raw-frame directory");
                        state.send_error(format!("Guide frames are not being saved: {e}"));
                    }
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
            state.slot(CameraRole::Guide).set_raw_session(Some(resume));
        }

        let metadata = storage::guide_frame_metadata(state, settings, camera_info, frame_number);
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
