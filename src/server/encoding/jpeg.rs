use std::cell::RefCell;

use crate::server::encoding::format::*;
use crate::server::encoding::fused::frame_to_rgb8_downsampled;

use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, warn};
pub(crate) fn calculate_dynamic_jpeg_quality(width: u32, height: u32) -> i32 {
    let smallest_side = width.min(height);
    // If resolution is lower than 2K (1440p), use 95% quality.
    // Otherwise, default to 90% quality.
    if smallest_side < 1440 {
        95
    } else {
        90
    }
}

thread_local! {
    /// Reused TurboJPEG compressor. The render task encodes one payload per
    /// active resolution tier per frame, so keeping the compressor alive avoids
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

fn compress_rgb8_to_jpeg(rgb8_data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let quality = calculate_dynamic_jpeg_quality(width, height);
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
/// The box is used verbatim, which lets the `Original` resolution tier stream a
/// frame at its native size. Clients go through [`encode_rgb8_jpeg_dynamic`],
/// which clamps the request first.
pub fn encode_rgb8_jpeg_bounded(
    ready_frame: &crate::server::state::RenderReadyFrame,
    max_w: u32,
    max_h: u32,
) -> Result<Vec<u8>, String> {
    let (rgb8_data, width, height) = {
        let _span = tracing::info_span!("frame_to_rgb8").entered();
        frame_to_rgb8_downsampled(ready_frame, max_w, max_h)?
    };

    encode_rgb8_jpeg_bounded_from_u8(&rgb8_data, width, height)
}

/// Encode already-converted RGB8 data as JPEG (SA10 format)
pub fn encode_rgb8_jpeg_bounded_from_u8(
    rgb8_data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let compressed = {
        let _span = tracing::info_span!("jpeg_compress").entered();
        compress_rgb8_to_jpeg(rgb8_data, width, height)?
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

/// Encode frame as JPEG at a client-requested resolution (SA10 format)
pub fn encode_rgb8_jpeg_dynamic(
    ready_frame: &crate::server::state::RenderReadyFrame,
    req_w: Option<u32>,
    req_h: Option<u32>,
) -> Result<Vec<u8>, String> {
    let (max_w, max_h) = clamp_client_resolution(req_w, req_h);
    encode_rgb8_jpeg_bounded(ready_frame, max_w, max_h)
}
