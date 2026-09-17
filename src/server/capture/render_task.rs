use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::server::state::{AppState, RenderReadyFrame, Resolution, StreamKind};
use crate::telemetry::metrics as telemetry_metrics;

use super::analysis::{AnalysisContext, PreviewAnalysis};
use super::channel::{QueueDepth, StackedFrame};
use super::pipeline;
use super::stream_encoding::{
    encode_jpeg, encode_lossless, ConversionCache, FailureReports, StreamResolutions,
};

/// Preview rendering and encoding, on a dedicated OS thread. Drains the channel to
/// the latest frame for UI responsiveness, runs `process_preview_frame()`, then
/// encodes each watched family once (the lossless LZ4 blob, the JPEG) here rather than
/// per client, so N clients cost one encode and WebSocket handlers just copy a pointer.
/// LZ4 chunk count is dynamic: max parallelism in live view, single chunk while
/// stacking (to yield cores to it).
pub fn run_render_task(
    state: Arc<AppState>,
    render_rx: mpsc::Receiver<StackedFrame>,
    render_depth: QueueDepth,
    rt: tokio::runtime::Handle,
) {
    debug!("Render task started");

    let max_chunks = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);

    // Outlives the loop: its per-frame conversions are cleared each iteration,
    // but the denoise buffers behind them are the whole point and are kept.
    let mut conversions = ConversionCache::default();
    let mut failures = FailureReports::default();

    // Also outlives the loop, for the same reason and with the same ownership: the
    // white-balance coefficients, background model and image statistics describe the
    // stack, not the frame, so a frame of the same stack can be served the previous
    // frame's measurements. `analysis` decides that per frame; see `capture::analysis`.
    let mut analysis = PreviewAnalysis::new();

    // Same lifetime, and for a stronger reason: the factor decides what the tone-curve
    // solve measures, so it must not move under a viewer. See `SessionBinFactor`.
    let mut session_bin = SessionBinFactor::default();

    while let Ok(msg) = render_rx.recv() {
        // Drain to the latest frame — skip intermediate stacked states
        let (latest, skipped) = drain_to_latest(msg, &render_rx);
        telemetry_metrics::record_frames_skipped_to_latest(skipped);

        // Once per message taken off the channel, not once per iteration: a drained
        // frame is still one the stacking task no longer has to account for, and
        // undercounting here would leave the depth permanently above zero and stop it
        // ever building another display frame.
        for _ in 0..=skipped {
            render_depth.taken();
        }

        let StackedFrame {
            mut display_frame,
            showing_stack,
            was_stacked,
            frame_number,
            settings,
            stack_depth,
        } = latest;

        let _iter_span =
            tracing::info_span!("render_iteration", frame_number, showing_stack, was_stacked,)
                .entered();

        // The preview pipeline mutates in place. `make_mut` hands back the buffer
        // untouched when we hold the only handle (the usual case); a live second
        // holder (disk saving, an in-flight solve) forces the copy instead of paying
        // it unconditionally. Staying inside the `Arc` also lets the rendered frame
        // reach `latest_raw_frame` without re-wrapping.
        //
        // This log predicts `make_mut`'s decision rather than observing it, so a
        // holder dropping in between is a false positive — harmless, since no handle
        // can be *acquired* once the frame is here, so silence still proves no copy.
        if Arc::get_mut(&mut display_frame).is_none() {
            debug!("Preview frame still shared, copying before render");
        }

        // Bin before the pipeline touches the frame, rather than after. See
        // `preview_bin_factor` for why this is an integer and what it costs when it
        // comes out 1, and `SessionBinFactor` for why it is not re-derived per frame.
        let bin = session_bin.resolve(
            display_frame.width(),
            display_frame.height(),
            settings.preview_resolution,
        );
        if bin > 1 {
            let _span = tracing::info_span!("preview_bin", factor = bin).entered();
            match display_frame.downsample(bin) {
                Ok(binned) => display_frame = Arc::new(binned),
                // Not fatal: the pipeline is perfectly capable of running at sensor
                // resolution, it is just slower. A failure here must not cost the frame.
                Err(e) => warn!(error = %e, factor = bin, "Preview binning failed, rendering at full resolution"),
            }
        }

        // Process frame through unified render pipeline
        let (pipeline_config, stretch_result) = {
            let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::Render);
            match pipeline::process_preview_frame_with_analysis(
                Arc::make_mut(&mut display_frame),
                &settings,
                AnalysisContext {
                    showing_stack,
                    stack_depth,
                },
                &mut analysis,
            ) {
                Ok(res) => res,
                Err(e) => {
                    state.send_error(format!("Preview processing failed: {}", e));
                    continue;
                }
            }
        };

        // Use max parallel chunks for live view, single chunk during stacking.
        // Keyed on what is being displayed, not on whether this frame joined the
        // stack: a rejected frame still leaves the slow-moving stack on screen.
        let chunk_count = if showing_stack { 1 } else { max_chunks };

        let ready_frame = Arc::new(crate::server::state::RenderReadyFrame {
            linear_frame: display_frame,
            pipeline_config,
            stretch_result,
        });

        let raw_frame = ready_frame;

        // An async lock taken by `rt.block_on` from a thread that is not a tokio worker,
        // so the cost is a park/unpark round trip rather than a lock acquisition. This
        // and `publish_frame()` at the end of the loop are most of the 6.7 ms of
        // `render_iteration` self time that had no name. Only this end is spanned —
        // `publish_frame` is on the far side of the encode, so one span cannot cover
        // both without also covering the work between them.
        let (counter, resolutions) = {
            let _span = tracing::info_span!("publish_state").entered();
            // The live resolutions ride the same round trip rather than adding one.
            let resolutions = rt.block_on(async {
                state.main_stream.set_latest_raw_frame(Arc::clone(&raw_frame)).await;
                StreamResolutions::of(&*state.settings.read().await)
            });
            // Claim the counter before encoding so every payload below is filed
            // under the same frame, then wake clients once they are all in place.
            (state.main_stream.begin_frame(), resolutions)
        };

        encode_payloads(
            &state,
            &raw_frame,
            counter,
            resolutions,
            &mut conversions,
            &mut failures,
            chunk_count,
        );

        state.main_stream.publish_frame();
    }

    debug!("Render task ended");
}

/// Encode both families of the imaging stream at their settings' resolutions.
///
/// A failure is a display problem, not a camera one: it is logged and reported to the UI,
/// and deliberately *not* passed to `frame_rejected`, which would count it towards the
/// session's rejection rate — the signal that decides whether the camera still responds.
fn encode_payloads(
    state: &AppState,
    frame: &RenderReadyFrame,
    counter: u64,
    resolutions: StreamResolutions,
    conversions: &mut ConversionCache,
    failures: &mut FailureReports,
    chunk_count: usize,
) {
    // One RGB8 conversion per distinct output size: with both families at the same
    // resolution the denoised conversion (~5x the encode) happens once.
    conversions.begin_frame();
    let stream = &state.main_stream;

    let lossless = if stream.viewer_count(StreamKind::Lossless) > 0 {
        let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::EncodeLz4);
        encode_lossless(stream, frame, counter, conversions, resolutions.lossless, chunk_count)
    } else {
        Ok(())
    };
    let jpeg = encode_jpeg(stream, frame, counter, conversions, resolutions.jpeg);

    let results = [(StreamKind::Lossless, lossless), (StreamKind::Jpeg, jpeg)];
    for error in results.into_iter().filter_map(|(kind, result)| failures.to_report(kind, result)) {
        warn!(error = %error, "Stream payload encoding failed");
        state.send_error(error);
    }
}

/// The preview bin factor for one capture session, resolved once and held — not
/// recomputed per frame. It used to be: called every iteration against the largest
/// connected client's bounding box, flipping between 1 and 2 whenever the client set
/// crossed a 2x boundary. Binning isn't neutral: the tone curve solves from median
/// and MAD, and a 2x2 box average halves MAD, moving the black point and curve with
/// it (measured: solved `scale_lut` gained 25.7% at the 1% input point) — every
/// viewer saw the jump, not just the arriving client.
///
/// So the factor is a session property (sensor shape + [`Resolution`], both
/// observer-controlled), held until one changes. Shape stays part of the key because
/// hardware binning/ROI/mono-colour swaps reshape the frame mid-session and already
/// reset the stack — a deliberate observer act, the same class of event as starting
/// a session, logged for that reason.
#[derive(Default)]
struct SessionBinFactor {
    resolved: Option<((usize, usize), Resolution)>,
    factor: usize,
}

impl SessionBinFactor {
    fn resolve(
        &mut self,
        width: usize,
        height: usize,
        resolution: Resolution,
    ) -> usize {
        let key = ((width, height), resolution);
        if self.resolved == Some(key) {
            return self.factor;
        }

        let factor = match resolution.target_box() {
            Some(target) => preview_bin_factor(width, height, target),
            None => 1,
        };
        // `info`, not `debug`: this fires once per session and on the two changes the
        // observer makes deliberately, and it re-grades the picture when it moves.
        tracing::info!(
            width,
            height,
            ?resolution,
            factor,
            previous = ?self.resolved,
            "Preview bin factor resolved"
        );
        self.resolved = Some(key);
        self.factor = factor;
        factor
    }
}

/// Largest integer bin that still leaves the preview the pixels [`Resolution`]
/// asks for. Background neutralisation, subtraction, SCNR and black-point all walk
/// every sample before `frame_to_rgb8_downsampled` throws away what the stream doesn't
/// need (76% of a 3008² frame streamed at 1440p) — the same argument AGENTS.md
/// makes for running denoisers at stream resolution applies to every stage above them.
///
/// Integer, not the exact box: `Frame::downsample` stays an exact box average with
/// no resampling phase to get wrong, leaving the encoder's fractional resample to
/// land the final size — conservative, never smaller than the largest requested box,
/// 1 whenever halving would undershoot it. `target` comes from
/// [`Resolution::target_box`], never the connected clients (see
/// [`SessionBinFactor`]); `Native` has no box and never reaches here, making
/// "no downsampling" the default rather than something to protect.
///
/// All-or-nothing at the **2x boundary**: a 3008² sensor at 4K bins by 1
/// (saves nothing); at 1440p/1080p it bins by 2 and the whole pipeline runs on
/// a quarter of the samples (phones, tablets, eyepiece view). Capped at 4 — past that
/// the background grid is estimated from too few samples to mean anything, and
/// nothing served is under 1080 anyway.
///
/// Bounds against the **output size**, not the bounding box: a 3008² frame in a
/// 2560x1440 box comes out 1440x1440 (short edge binds, aspect preserved), so
/// comparing against the raw box would refuse to bin a square sensor at any resolution.
/// `encoding::output_dimensions` is the one copy of that arithmetic, kept here to
/// agree with the encoder.
fn preview_bin_factor(width: usize, height: usize, target: (u32, u32)) -> usize {
    const MAX_BIN: usize = 4;
    if target.0 == 0 || target.1 == 0 {
        return 1;
    }

    let (out_w, out_h) =
        crate::server::encoding::output_dimensions(width, height, target.0, target.1);

    (1..=MAX_BIN)
        .rev()
        .find(|&f| width / f >= out_w && height / f >= out_h)
        .unwrap_or(1)
}

/// Drain the receiver, keeping only the latest message.
///
/// Consumes all immediately available messages and returns the most recent one
/// along with how many were discarded, so a backed-up render stage is visible
/// in telemetry. This ensures the UI always shows the freshest available frame.
fn drain_to_latest(
    initial: StackedFrame,
    rx: &mpsc::Receiver<StackedFrame>,
) -> (StackedFrame, u64) {
    let mut latest = initial;
    let mut skipped = 0;
    while let Ok(newer) = rx.try_recv() {
        latest = newer;
        skipped += 1;
    }
    (latest, skipped)
}

#[cfg(test)]
mod tests {
    use super::{SessionBinFactor};
    use crate::server::state::Resolution;
    use std::sync::Arc;

    /// The default must bin nothing, whatever the sensor and whoever is connected.
    /// Native streaming promises the full frame, and this frame is also what
    /// `set_latest_raw_frame` stores for on-demand encodes — none of which upscale. The
    /// predecessor chose the factor from the connected client set against a 4K box,
    /// downsampling both cases: an unbinned ASI294MM Pro (8288x5644) fit the 4K box with
    /// room for a halving; an IMX411-class sensor lost a factor of four.
    #[test]
    fn the_default_preview_resolution_bins_nothing() {
        let mut session = SessionBinFactor::default();
        for (w, h) in [(8288, 5644), (14192, 10640), (3008, 3008), (2712, 1538)] {
            assert_eq!(
                session.resolve(w, h, crate::server::state::DEFAULT_PREVIEW_RESOLUTION),
                1,
                "{w}x{h} was binned at the default preview resolution"
            );
        }
    }

    /// The factor is a property of the session, not of who is watching.
    ///
    /// This is the whole point of `SessionBinFactor`: the tone curve is solved from the
    /// binned frame, so a factor that tracked the connected client set would re-solve
    /// the curve for *every* viewer whenever one of them opened or closed a tab.
    #[test]
    fn the_bin_factor_does_not_move_while_the_session_runs() {
        use crate::server::state::{AppState, StreamKind, ViewerGuard};
        use std::sync::Arc;

        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let mut session = SessionBinFactor::default();
        let first = session.resolve(3008, 3008, Resolution::Qhd1440);
        assert_eq!(first, 2, "a 3008x3008 sensor halves into the 1440 box");

        // A phone arrives, then an eyepiece, then both leave.
        {
            let _phone = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Jpeg);
            assert_eq!(session.resolve(3008, 3008, Resolution::Qhd1440), first);
            let _eyepiece = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Lossless);
            assert_eq!(session.resolve(3008, 3008, Resolution::Qhd1440), first);
        }
        assert_eq!(session.resolve(3008, 3008, Resolution::Qhd1440), first);
    }

    /// The two things the observer *does* control still re-solve it.
    #[test]
    fn a_shape_or_setting_change_re_resolves_the_bin_factor() {
        let mut session = SessionBinFactor::default();
        assert_eq!(session.resolve(3008, 3008, Resolution::Native), 1);
        assert_eq!(
            session.resolve(3008, 3008, Resolution::Hd1080),
            2,
            "the observer asked for a cheaper preview"
        );
        assert_eq!(
            session.resolve(1504, 1504, Resolution::Hd1080),
            1,
            "hardware binning already halved the frame; binning again would go under 1080"
        );
    }

    /// How far binning moves the tone curve, as a number rather than an assumption.
    /// Binning isn't neutral: the stretch solves from median/MAD, and a 2x2 box average
    /// roughly halves MAD, moving the black point and curve — why [`SessionBinFactor`]
    /// holds the factor for the session rather than tracking connected clients (a phone
    /// opening a tab would re-grade the picture for everyone). The bound is
    /// deliberately loose: it exists to keep the number tracked and catch the shift
    /// *growing*, not to claim it's small — tighten it if a change makes the solve less
    /// resolution-sensitive.
    #[test]
    fn binning_moves_the_tone_curve_by_a_bounded_amount() {
        use crate::frame::Frame;
        use crate::server::capture::pipeline::process_preview_frame;
        use crate::server::state::CaptureSettings;

        // A light-pollution gradient with read noise — the shape the solver is for.
        let (w, h) = (1200usize, 1200usize);
        let mut seed = 0x51A2_B3C4u32;
        let mut rand = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        let mut frame = Frame::zeros(w, h, 3).unwrap();
        for y in 0..h {
            for x in 0..w {
                let grad = 0.02 + 0.06 * (x as f32 / w as f32) + 0.03 * (y as f32 / h as f32);
                for c in 0..3 {
                    frame.set_pixel(x, y, c, grad + (rand() - 0.5) * 0.02);
                }
            }
        }
        // Stars, so the solve has real structure above the sky rather than only noise.
        for _ in 0..400 {
            let cx = (rand() * (w - 16) as f32) as usize + 8;
            let cy = (rand() * (h - 16) as f32) as usize + 8;
            let peak = 0.2 + rand() * 0.7;
            for dy in 0..7usize {
                for dx in 0..7usize {
                    let (x, y) = (cx + dx - 3, cy + dy - 3);
                    let d2 = (dx as f32 - 3.0).powi(2) + (dy as f32 - 3.0).powi(2);
                    let v = peak * (-d2 / 2.6).exp();
                    for c in 0..3 {
                        let cur = frame.get_pixel(x, y, c);
                        frame.set_pixel(x, y, c, (cur + v).min(1.0));
                    }
                }
            }
        }

        let settings = CaptureSettings::default();
        let mut full = frame.clone();
        let mut binned = frame.downsample(2).unwrap();

        let (_, full_stretch) = process_preview_frame(&mut full, &settings).unwrap();
        let (_, binned_stretch) = process_preview_frame(&mut binned, &settings).unwrap();

        let full_lut = full_stretch.expect("full-resolution stretch").scale_lut;
        let binned_lut = binned_stretch.expect("binned stretch").scale_lut;
        assert_eq!(full_lut.len(), binned_lut.len());

        let worst = full_lut
            .iter()
            .zip(binned_lut.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);

        // Measured on this fixture: gain 2.7351 -> 3.4380 at the 1 % input point
        // (+25.7 %) and 5.0621 -> 5.8775 at 10 % (+16.1 %), with the curve unchanged by
        // mid-tones. Shadows are exactly where an EAA viewer is looking.
        let sample = |lut: &[f32], t: f32| lut[((lut.len() - 1) as f32 * t) as usize];
        eprintln!(
            "curve at 1%/10%/50%: full {:.4}/{:.4}/{:.4}  binned {:.4}/{:.4}/{:.4}  worst {worst:.4}",
            sample(&full_lut, 0.01),
            sample(&full_lut, 0.10),
            sample(&full_lut, 0.50),
            sample(&binned_lut, 0.01),
            sample(&binned_lut, 0.10),
            sample(&binned_lut, 0.50),
        );

        assert!(
            worst > 0.1,
            "binning no longer moves the tone curve ({worst:.4}) — if the solve has been \
             made resolution-independent, tighten this bound rather than deleting it"
        );
        assert!(
            worst < 1.5,
            "binning moved the tone curve by {worst:.4}, up from the 1.0640 measured when \
             SessionBinFactor was introduced"
        );
    }

    /// Binning must never take the frame below what a client asked for — that is the
    /// whole safety property, and every other case is an optimisation on top of it.
    #[test]
    fn preview_binning_never_goes_under_the_requested_box() {
        use super::preview_bin_factor;

        // IMX533, 3008x3008. The 4K box does not survive a halving (1504 < 2160), so
        // it must bin by 1 — this is the traced configuration, and it saves nothing.
        assert_eq!(preview_bin_factor(3008, 3008, (3840, 2160)), 1);
        assert_eq!(preview_bin_factor(3008, 3008, (2560, 2160)), 1);

        // A 1440p or 1080p box leaves room for one halving: 1504 clears both.
        assert_eq!(preview_bin_factor(3008, 3008, (2560, 1440)), 2);
        assert_eq!(preview_bin_factor(3008, 3008, (1920, 1080)), 2);

        // IMX464, 2712x1538 — the short edge is what binds. 1356x769 is under 1080, so
        // even the smallest box cannot bin this sensor.
        assert_eq!(preview_bin_factor(2712, 1538, (1920, 1080)), 1);
    }

    /// The exact boundary, in both directions, on a frame where one more pixel decides
    /// it. An off-by-one here is a preview served below the resolution its client asked
    /// for, which no test downstream of the encoder would catch.
    #[test]
    fn preview_binning_is_exact_at_the_boundary() {
        use super::preview_bin_factor;

        assert_eq!(
            preview_bin_factor(2160, 2160, (1080, 1080)),
            2,
            "2160 / 2 == 1080 exactly, which still covers the box"
        );
        assert_eq!(
            preview_bin_factor(2159, 2159, (1080, 1080)),
            1,
            "one pixel short of twice the box must not bin"
        );
    }

    /// A frame far larger than anything asked for still stops at the cap, and a
    /// degenerate box cannot produce a divide-by-zero or an unbounded factor.
    #[test]
    fn preview_binning_is_bounded() {
        use super::preview_bin_factor;

        assert_eq!(preview_bin_factor(16_000, 16_000, (1920, 1080)), 4);
        assert_eq!(preview_bin_factor(3008, 3008, (0, 0)), 1);
    }

    #[test]
    fn test_drain_to_latest_single_frame() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<super::StackedFrame>(8);

        let settings = crate::server::state::CaptureSettings::default();
        let frame = crate::frame::Frame::zeros(4, 4, 3).unwrap();
        let msg = super::StackedFrame {
            display_frame: std::sync::Arc::new(frame),
            showing_stack: true,
            was_stacked: true,
            frame_number: 1,
            settings,
            stack_depth: 0,
        };

        // No extra messages — should return initial
        let (result, skipped) = super::drain_to_latest(msg, &rx);
        assert!(result.was_stacked);
        assert_eq!(skipped, 0);
        drop(tx);
    }

    #[test]
    fn test_drain_to_latest_multiple_frames() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<super::StackedFrame>(8);

        let settings = crate::server::state::CaptureSettings::default();
        let initial = super::StackedFrame {
            display_frame: Arc::new(crate::frame::Frame::zeros(4, 4, 3).unwrap()),
            showing_stack: false,
            was_stacked: false,
            frame_number: 0,
            settings: settings.clone(),
            stack_depth: 0,
        };

        // Queue additional frames
        for n in 0..3 {
            let msg = super::StackedFrame {
                display_frame: Arc::new(crate::frame::Frame::zeros(4, 4, 3).unwrap()),
                showing_stack: false,
                was_stacked: false,
                frame_number: n + 1,
                settings: settings.clone(),
                stack_depth: 0,
            };
            tx.send(msg).unwrap();
        }
        // Last frame is the "latest"
        let last = super::StackedFrame {
            display_frame: Arc::new(crate::frame::Frame::filled(4, 4, 3, 1.0).unwrap()),
            showing_stack: true,
            was_stacked: true,
            frame_number: 4,
            settings: settings.clone(),
            stack_depth: 0,
        };
        tx.send(last).unwrap();

        let (result, skipped) = super::drain_to_latest(initial, &rx);
        // Should get the last frame (was_stacked = true, filled with 1.0)
        assert!(result.was_stacked);
        assert!(result.display_frame.get_pixel(0, 0, 0) > 0.9);
        // The initial frame plus the three queued ones were all superseded.
        assert_eq!(skipped, 4);
        drop(tx);
    }
}

#[cfg(test)]
#[path = "render_task_stream_tests.rs"]
mod stream_tests;
