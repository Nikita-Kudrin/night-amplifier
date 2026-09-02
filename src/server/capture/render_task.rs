use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::server::state::{AppState, JpegTier, PreviewResolution};
use crate::telemetry::metrics as telemetry_metrics;

use super::analysis::{AnalysisContext, PreviewAnalysis};
use super::channel::{QueueDepth, StackedFrame};
use super::pipeline;

/// Preview rendering and encoding, running on a dedicated OS thread.
///
/// Drains the channel to the latest frame to keep the UI responsive, runs
/// `process_preview_frame()`, then encodes every payload the connected clients
/// need — the LZ4 blob for the lossless stream and one JPEG per active
/// resolution tier. Encoding all of it here (rather than per client) means N
/// clients on the same tier cost one encode, and WebSocket handlers only copy a
/// pointer to the socket.
///
/// LZ4 chunk count is dynamic:
/// - Live view (not stacking): max parallelism for responsive UI
/// - Stacking active: single chunk to yield CPU cores to the stacking pipeline
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

        // The preview pipeline mutates in place. `make_mut` hands back the
        // buffer untouched when we hold the only handle — the usual case, since
        // the capture thread has moved on and plate solving is only spawned when
        // it can actually run. A live second holder (raw-frame disk saving, an
        // in-flight solve) forces the copy we would otherwise have paid
        // unconditionally. Staying inside the `Arc` also means the rendered
        // frame reaches `latest_raw_frame` without being re-wrapped.
        //
        // The log predicts `make_mut`'s decision rather than observing it, so a
        // holder that drops in between turns it into a false positive. That is
        // the harmless direction: no handle can be *acquired* once the frame is
        // here, so silence still proves the frame was not copied.
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
        let counter = {
            let _span = tracing::info_span!("publish_state").entered();
            rt.block_on(state.set_latest_raw_frame(Arc::clone(&raw_frame)));
            // Claim the counter before encoding so every payload below is filed
            // under the same frame, then wake clients once they are all in place.
            state.begin_frame()
        };

        // One RGB8 conversion per distinct output size, shared by every payload
        // that resolves to it. Since tier 2 the conversion carries the
        // denoisers and costs several times the encode it feeds, so this is
        // where the frame's time goes if two clients are watching.
        conversions.begin_frame();

        if state.lossless_client_count() > 0 {
            let (max_w, max_h) = state.lossless_target_box();
            let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::EncodeLz4);
            match conversions.get(&raw_frame, max_w, max_h) {
                Some(rgb) => {
                    let _encode_span = tracing::info_span!("encode_rgb8_lz4").entered();
                    match crate::server::encoding::encode_rgb8_lz4_chunked_from_u8(
                        &rgb.0,
                        rgb.1,
                        rgb.2,
                        chunk_count,
                    ) {
                        Ok(encoded_data) => rt.block_on(state.set_latest_frame(encoded_data)),
                        Err(e) => rt.block_on(
                            state.frame_rejected(format!("RGB8+LZ4 encoding failed: {}", e)),
                        ),
                    }
                }
                None => rt.block_on(
                    state.frame_rejected("RGB8 conversion failed for the lossless stream".into()),
                ),
            }
        }

        encode_jpeg_tiers(&state, &raw_frame, counter, &mut conversions);

        state.publish_frame();
    }

    debug!("Render task ended");
}

/// The preview bin factor for one capture session, resolved once and held.
///
/// # Why this is not recomputed per frame
///
/// It used to be: `preview_bin_factor` was called every iteration against the largest
/// bounding box any *connected client* had asked for, so the factor flipped between 1
/// and 2 whenever the client set crossed a 2x boundary — a phone opening the page, a tab
/// closing. Binning is not neutral to the analysis that follows it: the tone curve is
/// solved from the frame's median and MAD, and a 2x2 box average cuts MAD by about half,
/// which moves the black point and the whole curve with it. Measured on a 1200x1200 sky
/// with 400 stars, the solved `scale_lut` gained 25.7 % at the 1 % input point and 16.1 %
/// at 10 %. Every connected viewer saw that jump, not just the client that arrived.
///
/// So the factor is a property of the session: it comes from the sensor shape and
/// [`PreviewResolution`], both of which the observer controls, and it is held until one
/// of them actually changes.
///
/// # Why the shape is still part of the key
///
/// Hardware binning, an ROI change or a mono/colour swap all reshape the frame mid
/// session, and a factor solved for the old shape would be meaningless against the new
/// one. Those are deliberate acts by the observer, not other people's browser tabs, and
/// they already reset the stack — so re-solving there is the same class of event as
/// starting a session. It is logged for that reason.
#[derive(Default)]
struct SessionBinFactor {
    resolved: Option<((usize, usize), PreviewResolution)>,
    factor: usize,
}

impl SessionBinFactor {
    fn resolve(
        &mut self,
        width: usize,
        height: usize,
        resolution: PreviewResolution,
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

/// Largest integer bin that still leaves the preview the pixels [`PreviewResolution`]
/// asks for.
///
/// # Why the preview pipeline should not run at sensor resolution
///
/// Background neutralisation, background subtraction, SCNR and the black-point pass all
/// walk every sample of the frame, and `frame_to_rgb8_downsampled` then throws away
/// whatever the client's tier does not need — 76 % of a 3008x3008 frame for a client on
/// the 1440 tier. This is the same argument `AGENTS.md` already makes for running the
/// denoisers at stream resolution rather than sensor resolution; every stage above them
/// has the property too.
///
/// # Why an integer factor and not the exact tier
///
/// `Frame::downsample` bins by an integer, which keeps it an exact box average with no
/// resampling phase to get wrong, and leaves the encoder's existing fractional resample
/// to land the final size. So this is deliberately conservative: it never produces a
/// frame smaller than the largest box a client asked for, and returns 1 whenever
/// halving would go under it.
///
/// `target` comes from [`PreviewResolution::target_box`], never from the connected
/// clients — see [`SessionBinFactor`]. [`PreviewResolution::Native`] has no box at all
/// and never reaches here, which is what makes "no downsampling" the default rather than
/// a property the client-set arithmetic has to be careful not to break.
///
/// The consequence is that the win is **all or nothing at the 2x boundary**. A
/// 3008x3008 sensor with a client on the 2160 tier bins by 1 and saves nothing; the same
/// sensor with a client on the 1440 or 1080 tier bins by 2 and the whole preview
/// pipeline runs on a quarter of the samples. Phones, tablets and the eyepiece view are
/// the second case.
///
/// Capped at 4: past that the analysis stages are estimating a background from so few
/// samples that the grid is no longer meaningful, and nothing the pipeline serves is
/// under 1080 anyway.
///
/// The bound is the **output size** the encoder will produce, not the bounding box
/// itself: a 3008x3008 frame fitted into the 2560x1440 box comes out 1440x1440, because
/// the short edge binds and the aspect ratio is preserved. Comparing against the raw box
/// would refuse to bin a square sensor for any tier. `encoding::output_dimensions` is
/// the one copy of that arithmetic — the same reason `ConversionCache` keys on it — so
/// asking it here is what keeps this decision and the encoder's from disagreeing.
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

/// The RGB8 conversions one frame needs, at most one per distinct output size.
///
/// Two payloads whose clients asked for different bounding boxes are the same
/// conversion whenever those boxes resolve to the same output size — a 2712x1538
/// sensor fitted into the 4K box and into no box at all are both 2712x1538. This
/// generalises the "share the native buffer between LZ4 and the tiers that do not
/// downsample" special case it replaces; native size is simply the case where
/// every box resolves to the frame's own dimensions.
///
/// A `Vec` rather than a map: there are at most five payloads per frame, and a
/// linear scan over five pairs beats hashing them.
#[derive(Default)]
struct ConversionCache {
    entries: Vec<((usize, usize), Arc<(Vec<u8>, u32, u32)>)>,
    /// The denoisers' working buffers, reused for the life of the render thread.
    /// Kept here rather than in a thread-local so nothing else in the process
    /// can strand 75 MB behind a pooled worker.
    scratch: crate::render::denoise::DenoiseScratch,
}

impl ConversionCache {
    /// Drop the previous frame's conversions, keeping the buffers that produced
    /// them.
    fn begin_frame(&mut self) {
        self.entries.clear();
    }

    /// The RGB8 buffer for a bounding box, converting only if nothing already
    /// built has the same output size.
    fn get(
        &mut self,
        frame: &crate::server::state::RenderReadyFrame,
        max_w: u32,
        max_h: u32,
    ) -> Option<Arc<(Vec<u8>, u32, u32)>> {
        let key = crate::server::encoding::output_dimensions(
            frame.linear_frame.width(),
            frame.linear_frame.height(),
            max_w,
            max_h,
        );
        if let Some((_, data)) = self.entries.iter().find(|(k, _)| *k == key) {
            return Some(Arc::clone(data));
        }

        let _span = tracing::info_span!("frame_to_rgb8", width = key.0, height = key.1).entered();
        match crate::server::encoding::frame_to_rgb8_downsampled_with(
            frame,
            max_w,
            max_h,
            &mut self.scratch,
        ) {
            Ok(data) => {
                let data = Arc::new(data);
                self.entries.push((key, Arc::clone(&data)));
                Some(data)
            }
            Err(e) => {
                // Logged, not raised: the LZ4 caller turns a missing conversion
                // into a rejected frame and a JPEG tier simply goes unencoded,
                // so raising here too would report one failure twice.
                warn!(error = %e, width = key.0, height = key.1, "RGB8 conversion failed");
                None
            }
        }
    }

    /// How many conversions were actually performed, for tests that need to see
    /// that sharing happened rather than infer it from a payload.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Encode one JPEG per resolution tier that has clients.
///
/// Tiers that resolve to the same output size produce the same bytes, so the
/// first one encodes and the rest are handed the same `Bytes`. For a sub-4K
/// sensor that collapses `Uhd2160` and `Original` into one encode *and* one
/// conversion.
fn encode_jpeg_tiers(
    state: &AppState,
    frame: &crate::server::state::RenderReadyFrame,
    counter: u64,
    conversions: &mut ConversionCache,
) {
    let _span = tracing::info_span!("encode_jpeg_tiers").entered();
    let mut encoded: Vec<((usize, usize), bytes::Bytes)> = Vec::new();

    for tier in JpegTier::all() {
        if state.jpeg_tier_client_count(tier) == 0 {
            continue;
        }

        let (max_w, max_h) = tier.bounding_box();
        let key = crate::server::encoding::output_dimensions(
            frame.linear_frame.width(),
            frame.linear_frame.height(),
            max_w,
            max_h,
        );

        if let Some((_, payload)) = encoded.iter().find(|(k, _)| *k == key) {
            state.set_tier_jpeg(tier, counter, payload.clone());
            continue;
        }

        let Some(rgb) = conversions.get(frame, max_w, max_h) else {
            continue;
        };

        let started = std::time::Instant::now();
        let result = crate::server::encoding::encode_rgb8_jpeg_bounded_from_u8(&rgb.0, rgb.1, rgb.2);
        telemetry_metrics::record_jpeg_encode_ms(
            tier.metric_label(),
            started.elapsed().as_secs_f64() * 1000.0,
        );
        match result {
            Ok(payload) => {
                let stored = state.set_tier_jpeg(tier, counter, payload);
                encoded.push((key, stored));
            }
            Err(e) => warn!(?tier, error = %e, "JPEG encoding failed for tier"),
        }
    }
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
    use crate::server::capture::channel::QueueDepth;
    use crate::server::state::PreviewResolution;

    /// The tests here drive the render task directly rather than through the capture
    /// pipeline, so nothing on the other end of the depth counter is running. A fresh
    /// counter per call is the honest stand-in: it starts at zero and nothing reads it.
    fn no_depth() -> QueueDepth {
        QueueDepth::default()
    }

    /// The default must bin nothing, whatever the sensor and whoever is connected.
    ///
    /// [`JpegTier::Original`] is documented as "native sensor resolution, no
    /// downsampling", and the frame this produces is also what `set_latest_raw_frame`
    /// stores — `ws::payload_for_new_client` encodes the first payload of every arriving
    /// client straight out of it, and `encode_rgb8_jpeg_bounded` does not upscale.
    ///
    /// The predecessor chose the factor from the connected client set against a box
    /// clamped to `JPEG_MAX_BOUNDING_BOX`, which downsampled both cases: an ASI294MM Pro
    /// unbinned is 8288x5644, fits the 4K box at 3172x2160, and so had room for a
    /// halving; an IMX411-class sensor lost a factor of four.
    #[test]
    fn the_default_preview_resolution_bins_nothing() {
        let mut session = SessionBinFactor::default();
        for (w, h) in [(8288, 5644), (14192, 10640), (3008, 3008), (2712, 1538)] {
            assert_eq!(
                session.resolve(w, h, PreviewResolution::default()),
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
        use crate::server::state::{AppState, JpegTier, StreamKind, TierClientGuard};
        use std::sync::Arc;

        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let mut session = SessionBinFactor::default();
        let first = session.resolve(3008, 3008, PreviewResolution::Qhd1440);
        assert_eq!(first, 2, "a 3008x3008 sensor halves into the 1440 box");

        // A phone arrives, then a 4K browser, then both leave.
        {
            let _phone = TierClientGuard::new(Arc::clone(&state), StreamKind::Jpeg, JpegTier::Hd1080);
            assert_eq!(session.resolve(3008, 3008, PreviewResolution::Qhd1440), first);
            let _desktop =
                TierClientGuard::new(Arc::clone(&state), StreamKind::Jpeg, JpegTier::Original);
            assert_eq!(session.resolve(3008, 3008, PreviewResolution::Qhd1440), first);
        }
        assert_eq!(session.resolve(3008, 3008, PreviewResolution::Qhd1440), first);
    }

    /// The two things the observer *does* control still re-solve it.
    #[test]
    fn a_shape_or_setting_change_re_resolves_the_bin_factor() {
        let mut session = SessionBinFactor::default();
        assert_eq!(session.resolve(3008, 3008, PreviewResolution::Native), 1);
        assert_eq!(
            session.resolve(3008, 3008, PreviewResolution::Hd1080),
            2,
            "the observer asked for a cheaper preview"
        );
        assert_eq!(
            session.resolve(1504, 1504, PreviewResolution::Hd1080),
            1,
            "hardware binning already halved the frame; binning again would go under 1080"
        );
    }

    /// How far binning moves the tone curve, as a number rather than an assumption.
    ///
    /// Binning is not neutral to the analysis that follows it. The stretch is solved from
    /// the frame's median and MAD, and a 2x2 box average cuts MAD by roughly half, so the
    /// black point and the whole curve land somewhere else. This is why
    /// [`SessionBinFactor`] holds the factor for the session instead of tracking the
    /// connected client set: at the measured size, a phone opening a tab would have
    /// re-graded the picture for everyone watching.
    ///
    /// The bound is deliberately loose — it exists to keep the number in the repository
    /// and to catch the shift *growing*, not to claim it is small. A change that makes
    /// the solve less sensitive to resolution should tighten it.
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

        // IMX533, 3008x3008. The 2160 tier does not survive a halving (1504 < 2160), so
        // it must bin by 1 — this is the traced configuration, and it saves nothing.
        assert_eq!(preview_bin_factor(3008, 3008, (3840, 2160)), 1);
        assert_eq!(preview_bin_factor(3008, 3008, (2560, 2160)), 1);

        // A 1440 or 1080 client leaves room for one halving: 1504 clears both.
        assert_eq!(preview_bin_factor(3008, 3008, (2560, 1440)), 2);
        assert_eq!(preview_bin_factor(3008, 3008, (1920, 1080)), 2);

        // IMX464, 2712x1538 — the short edge is what binds. 1356x769 is under 1080, so
        // even the smallest tier cannot bin this sensor.
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

    fn to_ready_frame(frame: &crate::frame::Frame) -> crate::server::state::RenderReadyFrame {
        crate::server::state::RenderReadyFrame {
            linear_frame: std::sync::Arc::new(frame.clone()),
            pipeline_config: crate::render::RenderPipelineConfig::default(),
            stretch_result: None,
        }
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

    use crate::server::state::{AppState, CaptureSettings, JpegTier};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// IMX464 sensor dimensions: below 4K, so `Uhd2160` and `Original` produce
    /// identical output.
    const IMX464: (usize, usize) = (2712, 1538);

    /// Render a single frame through `run_render_task` and return the state.
    async fn render_one_frame(state: Arc<AppState>, frame: crate::frame::Frame) {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(super::StackedFrame {
            display_frame: std::sync::Arc::new(frame),
            showing_stack: false,
            was_stacked: false,
            frame_number: 1,
            settings: CaptureSettings::default(),
            stack_depth: 0,
        })
        .unwrap();
        // Closing the channel lets run_render_task exit after this frame.
        drop(tx);

        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || super::run_render_task(state, rx, no_depth(), rt))
            .await
            .unwrap();
    }

    /// Settings that make the preview pipeline a no-op, so a test observes only
    /// how the frame buffer is handled and not what the render stages do to it.
    fn passthrough_settings() -> CaptureSettings {
        CaptureSettings {
            auto_stretch: false,
            background_subtraction: false,
            saturation_boost: false,
            ..CaptureSettings::default()
        }
    }

    /// Run one frame through the render task, returning the pixel buffer address
    /// the frame ended up at. `extra_holder` simulates another stage (disk
    /// saving, an in-flight plate solve) still holding the frame.
    async fn render_and_report_buffer_addr(
        frame: crate::frame::Frame,
        keep_extra_handle: bool,
    ) -> (usize, usize) {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let shared = Arc::new(frame);
        let addr_in = shared.data().as_ptr() as usize;
        let extra_handle = keep_extra_handle.then(|| Arc::clone(&shared));

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(super::StackedFrame {
            display_frame: shared,
            showing_stack: false,
            was_stacked: false,
            frame_number: 1,
            settings: passthrough_settings(),
            stack_depth: 0,
        })
        .unwrap();
        drop(tx);

        let rt = tokio::runtime::Handle::current();
        let task_state = Arc::clone(&state);
        tokio::task::spawn_blocking(move || super::run_render_task(task_state, rx, no_depth(), rt))
            .await
            .unwrap();
        drop(extra_handle);

        let addr_out = state
            .get_latest_raw_frame()
            .await
            .expect("render task published a frame")
            .linear_frame
            .data()
            .as_ptr() as usize;
        (addr_in, addr_out)
    }

    /// The whole point of `StackedFrame` carrying an `Arc`: when the render task
    /// holds the only handle it must reuse the buffer, not copy 50 MB.
    #[tokio::test]
    async fn test_render_task_reuses_uniquely_held_frame_buffer() {
        let (addr_in, addr_out) =
            render_and_report_buffer_addr(crate::frame::Frame::zeros(64, 48, 3).unwrap(), false)
                .await;
        assert_eq!(
            addr_in, addr_out,
            "uniquely-held frame was copied instead of moved into the render pipeline"
        );
    }

    /// When another stage still holds the frame, the render task must fall back
    /// to a copy rather than mutating a buffer someone else is reading.
    #[tokio::test]
    async fn test_render_task_copies_frame_still_held_elsewhere() {
        let (addr_in, addr_out) =
            render_and_report_buffer_addr(crate::frame::Frame::zeros(64, 48, 3).unwrap(), true)
                .await;
        assert_ne!(
            addr_in, addr_out,
            "shared frame must be copied before the preview pipeline mutates it"
        );
    }

    #[tokio::test]
    async fn test_run_render_task_notifies_frame_ready_with_no_lz4_clients() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        // Ensure no LZ4 clients are connected (this is the default, but let's be explicit)
        assert_eq!(state.lossless_client_count(), 0);

        let initial_counter = state.frame_counter.load(Ordering::SeqCst);
        render_one_frame(
            Arc::clone(&state),
            crate::frame::Frame::zeros(10, 10, 3).unwrap(),
        )
        .await;

        // The frame_counter MUST have increased so JPEG clients wake up
        let new_counter = state.frame_counter.load(Ordering::SeqCst);
        assert_eq!(
            new_counter,
            initial_counter + 1,
            "frame_counter did not increment when lz4_clients was 0"
        );
    }

    /// Dimensions of the LZ4 payload the render task published, read out of the
    /// SA09 header.
    fn lz4_payload_dimensions(payload: &[u8]) -> (u32, u32) {
        (
            u32::from_le_bytes(payload[4..8].try_into().unwrap()),
            u32::from_le_bytes(payload[8..12].try_into().unwrap()),
        )
    }

    /// Render one frame with a lossless client registered against `tier`, and
    /// report the size the published payload came out at.
    async fn lossless_payload_size_for(tier: JpegTier, width: usize, height: usize) -> (u32, u32) {
        use crate::server::state::{StreamKind, TierClientGuard};

        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let _guard = TierClientGuard::new(Arc::clone(&state), StreamKind::Lossless, tier);

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(super::StackedFrame {
            display_frame: std::sync::Arc::new(
                crate::frame::Frame::filled(width, height, 3, 0.25).unwrap(),
            ),
            showing_stack: false,
            was_stacked: false,
            frame_number: 1,
            settings: passthrough_settings(),
            stack_depth: 0,
        })
        .unwrap();
        drop(tx);

        let rt = tokio::runtime::Handle::current();
        let task_state = Arc::clone(&state);
        tokio::task::spawn_blocking(move || super::run_render_task(task_state, rx, no_depth(), rt))
            .await
            .unwrap();

        let payload = state
            .get_latest_frame()
            .await
            .expect("render task published no lossless payload");
        lz4_payload_dimensions(&payload)
    }

    /// The point of T0.1: a 1440p eyepiece must receive a 1440p frame, not a
    /// near-native one for the GPU to minify. An IMX533 frame is square and
    /// 3008 on a side, so the 1440 box takes it to 1440x1440.
    #[tokio::test]
    async fn lossless_stream_encodes_into_the_clients_tier() {
        assert_eq!(
            lossless_payload_size_for(JpegTier::Qhd1440, 3008, 3008).await,
            (1440, 1440)
        );
    }

    /// A client on a smaller tier gets a correspondingly smaller frame — the
    /// bandwidth half of the same change.
    #[tokio::test]
    async fn lossless_stream_follows_a_smaller_tier_down() {
        assert_eq!(
            lossless_payload_size_for(JpegTier::Hd1080, 3008, 3008).await,
            (1080, 1080)
        );
    }

    /// A client that never reports a viewport keeps the historical 4K cap.
    ///
    /// `handle_eyepiece_quality` registers `JpegTier::LOSSLESS_DEFAULT` for the
    /// life of the connection, so this models the real handler rather than an
    /// unreachable zero-tier state: an older frontend, or one whose first report
    /// is still in flight, must not be *downgraded* by a change meant to help it.
    #[tokio::test]
    async fn lossless_stream_without_a_reported_viewport_keeps_the_4k_cap() {
        use crate::server::state::{StreamKind, TierClientGuard};

        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        let _guard = TierClientGuard::new(
            Arc::clone(&state),
            StreamKind::Lossless,
            JpegTier::LOSSLESS_DEFAULT,
        );

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(super::StackedFrame {
            display_frame: std::sync::Arc::new(
                crate::frame::Frame::filled(3008, 3008, 3, 0.25).unwrap(),
            ),
            showing_stack: false,
            was_stacked: false,
            frame_number: 1,
            settings: passthrough_settings(),
            stack_depth: 0,
        })
        .unwrap();
        drop(tx);

        let rt = tokio::runtime::Handle::current();
        let task_state = Arc::clone(&state);
        tokio::task::spawn_blocking(move || super::run_render_task(task_state, rx, no_depth(), rt))
            .await
            .unwrap();

        let payload = state.get_latest_frame().await.expect("no payload");
        assert_eq!(lz4_payload_dimensions(&payload), (2160, 2160));
    }

    /// The conversion cache shares a buffer between the lossless and JPEG
    /// encoders only when both resolve to the same output size. A lossless
    /// client on a smaller tier must get its own conversion — serving it the
    /// native one would silently ship a native-size payload and undo the whole
    /// change.
    #[tokio::test]
    async fn lossless_downsample_is_not_served_from_a_native_conversion() {
        use crate::server::state::{StreamKind, TierClientGuard};

        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        // A JPEG client on a non-downsampling tier puts a native-size buffer in
        // the cache, so the lossless path has something to wrongly reuse.
        state.jpeg_tier_clients[JpegTier::Uhd2160 as usize].store(1, Ordering::SeqCst);
        let _guard =
            TierClientGuard::new(Arc::clone(&state), StreamKind::Lossless, JpegTier::Hd1080);

        let (tx, rx) = std::sync::mpsc::channel();
        // Below 4K, so `fits_in_4k` holds and the shared buffer is native-size.
        tx.send(super::StackedFrame {
            display_frame: std::sync::Arc::new(
                crate::frame::Frame::filled(IMX464.0, IMX464.1, 3, 0.25).unwrap(),
            ),
            showing_stack: false,
            was_stacked: false,
            frame_number: 1,
            settings: passthrough_settings(),
            stack_depth: 0,
        })
        .unwrap();
        drop(tx);

        let rt = tokio::runtime::Handle::current();
        let task_state = Arc::clone(&state);
        tokio::task::spawn_blocking(move || super::run_render_task(task_state, rx, no_depth(), rt))
            .await
            .unwrap();

        let payload = state.get_latest_frame().await.expect("no payload");
        let (w, h) = lz4_payload_dimensions(&payload);
        assert!(
            w < IMX464.0 as u32 && h < IMX464.1 as u32,
            "lossless payload came out at {w}x{h}, i.e. the shared native buffer \
             was reused instead of downsampling to the client's tier"
        );

        // The JPEG tier that did want native size must still have got it.
        let counter = state.frame_counter.load(Ordering::SeqCst);
        let jpeg = state
            .get_tier_jpeg(JpegTier::Uhd2160, counter)
            .expect("Uhd2160 payload missing");
        assert_eq!(
            u32::from_le_bytes(jpeg[4..8].try_into().unwrap()),
            IMX464.0 as u32
        );
    }

    /// What the cache exists for: two payloads whose clients asked for different
    /// bounding boxes but that resolve to the same output size are one
    /// conversion. Since tier 2 that conversion carries the denoisers and costs
    /// several times the encode it feeds, so doing it twice is the difference
    /// between one stream and two on a Pi.
    #[test]
    fn conversion_cache_shares_one_buffer_across_equivalent_boxes() {
        let frame =
            to_ready_frame(&crate::frame::Frame::filled(IMX464.0, IMX464.1, 3, 0.25).unwrap());
        let mut cache = super::ConversionCache::default();

        // An IMX464 frame fits both the 4K box and no box at all, so `Uhd2160`
        // and `Original` are the same conversion.
        let uhd = cache.get(&frame, 3840, 2160).expect("conversion");
        let original = cache
            .get(&frame, u32::MAX, u32::MAX)
            .expect("conversion");
        assert_eq!(cache.len(), 1, "equivalent boxes converted twice");
        assert!(Arc::ptr_eq(&uhd, &original));

        // A box that genuinely shrinks the frame is a different conversion.
        let hd = cache.get(&frame, 1920, 1080).expect("conversion");
        assert_eq!(cache.len(), 2);
        assert_ne!((hd.1, hd.2), (uhd.1, uhd.2));

        // ...and asking for it again is free.
        cache.get(&frame, 1920, 1080).expect("conversion");
        assert_eq!(cache.len(), 2);
    }

    #[tokio::test]
    async fn test_render_task_skips_jpeg_when_no_clients() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        render_one_frame(
            Arc::clone(&state),
            crate::frame::Frame::zeros(10, 10, 3).unwrap(),
        )
        .await;

        let counter = state.frame_counter.load(Ordering::SeqCst);
        for tier in JpegTier::all() {
            assert!(
                state.get_tier_jpeg(tier, counter).is_none(),
                "{tier:?} was encoded with no clients watching it"
            );
        }
    }

    #[tokio::test]
    async fn test_render_task_encodes_jpeg_for_active_tier() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        state.jpeg_tier_clients[JpegTier::Hd1080 as usize].store(1, Ordering::SeqCst);

        render_one_frame(
            Arc::clone(&state),
            crate::frame::Frame::zeros(10, 10, 3).unwrap(),
        )
        .await;

        let counter = state.frame_counter.load(Ordering::SeqCst);
        let payload = state
            .get_tier_jpeg(JpegTier::Hd1080, counter)
            .expect("Hd1080 payload missing");
        let magic = u32::from_le_bytes(payload[0..4].try_into().unwrap());
        assert_eq!(magic, crate::server::encoding::JPEG_MAGIC);
        assert!(state.get_tier_jpeg(JpegTier::Qhd1440, counter).is_none());
    }

    #[tokio::test]
    async fn test_render_task_deduplicates_equivalent_tiers() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        state.jpeg_tier_clients[JpegTier::Uhd2160 as usize].store(1, Ordering::SeqCst);
        state.jpeg_tier_clients[JpegTier::Original as usize].store(1, Ordering::SeqCst);

        // Encode directly: the preview pipeline is irrelevant here and costly at
        // sensor resolution.
        let (width, height) = IMX464;
        let frame = Arc::new(crate::frame::Frame::filled(width, height, 3, 0.25).unwrap());
        let state_clone = Arc::clone(&state);
        tokio::task::spawn_blocking(move || {
            super::encode_jpeg_tiers(
                &state_clone,
                &to_ready_frame(&frame),
                1,
                &mut super::ConversionCache::default(),
            )
        })
        .await
        .unwrap();

        let uhd = state.get_tier_jpeg(JpegTier::Uhd2160, 1).unwrap();
        let original = state.get_tier_jpeg(JpegTier::Original, 1).unwrap();
        assert_eq!(
            uhd.as_ptr(),
            original.as_ptr(),
            "equivalent tiers should share a single encode"
        );
    }

    /// The dedup case clients can actually reach: `Original` is unselectable, so
    /// sharing only ever happens between the three clamped tiers, and only when
    /// the frame fits inside all of them — e.g. IMX464 at bin 2.
    #[tokio::test]
    async fn test_render_task_shares_one_encode_across_client_reachable_tiers() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        for tier in [JpegTier::Hd1080, JpegTier::Qhd1440, JpegTier::Uhd2160] {
            state.jpeg_tier_clients[tier as usize].store(1, Ordering::SeqCst);
        }

        let (width, height) = (IMX464.0 / 2, IMX464.1 / 2);
        let frame = Arc::new(crate::frame::Frame::filled(width, height, 3, 0.25).unwrap());
        let state_clone = Arc::clone(&state);
        tokio::task::spawn_blocking(move || {
            super::encode_jpeg_tiers(
                &state_clone,
                &to_ready_frame(&frame),
                1,
                &mut super::ConversionCache::default(),
            )
        })
        .await
        .unwrap();

        let hd = state.get_tier_jpeg(JpegTier::Hd1080, 1).unwrap();
        for tier in [JpegTier::Qhd1440, JpegTier::Uhd2160] {
            let other = state.get_tier_jpeg(tier, 1).unwrap();
            assert_eq!(
                hd.as_ptr(),
                other.as_ptr(),
                "{tier:?} re-encoded needlessly"
            );
        }
        assert_eq!(
            u32::from_le_bytes(hd[4..8].try_into().unwrap()),
            width as u32
        );
        // Nobody selected Original, so it must not have been encoded at all.
        assert!(state.get_tier_jpeg(JpegTier::Original, 1).is_none());
    }

    #[tokio::test]
    async fn test_render_task_encodes_downsampled_tier_separately() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        state.jpeg_tier_clients[JpegTier::Hd1080 as usize].store(1, Ordering::SeqCst);
        state.jpeg_tier_clients[JpegTier::Original as usize].store(1, Ordering::SeqCst);

        // 2000x1200 is wider than the 1080p box, so Hd1080 must not reuse the
        // native-resolution payload.
        let frame = Arc::new(crate::frame::Frame::filled(2000, 1200, 3, 0.25).unwrap());
        let state_clone = Arc::clone(&state);
        tokio::task::spawn_blocking(move || {
            super::encode_jpeg_tiers(
                &state_clone,
                &to_ready_frame(&frame),
                1,
                &mut super::ConversionCache::default(),
            )
        })
        .await
        .unwrap();

        let hd = state.get_tier_jpeg(JpegTier::Hd1080, 1).unwrap();
        let original = state.get_tier_jpeg(JpegTier::Original, 1).unwrap();
        assert_ne!(hd.as_ptr(), original.as_ptr());

        let hd_width = u32::from_le_bytes(hd[4..8].try_into().unwrap());
        let original_width = u32::from_le_bytes(original[4..8].try_into().unwrap());
        assert_eq!(hd_width, 1800); // 2000x1200 fitted into 1920x1080
        assert_eq!(original_width, 2000);
    }
}
