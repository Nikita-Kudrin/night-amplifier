use crate::frame::sample_to_u8;
/// Binary header magic number for RGB8+LZ4 stream format (legacy single-block)
///
/// Header layout (16 bytes):
/// - bytes 0-3:   Magic number "SA08" (0x53413038)
/// - bytes 4-7:   Width (u32, little-endian)
/// - bytes 8-11:  Height (u32, little-endian)
/// - bytes 12-15: Compressed size (u32, little-endian)
///
/// Followed by LZ4-compressed RGB8 pixel data (3 bytes per pixel)
pub const RGB8_MAGIC: u32 = 0x53413038; // "SA08" in little-endian

/// Binary header magic number for chunked RGB8+LZ4 stream format
///
/// Header (20 bytes):
/// - bytes 0-3:    Magic "SA09" (0x53413039)
/// - bytes 4-7:    Width (u32 LE)
/// - bytes 8-11:   Height (u32 LE)
/// - bytes 12-15:  Total payload size (u32 LE) — everything after header
/// - bytes 16-19:  Chunk count (u32 LE)
///
/// Per-chunk descriptor (8 bytes each, chunk_count entries):
/// - bytes 0-3:    Compressed size of this chunk (u32 LE)
/// - bytes 4-7:    Decompressed size of this chunk (u32 LE)
///
/// Followed by concatenated compressed chunk data
pub const RGB8_CHUNKED_MAGIC: u32 = 0x53413039; // "SA09" in little-endian

/// Binary header magic number for JPEG stream format (SA10)
///
/// Header (16 bytes):
/// - bytes 0-3:    Magic "SA10" (0x53413130)
/// - bytes 4-7:    Width (u32 LE)
/// - bytes 8-11:   Height (u32 LE)
/// - bytes 12-15:  Payload size (u32 LE)
/// - Followed by raw JPEG bytes
pub const JPEG_MAGIC: u32 = 0x53413130; // "SA10" in little-endian

pub const SA09_HEADER_SIZE: usize = 20;
pub const SA09_CHUNK_DESCRIPTOR_SIZE: usize = 8;
pub const SA10_HEADER_SIZE: usize = 16;

/// Smallest bounding box a JPEG client may ask for.
pub const JPEG_MIN_BOUNDING_BOX: (u32, u32) = (1920, 1080);
/// Largest bounding box a JPEG client may ask for.
pub const JPEG_MAX_BOUNDING_BOX: (u32, u32) = (3840, 2160);

/// Clamp a client-requested viewport to the streamable JPEG range.
///
/// Single source of truth for the bounds: resolution tiers and the encoder
/// both derive from it, so a request always maps to the tier that is actually
/// encoded.
pub fn clamp_client_resolution(req_w: Option<u32>, req_h: Option<u32>) -> (u32, u32) {
    let (min_w, min_h) = JPEG_MIN_BOUNDING_BOX;
    let (max_w, max_h) = JPEG_MAX_BOUNDING_BOX;
    (
        req_w.unwrap_or(min_w).clamp(min_w, max_w),
        req_h.unwrap_or(min_h).clamp(min_h, max_h),
    )
}

/// Convert a Frame to RGB8 data, box-averaging down to a bounding box if needed.
///
/// # Why there is no debayering here
///
/// A 1-channel frame reaching this function is genuine monochrome, never a raw
/// CFA mosaic: every provider debayers colour sensors at capture — `from_bayer`
/// in `camera/{zwo,qhy,playerone,svbony,touptek}` and `SerColorId::is_bayer` in
/// `ser/reader.rs` — while mono sensors take the `from_raw` path at
/// `channels = 1`, and the simulator only builds a `Debayerer` for
/// `SensorType::Color`. Nothing between capture and here changes the channel
/// count.
///
/// So mono channels are replicated across RGB. The previous code instead ran
/// `detect_cfa_pattern` (which never errors for a 1-channel frame ≥ 4x4, and
/// whose confidence was discarded) and debayered unconditionally, which cost a
/// full-resolution f32 RGB frame — 3x the mono source, ~196 MB on an
/// ASI1600MM — per tier per frame, and put colour fringing on grey data.
pub fn frame_to_rgb8_downsampled(
    ready_frame: &crate::server::state::RenderReadyFrame,
    max_width: u32,
    max_height: u32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let frame = &ready_frame.linear_frame;
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();

    if channels != 1 && channels != 3 {
        return Err(format!(
            "Unsupported channel count for RGB8 conversion: {}",
            channels
        ));
    }

    if width <= max_width as usize && height <= max_height as usize {
        return Ok((
            expand_to_rgb8_fused(ready_frame),
            width as u32,
            height as u32,
        ));
    }

    let aspect_ratio = width as f32 / height as f32;
    let (target_width, target_height) =
        if width as f32 / max_width as f32 > height as f32 / max_height as f32 {
            (
                max_width as usize,
                (max_width as f32 / aspect_ratio) as usize,
            )
        } else {
            (
                (max_height as f32 * aspect_ratio) as usize,
                max_height as usize,
            )
        };
    let target_width = target_width.max(1);
    let target_height = target_height.max(1);

    let rgb8 = box_downsample_to_rgb8_fused(ready_frame, target_width, target_height);
    Ok((rgb8, target_width as u32, target_height as u32))
}

/// Encode RGB8 data with LZ4 compression for high-speed streaming (legacy SA08 format)
pub fn expand_to_rgb8_fused(ready_frame: &crate::server::state::RenderReadyFrame) -> Vec<u8> {
    use rayon::prelude::*;
    use std::cell::RefCell;

    thread_local! {
        static ROW_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }

    let frame = &ready_frame.linear_frame;
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let src_data = frame.data();

    let config = &ready_frame.pipeline_config;
    let has_stretch = config.auto_stretch && ready_frame.stretch_result.is_some();
    let has_saturate = config.saturation_boost;
    let has_contrast = config.contrast;

    let (black_point, scale_lut) = if has_stretch {
        let sr = ready_frame.stretch_result.as_ref().unwrap();
        (sr.black_point, sr.scale_lut.clone())
    } else {
        (0.0, std::sync::Arc::new(vec![]))
    };

    let row_len = width * 3;
    let mut output = vec![0u8; width * height * 3];

    output
        .par_chunks_mut(row_len)
        .with_min_len(32)
        .enumerate()
        .for_each(|(y, row_out)| {
            ROW_BUF.with(|cell| {
                let mut f32_row = cell.borrow_mut();
                f32_row.resize(row_len, 0.0);

                let row_start = y * width * channels;

                for x in 0..width {
                    let out_idx = x * 3;
                    if channels == 1 {
                        let val = src_data[row_start + x];
                        f32_row[out_idx] = val;
                        f32_row[out_idx + 1] = val;
                        f32_row[out_idx + 2] = val;
                    } else {
                        let in_idx = row_start + x * channels;
                        f32_row[out_idx] = src_data[in_idx];
                        f32_row[out_idx + 1] = src_data[in_idx + 1];
                        f32_row[out_idx + 2] = src_data[in_idx + 2];
                    }
                }

                if has_stretch {
                    crate::render::simd::apply_luminance_scale_lut_simd(
                        &mut f32_row,
                        black_point,
                        &scale_lut,
                        config.stretch_config.color_intensity,
                    );
                }
                if has_saturate {
                    if let Some(plugin) = crate::license::pro_plugin(
                        &crate::render::stretch::saturation::SATURATION_PLUGIN,
                    ) {
                        plugin.apply_boost_slice(&mut f32_row, &config.saturation_config);
                    }
                }
                if has_contrast {
                    crate::render::output::apply_contrast_slice(
                        &mut f32_row,
                        &config.contrast_config,
                    );
                }

                for i in 0..row_len {
                    row_out[i] = sample_to_u8(f32_row[i]);
                }
            });
        });

    output
}

/// Box-average `frame` to `target_width` x `target_height` in **linear light**, then apply
/// the tone-curve stretch (+ saturation/contrast) to the averaged result.
///
/// # Why stretch happens after downsampling, not before
///
/// This resamples before applying the non-linear, shadow-boosting tone curve, rather than
/// the reverse (which is what the pre-fusion pipeline did: stretch once at full resolution,
/// then downsample the already-stretched result). Because the stretch curves used here
/// (asinh, MTF) are concave, Jensen's inequality guarantees `curve(average(pixels)) >=
/// average(curve(pixels))` for any box of source pixels — so this order can only preserve
/// or brighten faint detail in a downsampled tier relative to the old order, never dim it.
/// See `test_downsample_then_stretch_is_at_least_as_bright_as_stretch_then_downsample` for a
/// pinned numerical example.
pub fn box_downsample_to_rgb8_fused(
    ready_frame: &crate::server::state::RenderReadyFrame,
    target_width: usize,
    target_height: usize,
) -> Vec<u8> {
    use rayon::prelude::*;
    use std::cell::RefCell;

    thread_local! {
        static DS_ROW_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }

    let frame = &ready_frame.linear_frame;
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let src_data = frame.data();
    let src_stride = width * channels;

    let config = &ready_frame.pipeline_config;
    let has_stretch = config.auto_stretch && ready_frame.stretch_result.is_some();
    let has_saturate = config.saturation_boost;
    let has_contrast = config.contrast;

    let (black_point, scale_lut) = if has_stretch {
        let sr = ready_frame.stretch_result.as_ref().unwrap();
        (sr.black_point, sr.scale_lut.clone())
    } else {
        (0.0, std::sync::Arc::new(vec![]))
    };

    let x_scale = width as f32 / target_width as f32;
    let y_scale = height as f32 / target_height as f32;

    let col_ranges: Vec<(usize, usize, f32)> = (0..target_width)
        .map(|x| {
            let src_x0 = (x as f32 * x_scale) as usize;
            let src_x1 = (((x + 1) as f32 * x_scale) as usize).min(width);
            (src_x0, src_x1, 1.0 / (src_x1 - src_x0).max(1) as f32)
        })
        .collect();

    let row_len = target_width * 3;
    let mut output = vec![0u8; target_width * target_height * 3];

    output
        .par_chunks_mut(row_len)
        .with_min_len(32)
        .enumerate()
        .for_each(|(y, row_out)| {
            DS_ROW_BUF.with(|cell| {
                let mut f32_row = cell.borrow_mut();
                f32_row.resize(row_len, 0.0);

                let src_y0 = (y as f32 * y_scale) as usize;
                let src_y1 = (((y + 1) as f32 * y_scale) as usize).min(height);
                let row_inv_area = 1.0 / (src_y1 - src_y0).max(1) as f32;

                for (tgt_x, &(src_x0, src_x1, col_inv_area)) in col_ranges.iter().enumerate() {
                    let inv_area = row_inv_area * col_inv_area;

                    let mut acc = [0.0f32; 3];

                    for src_y in src_y0..src_y1 {
                        let row_start = src_y * src_stride;
                        for src_x in src_x0..src_x1 {
                            let src_idx = row_start + src_x * channels;
                            if channels == 1 {
                                acc[0] += src_data[src_idx];
                            } else {
                                acc[0] += src_data[src_idx];
                                acc[1] += src_data[src_idx + 1];
                                acc[2] += src_data[src_idx + 2];
                            }
                        }
                    }

                    let out_idx = tgt_x * 3;
                    if channels == 1 {
                        let val = acc[0] * inv_area;
                        f32_row[out_idx] = val;
                        f32_row[out_idx + 1] = val;
                        f32_row[out_idx + 2] = val;
                    } else {
                        f32_row[out_idx] = acc[0] * inv_area;
                        f32_row[out_idx + 1] = acc[1] * inv_area;
                        f32_row[out_idx + 2] = acc[2] * inv_area;
                    }
                }

                if has_stretch {
                    crate::render::simd::apply_luminance_scale_lut_simd(
                        &mut f32_row,
                        black_point,
                        &scale_lut,
                        config.stretch_config.color_intensity,
                    );
                }
                if has_saturate {
                    if let Some(plugin) = crate::license::pro_plugin(
                        &crate::render::stretch::saturation::SATURATION_PLUGIN,
                    ) {
                        plugin.apply_boost_slice(&mut f32_row, &config.saturation_config);
                    }
                }
                if has_contrast {
                    crate::render::output::apply_contrast_slice(
                        &mut f32_row,
                        &config.contrast_config,
                    );
                }

                for i in 0..row_len {
                    row_out[i] = sample_to_u8(f32_row[i]);
                }
            });
        });

    output
}
