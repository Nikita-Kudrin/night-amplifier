//! Payload encoding shared by the imaging render task and the guide loop.
//!
//! Each family ([`StreamKind`]) is encoded once per frame at the size its setting chose —
//! Streaming Resolution for JPEG, Eyepiece Streaming Resolution for lossless — and only
//! while it has viewers. Every client of a family is sent the same bytes.

use std::sync::Arc;

use crate::server::state::{CaptureSettings, FrameStream, RenderReadyFrame, Resolution, StreamKind};
use crate::telemetry::metrics as telemetry_metrics;

/// Both families' resolutions for one frame, read from the live settings (see
/// [`CaptureSettings::stream_resolution`] for why not the frame's snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StreamResolutions {
    pub jpeg: Resolution,
    pub lossless: Resolution,
}

impl StreamResolutions {
    pub(super) fn of(settings: &CaptureSettings) -> Self {
        Self {
            jpeg: settings.stream_resolution(StreamKind::Jpeg),
            lossless: settings.stream_resolution(StreamKind::Lossless),
        }
    }
}

/// An interleaved RGB8 buffer with its width and height.
pub(super) type Rgb8Image = Arc<(Vec<u8>, u32, u32)>;

/// The RGB8 conversions one frame needs, at most one per distinct output size. The two
/// families share one whenever their resolutions resolve to the same size (a 2712x1538
/// sensor fitted into the 4K box or no box are both 2712x1538) — and the conversion
/// carries the denoisers, ~5x the encode. A `Vec`, not a map: at most two entries.
#[derive(Default)]
pub(super) struct ConversionCache {
    entries: Vec<((usize, usize), Rgb8Image)>,
    /// The denoisers' working buffers, reused for the life of the render thread.
    /// Kept here rather than in a thread-local so nothing else in the process
    /// can strand 75 MB behind a pooled worker.
    scratch: crate::render::denoise::DenoiseScratch,
}

impl ConversionCache {
    /// Drop the previous frame's conversions, keeping the buffers that produced
    /// them.
    pub(super) fn begin_frame(&mut self) {
        self.entries.clear();
    }

    /// The RGB8 buffer for a bounding box, converting only if nothing already
    /// built has the same output size.
    pub(super) fn get(
        &mut self,
        frame: &RenderReadyFrame,
        max_w: u32,
        max_h: u32,
    ) -> Result<Rgb8Image, String> {
        let key = crate::server::encoding::output_dimensions(
            frame.linear_frame.width(),
            frame.linear_frame.height(),
            max_w,
            max_h,
        );
        if let Some((_, data)) = self.entries.iter().find(|(k, _)| *k == key) {
            return Ok(Arc::clone(data));
        }

        let _span = tracing::info_span!("frame_to_rgb8", width = key.0, height = key.1).entered();
        let data = crate::server::encoding::frame_to_rgb8_downsampled_with(
            frame,
            max_w,
            max_h,
            &mut self.scratch,
        )
        .map_err(|e| format!("{e} ({}x{})", key.0, key.1))?;
        let data = Arc::new(data);
        self.entries.push((key, Arc::clone(&data)));
        Ok(data)
    }

    /// How many conversions were actually performed, for tests that need to see
    /// that sharing happened rather than infer it from a payload.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Encode the JPEG payload at `resolution`, if anyone watches the JPEG family.
pub(super) fn encode_jpeg(
    stream: &FrameStream,
    frame: &RenderReadyFrame,
    counter: u64,
    conversions: &mut ConversionCache,
    resolution: Resolution,
) -> Result<(), String> {
    let _span = tracing::info_span!("encode_jpeg").entered();
    encode_family(stream, StreamKind::Jpeg, frame, counter, conversions, resolution, |rgb, w, h| {
        let started = std::time::Instant::now();
        let result = crate::server::encoding::encode_rgb8_jpeg_bounded_from_u8(rgb, w, h);
        telemetry_metrics::record_jpeg_encode_ms(
            resolution.label(),
            started.elapsed().as_secs_f64() * 1000.0,
        );
        result
    })
}

/// Encode the RGB8+LZ4 payload at `resolution` in `chunk_count` parallel stripes, if
/// anyone watches the lossless family.
pub(super) fn encode_lossless(
    stream: &FrameStream,
    frame: &RenderReadyFrame,
    counter: u64,
    conversions: &mut ConversionCache,
    resolution: Resolution,
    chunk_count: usize,
) -> Result<(), String> {
    let _span = tracing::info_span!("encode_rgb8_lz4").entered();
    encode_family(stream, StreamKind::Lossless, frame, counter, conversions, resolution, |rgb, w, h| {
        crate::server::encoding::encode_rgb8_lz4_chunked_from_u8(rgb, w, h, chunk_count)
    })
}

fn encode_family(
    stream: &FrameStream,
    kind: StreamKind,
    frame: &RenderReadyFrame,
    counter: u64,
    conversions: &mut ConversionCache,
    resolution: Resolution,
    encode: impl FnOnce(&[u8], u32, u32) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    if stream.viewer_count(kind) == 0 {
        return Ok(());
    }
    let (max_w, max_h) = resolution.bounding_box();
    let rgb = conversions.get(frame, max_w, max_h).map_err(|e| {
        format!("RGB8 conversion failed for the {} stream at {}: {e}", kind.label(), resolution.label())
    })?;
    let payload = encode(&rgb.0, rgb.1, rgb.2)
        .map_err(|e| format!("{} encoding failed at {}: {e}", kind.label(), resolution.label()))?;
    stream.set_payload(kind, counter, payload);
    Ok(())
}

/// Each family's last reported encode failure.
///
/// A failure that repeats every frame (a frame the encoder refuses, say) is reported once:
/// the UI shows the latest `error` event, so a per-frame repeat re-raised it several times
/// a second. A successful or skipped encode clears it, so a later failure is news again.
#[derive(Default)]
pub(super) struct FailureReports {
    last: [Option<String>; StreamKind::COUNT],
}

impl FailureReports {
    /// The error worth reporting for `kind` now: `None` on success or a repeat.
    pub(super) fn to_report(&mut self, kind: StreamKind, result: Result<(), String>) -> Option<String> {
        let last = &mut self.last[kind as usize];
        match result {
            Ok(()) => {
                *last = None;
                None
            }
            Err(e) if last.as_deref() == Some(e.as_str()) => None,
            Err(e) => {
                *last = Some(e.clone());
                Some(e)
            }
        }
    }
}

#[cfg(test)]
#[path = "stream_encoding_tests.rs"]
mod tests;
