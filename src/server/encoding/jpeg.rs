use std::cell::RefCell;

use crate::server::encoding::format::*;
use crate::server::encoding::fused::frame_to_rgb8_downsampled;

use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, warn};

/// JPEG quality for an output of this size, and whether the denoisers ran on it.
///
/// 95 below 1440p and 90 from there up, where the payload is largest — except for a
/// denoised frame, which is 95 at every size. Its sky is smooth to a fraction of a level
/// and the dither carries what lies between levels; q90 quantises that away with the
/// rest of each 8x8 block's fine detail, and block means then miss by 4-9x what they did
/// before the encoder (`dither_tests`). On four denoised sessions q95 cuts that loss over
/// the sky by 31-55 % (0.24-0.27 -> 0.12-0.17 levels) for 1.5-2x the payload: 2.5-4.2 Mb
/// a 1440p frame (Pro `measure_the_jpeg_quality_on_denoised_sessions`).
pub fn jpeg_quality(width: u32, height: u32, denoised: bool) -> i32 {
    if denoised || width.min(height) < 1440 {
        95
    } else {
        90
    }
}

thread_local! {
    /// Reused TurboJPEG compressor. The render task encodes a JPEG every frame,
    /// so keeping the compressor alive avoids
    /// re-allocating libjpeg-turbo's internal buffers on every encode.
    static JPEG_COMPRESSOR: std::cell::RefCell<Option<turbojpeg::Compressor>> =
        const { std::cell::RefCell::new(None) };
}

fn configure_compressor(
    compressor: &mut turbojpeg::Compressor,
    quality: i32,
) -> Result<(), String> {
    compressor
        .set_quality(quality)
        .map_err(|e| format!("TurboJPEG set_quality failed: {}", e))?;
    compressor
        .set_subsamp(turbojpeg::Subsamp::Sub2x2)
        .map_err(|e| format!("TurboJPEG set_subsamp failed: {}", e))
}

fn compress_rgb8_to_jpeg(
    rgb8_data: &[u8],
    width: u32,
    height: u32,
    quality: i32,
) -> Result<Vec<u8>, String> {
    let image = turbojpeg::Image {
        pixels: rgb8_data,
        width: width as usize,
        pitch: 3 * width as usize,
        height: height as usize,
        format: turbojpeg::PixelFormat::RGB,
    };

    JPEG_COMPRESSOR.with(|slot| {
        // A re-entrant call would find the slot already borrowed; fall back to a
        // throwaway compressor instead of panicking.
        let Ok(mut borrowed) = slot.try_borrow_mut() else {
            let mut compressor = turbojpeg::Compressor::new().map_err(|e| e.to_string())?;
            configure_compressor(&mut compressor, quality)?;
            return compressor.compress_to_vec(image).map_err(|e| e.to_string());
        };

        if borrowed.is_none() {
            *borrowed = Some(turbojpeg::Compressor::new().map_err(|e| e.to_string())?);
        }
        let Some(compressor) = borrowed.as_mut() else {
            return Err("TurboJPEG compressor unavailable".to_string());
        };
        configure_compressor(compressor, quality)?;
        compressor.compress_to_vec(image).map_err(|e| e.to_string())
    })
}

/// Encode a frame as JPEG (SA10 format) fitted into an exact bounding box.
///
/// The box is used verbatim: `(u32::MAX, u32::MAX)` streams the frame at its native size
/// (see `Resolution::bounding_box`).
pub fn encode_rgb8_jpeg_bounded(
    ready_frame: &crate::server::state::RenderReadyFrame,
    max_w: u32,
    max_h: u32,
) -> Result<Vec<u8>, String> {
    let (rgb8_data, width, height) = {
        let _span = tracing::info_span!("frame_to_rgb8").entered();
        frame_to_rgb8_downsampled(ready_frame, max_w, max_h)?
    };

    let quality = jpeg_quality(width, height, ready_frame.pipeline_config.denoise.is_enabled());
    encode_rgb8_jpeg_bounded_from_u8(&rgb8_data, width, height, quality)
}

/// Encode already-converted RGB8 data as JPEG (SA10 format) at `quality`, which a stream
/// takes from [`jpeg_quality`].
pub fn encode_rgb8_jpeg_bounded_from_u8(
    rgb8_data: &[u8],
    width: u32,
    height: u32,
    quality: i32,
) -> Result<Vec<u8>, String> {
    let compressed = {
        let _span = tracing::info_span!("jpeg_compress").entered();
        compress_rgb8_to_jpeg(rgb8_data, width, height, quality)?
    };

    let payload_size = compressed.len() as u32;
    let mut output = Vec::with_capacity(SA10_HEADER_SIZE + compressed.len());
    output.extend_from_slice(&JPEG_MAGIC.to_le_bytes());
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.extend_from_slice(&payload_size.to_le_bytes());
    output.extend_from_slice(&compressed);

    Ok(output)
}
