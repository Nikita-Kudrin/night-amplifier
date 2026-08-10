use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::frame::Frame;
use crate::server::encoding::{encode_rgb8_jpeg_bounded, encode_rgb8_lz4_chunked};
use crate::server::state::{AppState, JpegTier};

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
        let latest = drain_to_latest(msg, &render_rx);

        let StackedFrame {
            mut display_frame,
            was_stacked,
            frame_number,
            settings,
        } = latest;

        let _iter_span =
            tracing::info_span!("render_iteration", frame_number, was_stacked,).entered();

        // Process frame through unified render pipeline
        if let Err(e) = pipeline::process_preview_frame(&mut display_frame, &settings) {
            state.send_error(format!("Preview processing failed: {}", e));
            continue;
        }

        // Use max parallel chunks for live view, single chunk during stacking
        let chunk_count = if was_stacked { 1 } else { max_chunks };

        let raw_frame = Arc::new(display_frame);
        rt.block_on(state.set_latest_raw_frame(Arc::clone(&raw_frame)));

        // Claim the counter before encoding so every payload below is filed
        // under the same frame, then wake clients once they are all in place.
        let counter = state.begin_frame();

        if state.lz4_clients.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // Encode frame as RGB8+LZ4 for streaming
            let encode_result = {
                let _encode_span = tracing::info_span!("encode_rgb8_lz4").entered();
                encode_rgb8_lz4_chunked(&raw_frame, chunk_count)
            };
            match encode_result {
                Ok(encoded_data) => rt.block_on(state.set_latest_frame(encoded_data)),
                Err(e) => {
                    rt.block_on(state.frame_rejected(format!("RGB8+LZ4 encoding failed: {}", e)))
                }
            }
        }

        encode_jpeg_tiers(&state, &raw_frame, counter);

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
fn encode_jpeg_tiers(state: &AppState, frame: &Frame, counter: u64) {
    let _span = tracing::info_span!("encode_jpeg_tiers").entered();
    let mut native: Option<bytes::Bytes> = None;

    for tier in JpegTier::all() {
        if state.jpeg_tier_client_count(tier) == 0 {
            continue;
        }

        let downsamples = tier.would_downsample(frame.width(), frame.height());
        if !downsamples {
            if let Some(shared) = &native {
                state.set_tier_jpeg(tier, counter, shared.clone());
                continue;
            }
        }

        let (max_w, max_h) = tier.bounding_box();
        match encode_rgb8_jpeg_bounded(frame, max_w, max_h) {
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
/// Consumes all immediately available messages and returns the most recent
/// one, discarding intermediate frames. This ensures the UI always shows
/// the freshest available frame.
fn drain_to_latest(initial: StackedFrame, rx: &mpsc::Receiver<StackedFrame>) -> StackedFrame {
    let mut latest = initial;
    while let Ok(newer) = rx.try_recv() {
        latest = newer;
    }
    latest
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_drain_to_latest_single_frame() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<super::StackedFrame>(8);

        let settings = crate::server::state::CaptureSettings::default();
        let frame = crate::frame::Frame::zeros(4, 4, 3).unwrap();
        let msg = super::StackedFrame {
            display_frame: frame,
            was_stacked: true,
            frame_number: 1,
            settings,
        };

        // No extra messages — should return initial
        let result = super::drain_to_latest(msg, &rx);
        assert!(result.was_stacked);
        drop(tx);
    }

    #[test]
    fn test_drain_to_latest_multiple_frames() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<super::StackedFrame>(8);

        let settings = crate::server::state::CaptureSettings::default();
        let initial = super::StackedFrame {
            display_frame: crate::frame::Frame::zeros(4, 4, 3).unwrap(),
            was_stacked: false,
            frame_number: 0,
            settings: settings.clone(),
        };

        // Queue additional frames
        for n in 0..3 {
            let msg = super::StackedFrame {
                display_frame: crate::frame::Frame::zeros(4, 4, 3).unwrap(),
                was_stacked: false,
                frame_number: n + 1,
                settings: settings.clone(),
            };
            tx.send(msg).unwrap();
        }
        // Last frame is the "latest"
        let last = super::StackedFrame {
            display_frame: crate::frame::Frame::filled(4, 4, 3, 1.0).unwrap(),
            was_stacked: true,
            frame_number: 4,
            settings: settings.clone(),
        };
        tx.send(last).unwrap();

        let result = super::drain_to_latest(initial, &rx);
        // Should get the last frame (was_stacked = true, filled with 1.0)
        assert!(result.was_stacked);
        assert!(result.display_frame.get_pixel(0, 0, 0) > 0.9);
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
            display_frame: frame,
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
        tokio::task::spawn_blocking(move || super::encode_jpeg_tiers(&state_clone, &frame, 1))
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
        tokio::task::spawn_blocking(move || super::encode_jpeg_tiers(&state_clone, &frame, 1))
            .await
            .unwrap();

        let hd = state.get_tier_jpeg(JpegTier::Hd1080, 1).unwrap();
        for tier in [JpegTier::Qhd1440, JpegTier::Uhd2160] {
            let other = state.get_tier_jpeg(tier, 1).unwrap();
            assert_eq!(hd.as_ptr(), other.as_ptr(), "{tier:?} re-encoded needlessly");
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
        tokio::task::spawn_blocking(move || super::encode_jpeg_tiers(&state_clone, &frame, 1))
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
