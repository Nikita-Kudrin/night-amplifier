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

        let raw_frame = Arc::new(display_frame);
        rt.block_on(state.set_latest_raw_frame(Arc::clone(&raw_frame)));

        if state.lz4_clients.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // Encode frame as RGB8+LZ4 for streaming
            let encode_result = {
                let _encode_span = tracing::info_span!("encode_rgb8_lz4").entered();
                encode_rgb8_lz4_chunked(&raw_frame, chunk_count)
            };
            match encode_result {
                Ok(encoded_data) => {
                    rt.block_on(state.set_latest_frame(encoded_data));
                }
                Err(e) => {
                    rt.block_on(state.frame_rejected(format!("RGB8+LZ4 encoding failed: {}", e)));
                    rt.block_on(state.notify_new_frame());
                }
            }
        } else {
            // Even if we skip LZ4 encoding, we must notify that a frame is ready
            // so JPEG clients can wake up and stream it.
            rt.block_on(state.notify_new_frame());
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

    #[tokio::test]
    async fn test_run_render_task_notifies_frame_ready_with_no_lz4_clients() {
        use std::sync::atomic::Ordering;
        use std::time::Duration;
        
        let (state, _disk_writer) = crate::server::state::AppState::new_for_testing();
        let state = std::sync::Arc::new(state);
        
        // Ensure no LZ4 clients are connected (this is the default, but let's be explicit)
        assert_eq!(state.lz4_clients.load(Ordering::SeqCst), 0);
        
        let initial_counter = state.frame_counter.load(Ordering::SeqCst);
        
        let (tx, rx) = std::sync::mpsc::channel();
        let settings = crate::server::state::CaptureSettings::default();
        
        // Send a frame to be rendered
        tx.send(super::StackedFrame {
            display_frame: crate::frame::Frame::zeros(10, 10, 3).unwrap(),
            was_stacked: false,
            frame_number: 1,
            settings,
        }).unwrap();
        
        // Drop tx so the channel closes and run_render_task naturally exits
        drop(tx);
        
        let rt = tokio::runtime::Handle::current();
        
        // Run the render task on a blocking thread to avoid 'Cannot start a runtime from within a runtime' panic
        let state_clone = state.clone();
        tokio::task::spawn_blocking(move || {
            super::run_render_task(state_clone, rx, rt);
        }).await.unwrap();
        
        // The frame_counter MUST have increased because notify_new_frame was called
        let new_counter = state.frame_counter.load(Ordering::SeqCst);
        assert_eq!(new_counter, initial_counter + 1, "frame_counter did not increment when lz4_clients was 0");
    }
}
