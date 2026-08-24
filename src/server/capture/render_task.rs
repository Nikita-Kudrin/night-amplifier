use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::frame::Frame;
use crate::server::encoding::{encode_rgb8_jpeg_bounded, encode_rgb8_lz4_chunked};
use crate::server::state::{AppState, JpegTier};
use crate::telemetry::metrics as telemetry_metrics;

use super::channel::StackedFrame;
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
    rt: tokio::runtime::Handle,
) {
    debug!("Render task started");

    let max_chunks = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);

    while let Ok(msg) = render_rx.recv() {
        // Drain to the latest frame — skip intermediate stacked states
        let (latest, skipped) = drain_to_latest(msg, &render_rx);
        telemetry_metrics::record_frames_skipped_to_latest(skipped);

        let StackedFrame {
            mut display_frame,
            was_stacked,
            frame_number,
            settings,
        } = latest;

        let _iter_span =
            tracing::info_span!("render_iteration", frame_number, was_stacked,).entered();

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

        // Process frame through unified render pipeline
        let (pipeline_config, stretch_result) = {
            let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::Render);
            match pipeline::process_preview_frame(Arc::make_mut(&mut display_frame), &settings) {
                Ok(res) => res,
                Err(e) => {
                    state.send_error(format!("Preview processing failed: {}", e));
                    continue;
                }
            }
        };

        // Use max parallel chunks for live view, single chunk during stacking
        let chunk_count = if was_stacked { 1 } else { max_chunks };

        let ready_frame = Arc::new(crate::server::state::RenderReadyFrame {
            linear_frame: display_frame,
            pipeline_config,
            stretch_result,
        });

        let raw_frame = ready_frame;
        rt.block_on(state.set_latest_raw_frame(Arc::clone(&raw_frame)));

        // Claim the counter before encoding so every payload below is filed
        // under the same frame, then wake clients once they are all in place.
        let counter = state.begin_frame();

        // Deduplicate f32 -> u8 conversion for native resolution streams.
        // If the frame fits in 4K, LZ4's 4K limit and JPEG's native resolution are identical.
        let fits_in_4k =
            raw_frame.linear_frame.width() <= 3840 && raw_frame.linear_frame.height() <= 2160;
        let lz4_active = state.lz4_clients.load(std::sync::atomic::Ordering::SeqCst) > 0;
        let mut original_jpeg_active = false;
        for tier in JpegTier::all() {
            if state.jpeg_tier_client_count(tier) > 0
                && !tier.would_downsample(
                    raw_frame.linear_frame.width(),
                    raw_frame.linear_frame.height(),
                )
            {
                original_jpeg_active = true;
                break;
            }
        }

        let mut shared_native_rgb8: Option<Arc<(Vec<u8>, u32, u32)>> = None;
        if fits_in_4k && (lz4_active || original_jpeg_active) {
            let _span = tracing::info_span!("frame_to_rgb8_shared").entered();
            if let Ok(data) =
                crate::server::encoding::frame_to_rgb8_downsampled(&raw_frame, 3840, 2160)
            {
                shared_native_rgb8 = Some(Arc::new(data));
            }
        }

        if lz4_active {
            // Encode frame as RGB8+LZ4 for streaming
            let encode_result = {
                let _encode_span = tracing::info_span!("encode_rgb8_lz4").entered();
                let _timer =
                    telemetry_metrics::time_stage(telemetry_metrics::FrameStage::EncodeLz4);
                if let Some(ref shared) = shared_native_rgb8 {
                    crate::server::encoding::encode_rgb8_lz4_chunked_from_u8(
                        &shared.0,
                        shared.1,
                        shared.2,
                        chunk_count,
                    )
                } else {
                    crate::server::encoding::encode_rgb8_lz4_chunked(&raw_frame, chunk_count)
                }
            };
            match encode_result {
                Ok(encoded_data) => rt.block_on(state.set_latest_frame(encoded_data)),
                Err(e) => {
                    rt.block_on(state.frame_rejected(format!("RGB8+LZ4 encoding failed: {}", e)))
                }
            }
        }

        encode_jpeg_tiers(&state, &raw_frame, counter, shared_native_rgb8);

        state.publish_frame();
    }

    debug!("Render task ended");
}

/// Encode one JPEG per resolution tier that has clients.
///
/// Tiers whose bounding box does not shrink the frame all produce the same
/// native-resolution payload, so the first such encode is shared with the rest.
/// For sensors below 4K that collapses `Uhd2160` and `Original` into a single
/// encode.
fn encode_jpeg_tiers(
    state: &AppState,
    frame: &crate::server::state::RenderReadyFrame,
    counter: u64,
    shared_native_rgb8: Option<Arc<(Vec<u8>, u32, u32)>>,
) {
    let _span = tracing::info_span!("encode_jpeg_tiers").entered();
    let mut native: Option<bytes::Bytes> = None;

    for tier in JpegTier::all() {
        if state.jpeg_tier_client_count(tier) == 0 {
            continue;
        }

        let downsamples =
            tier.would_downsample(frame.linear_frame.width(), frame.linear_frame.height());
        if !downsamples {
            if let Some(shared) = &native {
                state.set_tier_jpeg(tier, counter, shared.clone());
                continue;
            }
        }

        let (max_w, max_h) = tier.bounding_box();
        let started = std::time::Instant::now();
        let encoded = if let (false, Some(data)) = (downsamples, shared_native_rgb8.as_ref()) {
            crate::server::encoding::encode_rgb8_jpeg_bounded_from_u8(&data.0, data.1, data.2)
        } else {
            crate::server::encoding::encode_rgb8_jpeg_bounded(frame, max_w, max_h)
        };
        telemetry_metrics::record_jpeg_encode_ms(
            tier.metric_label(),
            started.elapsed().as_secs_f64() * 1000.0,
        );
        match encoded {
            Ok(encoded) => {
                let stored = state.set_tier_jpeg(tier, counter, encoded);
                if !downsamples {
                    native = Some(stored);
                }
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
            was_stacked: true,
            frame_number: 1,
            settings,
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
            was_stacked: false,
            frame_number: 0,
            settings: settings.clone(),
        };

        // Queue additional frames
        for n in 0..3 {
            let msg = super::StackedFrame {
                display_frame: Arc::new(crate::frame::Frame::zeros(4, 4, 3).unwrap()),
                was_stacked: false,
                frame_number: n + 1,
                settings: settings.clone(),
            };
            tx.send(msg).unwrap();
        }
        // Last frame is the "latest"
        let last = super::StackedFrame {
            display_frame: Arc::new(crate::frame::Frame::filled(4, 4, 3, 1.0).unwrap()),
            was_stacked: true,
            frame_number: 4,
            settings: settings.clone(),
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
            was_stacked: false,
            frame_number: 1,
            settings: CaptureSettings::default(),
        })
        .unwrap();
        // Closing the channel lets run_render_task exit after this frame.
        drop(tx);

        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || super::run_render_task(state, rx, rt))
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
            was_stacked: false,
            frame_number: 1,
            settings: passthrough_settings(),
        })
        .unwrap();
        drop(tx);

        let rt = tokio::runtime::Handle::current();
        let task_state = Arc::clone(&state);
        tokio::task::spawn_blocking(move || super::run_render_task(task_state, rx, rt))
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
        assert_eq!(state.lz4_clients.load(Ordering::SeqCst), 0);

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
            super::encode_jpeg_tiers(&state_clone, &to_ready_frame(&frame), 1, None)
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
            super::encode_jpeg_tiers(&state_clone, &to_ready_frame(&frame), 1, None)
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
            super::encode_jpeg_tiers(&state_clone, &to_ready_frame(&frame), 1, None)
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
