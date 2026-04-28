use std::sync::mpsc;
use std::sync::Arc;
use tracing::debug;

use crate::server::encoding::encode_rgb8_lz4_chunked;
use crate::server::state::AppState;

use super::channel::StackedFrame;
use super::pipeline;

/// Preview rendering and encoding, running on a dedicated OS thread.
///
/// Drains the channel to the latest frame to keep the UI responsive.
/// Runs `process_preview_frame()` + `encode_rgb8_lz4_chunked()` and pushes
/// the encoded data to the WebSocket stream.
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

        // Encode frame as RGB8+LZ4 for streaming
        let encode_result = {
            let _encode_span = tracing::info_span!("encode_rgb8_lz4").entered();
            encode_rgb8_lz4_chunked(&display_frame, chunk_count)
        };
        match encode_result {
            Ok(encoded_data) => {
                rt.block_on(state.set_latest_frame(encoded_data));
            }
            Err(e) => {
                rt.block_on(state.frame_rejected(format!("RGB8+LZ4 encoding failed: {}", e)));
            }
        }
    }

    debug!("Render task ended");
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
}
