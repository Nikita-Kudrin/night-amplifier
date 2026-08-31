//! The two fused f32 → RGB8 kernels every streamed frame goes through.
//!
//! Both share one shape: a **row source** that produces one interleaved RGB f32
//! row at output resolution, a **tail** that applies the tone curve, saturation
//! and contrast to that row, and the 8-bit write. The two sources are the only
//! part that differs — one expands a frame that already fits the bounding box,
//! the other box-averages a larger one down to it — and they are separate
//! traversals with separate planar indexing, which is why `frame/layout_tests.rs`
//! carries a row for each.
//!
//! # Two drivers, because the denoisers cannot fuse
//!
//! With denoising off, each row is gathered, transformed and written inside one
//! closure against a thread-local scratch row — no full-resolution intermediate
//! exists. Both spatial denoisers need neighbourhood access across rows, so with
//! either of them on the driver instead stages the whole resampled image as f32,
//! denoises it, and only then runs the per-row tail. The staged buffer is at
//! *output* resolution: for a 1440² eyepiece that is 24 MB, against the 108 MB
//! the same buffer would cost at an IMX533's native 3008².
//!
//! Keeping the fused path for the off case is not just an optimization — it is
//! what makes `DenoiseConfig::OFF` byte-identical to the pre-denoise output
//! rather than merely equivalent.

use std::cell::RefCell;

use rayon::prelude::*;

use crate::render::denoise::{DenoiseConfig, DenoiseScratch};
use crate::render::output::{
    apply_shadow_floor_slice, write_row_rgb8, DisplayOutput, ShadowFloorTable,
};
use crate::server::state::RenderReadyFrame;

thread_local! {
    /// One interleaved RGB row, reused across frames and tiers. The fused
    /// driver is the only user: the staged driver transforms rows of its own
    /// buffer in place.
    static ROW_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Convert a Frame to RGB8 data, box-averaging down to a bounding box if needed.
///
/// # Why there is no debayering here
///
/// A 1-channel frame reaching this function is genuine monochrome, never a raw
/// CFA mosaic: the stacking task demosaics colour sensors before anything in the
/// render path sees a frame, while mono sensors stay at `channels = 1`. Nothing
/// between there and here changes the channel count.
///
/// So mono channels are replicated across RGB. The previous code instead ran
/// `detect_cfa_pattern` (which never errors for a 1-channel frame ≥ 4x4, and
/// whose confidence was discarded) and debayered unconditionally, which cost a
/// full-resolution f32 RGB frame — 3x the mono source, ~196 MB on an
/// ASI1600MM — per tier per frame, and put colour fringing on grey data.
pub fn frame_to_rgb8_downsampled(
    ready_frame: &RenderReadyFrame,
    max_width: u32,
    max_height: u32,
) -> Result<(Vec<u8>, u32, u32), String> {
    frame_to_rgb8_downsampled_with(
        ready_frame,
        max_width,
        max_height,
        &mut DenoiseScratch::default(),
    )
}

/// [`frame_to_rgb8_downsampled`], reusing a caller-owned set of denoise buffers.
///
/// The render task holds one for the life of its thread: with denoising on, the
/// buffers this saves re-allocating are 13 ms of the 20 ms the filters add to an
/// encode. Callers that convert once — the inline encode for a newly-connected
/// client, tests, benchmarks — use the plain form and pay it once.
pub fn frame_to_rgb8_downsampled_with(
    ready_frame: &RenderReadyFrame,
    max_width: u32,
    max_height: u32,
    scratch: &mut DenoiseScratch,
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

    let (target_width, target_height) = output_dimensions(width, height, max_width, max_height);
    if (target_width, target_height) == (width, height) {
        return Ok((
            expand_to_rgb8_fused(ready_frame, scratch),
            width as u32,
            height as u32,
        ));
    }

    let rgb8 = box_downsample_to_rgb8_fused(ready_frame, target_width, target_height, scratch);
    Ok((rgb8, target_width as u32, target_height as u32))
}

/// The exact size [`frame_to_rgb8_downsampled`] produces for a frame fitted into
/// a bounding box, without doing the conversion.
///
/// The render task keys its per-frame conversion cache on this: two payloads
/// whose clients asked for different boxes but that resolve to the same output
/// size are the *same* conversion, and since tier 2 that conversion carries the
/// denoisers and costs several times the encode that follows it. Sharing it is
/// only sound if the size is decided by exactly the arithmetic the conversion
/// will use, which is why this is the one copy of that arithmetic.
pub fn output_dimensions(
    width: usize,
    height: usize,
    max_width: u32,
    max_height: u32,
) -> (usize, usize) {
    if width <= max_width as usize && height <= max_height as usize {
        return (width, height);
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
    (target_width.max(1), target_height.max(1))
}

/// Expand a frame that already fits the bounding box to interleaved RGB8, fusing the
/// stretch, saturation and contrast stages into the one traversal.
///
/// `pub(crate)`, not `pub`: the `channels == 1` test below is an `if`/`else`, so the
/// `else` arm reads `plane_size * 2 + idx` for *any* other channel count and would run
/// off the end of a 2-channel frame. [`frame_to_rgb8_downsampled`] rejects
/// `channels ∉ {1, 3}` before calling either kernel, and keeping these two
/// crate-private is what makes it the only door in rather than merely the usual one.
pub(crate) fn expand_to_rgb8_fused(
    ready_frame: &RenderReadyFrame,
    scratch: &mut DenoiseScratch,
) -> Vec<u8> {
    debug_assert!(
        matches!(ready_frame.linear_frame.channels(), 1 | 3),
        "expand_to_rgb8_fused requires 1 or 3 channels; frame_to_rgb8_downsampled is the guard"
    );

    let frame = &ready_frame.linear_frame;
    let source = ExpandSource {
        width: frame.width(),
        height: frame.height(),
        channels: frame.channels(),
        src: frame.data(),
    };
    render_rgb8(&source, ready_frame, scratch)
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
///
/// `pub(crate)` for the same reason as [`expand_to_rgb8_fused`]: its `else` arm indexes
/// `plane_size * 2` unconditionally, and [`frame_to_rgb8_downsampled`] is the guard.
pub(crate) fn box_downsample_to_rgb8_fused(
    ready_frame: &RenderReadyFrame,
    target_width: usize,
    target_height: usize,
    scratch: &mut DenoiseScratch,
) -> Vec<u8> {
    debug_assert!(
        matches!(ready_frame.linear_frame.channels(), 1 | 3),
        "box_downsample_to_rgb8_fused requires 1 or 3 channels; frame_to_rgb8_downsampled is the guard"
    );

    let frame = &ready_frame.linear_frame;
    let width = frame.width();
    let height = frame.height();
    let x_scale = width as f32 / target_width as f32;

    let col_ranges: Vec<(usize, usize, f32)> = (0..target_width)
        .map(|x| {
            let src_x0 = (x as f32 * x_scale) as usize;
            let src_x1 = (((x + 1) as f32 * x_scale) as usize).min(width);
            (src_x0, src_x1, 1.0 / (src_x1 - src_x0).max(1) as f32)
        })
        .collect();

    let source = DownsampleSource {
        width,
        height,
        channels: frame.channels(),
        src: frame.data(),
        target_width,
        target_height,
        y_scale: height as f32 / target_height as f32,
        col_ranges,
    };
    render_rgb8(&source, ready_frame, scratch)
}

/// One interleaved RGB f32 row at output resolution.
///
/// Implementors own the planar → interleaved gather, which is the step
/// `frame/layout_tests.rs` guards: `Frame` is plane-major and every 8-bit output
/// format is interleaved, and crossing that boundary wrongly still compiles.
trait RowSource: Sync {
    fn target_width(&self) -> usize;
    fn target_height(&self) -> usize;
    /// Fill `out` (`target_width * 3` samples) with output row `y`.
    fn gather_row(&self, y: usize, out: &mut [f32]);
}

struct ExpandSource<'a> {
    width: usize,
    height: usize,
    channels: usize,
    src: &'a [f32],
}

impl RowSource for ExpandSource<'_> {
    fn target_width(&self) -> usize {
        self.width
    }

    fn target_height(&self) -> usize {
        self.height
    }

    fn gather_row(&self, y: usize, out: &mut [f32]) {
        let plane_size = self.width * self.height;
        // Hoisted: the migration moved `y * width` into the per-pixel
        // expression, and a mono frame — where planar and interleaved are the
        // same thing — measured 6-10 % slower for it.
        let row = y * self.width;
        for x in 0..self.width {
            let out_idx = x * 3;
            if self.channels == 1 {
                let val = self.src[row + x];
                out[out_idx] = val;
                out[out_idx + 1] = val;
                out[out_idx + 2] = val;
            } else {
                out[out_idx] = self.src[row + x];
                out[out_idx + 1] = self.src[plane_size + row + x];
                out[out_idx + 2] = self.src[plane_size * 2 + row + x];
            }
        }
    }
}

struct DownsampleSource<'a> {
    width: usize,
    height: usize,
    channels: usize,
    src: &'a [f32],
    target_width: usize,
    target_height: usize,
    y_scale: f32,
    col_ranges: Vec<(usize, usize, f32)>,
}

impl RowSource for DownsampleSource<'_> {
    fn target_width(&self) -> usize {
        self.target_width
    }

    fn target_height(&self) -> usize {
        self.target_height
    }

    fn gather_row(&self, y: usize, out: &mut [f32]) {
        let plane_size = self.width * self.height;
        let src_y0 = (y as f32 * self.y_scale) as usize;
        let src_y1 = (((y + 1) as f32 * self.y_scale) as usize).min(self.height);
        let row_inv_area = 1.0 / (src_y1 - src_y0).max(1) as f32;

        for (tgt_x, &(src_x0, src_x1, col_inv_area)) in self.col_ranges.iter().enumerate() {
            let inv_area = row_inv_area * col_inv_area;
            let mut acc = [0.0f32; 3];

            for src_y in src_y0..src_y1 {
                let src_row = src_y * self.width;
                for src_x in src_x0..src_x1 {
                    if self.channels == 1 {
                        acc[0] += self.src[src_row + src_x];
                    } else {
                        let idx = src_row + src_x;
                        acc[0] += self.src[idx];
                        acc[1] += self.src[plane_size + idx];
                        acc[2] += self.src[plane_size * 2 + idx];
                    }
                }
            }

            let out_idx = tgt_x * 3;
            if self.channels == 1 {
                let val = acc[0] * inv_area;
                out[out_idx] = val;
                out[out_idx + 1] = val;
                out[out_idx + 2] = val;
            } else {
                out[out_idx] = acc[0] * inv_area;
                out[out_idx + 1] = acc[1] * inv_area;
                out[out_idx + 2] = acc[2] * inv_area;
            }
        }
    }
}

/// The tone-curve half of both kernels, hoisted out of the per-row closure.
struct RowTail<'a> {
    config: &'a crate::render::RenderPipelineConfig,
    has_stretch: bool,
    has_saturate: bool,
    has_contrast: bool,
    black_point: f32,
    scale_lut: std::sync::Arc<Vec<f32>>,
    /// Set only when the floor could not ride the scale LUT — see
    /// `StretchResult::deferred_shadow_floor`. Resampled once per *frame* by
    /// `process_preview_frame` and shared across every payload, rather than
    /// evaluated per pixel or rebuilt per encode.
    floor: Option<&'a ShadowFloorTable>,
}

impl<'a> RowTail<'a> {
    fn new(ready_frame: &'a RenderReadyFrame) -> Self {
        let config = &ready_frame.pipeline_config;
        let has_stretch = config.auto_stretch && ready_frame.stretch_result.is_some();

        let (black_point, scale_lut) = match (has_stretch, ready_frame.stretch_result.as_ref()) {
            (true, Some(sr)) => (sr.black_point, sr.scale_lut.clone()),
            _ => (0.0, std::sync::Arc::new(vec![])),
        };

        let floor = ready_frame
            .stretch_result
            .as_ref()
            .and_then(|sr| sr.deferred_shadow_floor.as_deref());

        Self {
            config,
            has_stretch,
            has_saturate: config.saturation_boost,
            has_contrast: config.contrast,
            black_point,
            scale_lut,
            floor,
        }
    }

    fn apply(&self, f32_row: &mut [f32]) {
        if self.has_stretch {
            crate::render::simd::apply_luminance_scale_lut_simd(
                f32_row,
                self.black_point,
                &self.scale_lut,
                self.config.stretch_config.color_intensity,
            );
        }
        if self.has_saturate {
            if let Some(plugin) =
                crate::license::pro_plugin(&crate::render::stretch::saturation::SATURATION_PLUGIN)
            {
                plugin.apply_boost_slice(f32_row, &self.config.saturation_config);
            }
        }
        if self.has_contrast {
            crate::render::output::apply_contrast_slice(f32_row, &self.config.contrast_config);
        }
        // Last, and after contrast: the fused path puts it last inside the scale
        // LUT for the same reason, so one slider position means one thing on
        // both paths.
        if let Some(table) = self.floor {
            apply_shadow_floor_slice(f32_row, table);
        }
    }
}

/// Drive a row source to interleaved RGB8, staging the resampled image only when
/// a denoiser needs to see across rows.
fn render_rgb8<S: RowSource>(
    source: &S,
    ready_frame: &RenderReadyFrame,
    scratch: &mut DenoiseScratch,
) -> Vec<u8> {
    let tail = RowTail::new(ready_frame);
    let display = ready_frame.pipeline_config.display;
    let denoise = ready_frame.pipeline_config.denoise;

    let target_width = source.target_width();
    let target_height = source.target_height();
    let row_len = target_width * 3;
    let mut output = vec![0u8; row_len * target_height];

    if !denoise.is_enabled() {
        output
            .par_chunks_mut(row_len)
            .with_min_len(32)
            .enumerate()
            .for_each(|(y, row_out)| {
                ROW_BUF.with(|cell| {
                    let mut f32_row = cell.borrow_mut();
                    f32_row.resize(row_len, 0.0);
                    source.gather_row(y, &mut f32_row);
                    tail.apply(&mut f32_row);
                    write_row_rgb8(row_out, &f32_row, y, display);
                });
            });
        return output;
    }

    stage_and_denoise(source, &tail, display, &denoise, &mut output, scratch);
    output
}

/// The staged traversal: resample the whole frame to f32 at output resolution,
/// denoise it, then run the tone curve and the 8-bit write per row.
fn stage_and_denoise<S: RowSource>(
    source: &S,
    tail: &RowTail,
    display: DisplayOutput,
    denoise: &DenoiseConfig,
    output: &mut [u8],
    scratch: &mut DenoiseScratch,
) {
    let target_width = source.target_width();
    let target_height = source.target_height();
    let row_len = target_width * 3;
    let staged_len = row_len * target_height;

    // Taken out rather than borrowed: the denoiser needs the rest of `scratch`
    // at the same time, and moving a `Vec` out and back costs a pointer swap.
    let mut owned = std::mem::take(&mut scratch.staged);
    let staged = crate::render::denoise::take(&mut owned, staged_len);

    staged
        .par_chunks_mut(row_len)
        .with_min_len(32)
        .enumerate()
        .for_each(|(y, row)| source.gather_row(y, row));

    crate::render::denoise::denoise_rgb_interleaved_with(
        staged,
        target_width,
        target_height,
        denoise,
        scratch,
    );

    output
        .par_chunks_mut(row_len)
        .zip(staged.par_chunks_mut(row_len))
        .with_min_len(32)
        .enumerate()
        .for_each(|(y, (row_out, row))| {
            tail.apply(row);
            write_row_rgb8(row_out, row, y, display);
        });

    scratch.staged = owned;
}
