//! The two fused f32 -> RGB8 kernels every streamed frame goes through. Both share
//! one shape: a **row source** producing one interleaved RGB f32 row at output
//! resolution, and a **tail** applying the tone curve, saturation, contrast and the
//! 8-bit write. The sources differ (one expands a frame already fitting the
//! bounding box, the other area-averages a larger one down) as separate traversals
//! with separate planar indexing — why `frame/layout_tests.rs` carries a row for each.
//!
//! Two drivers because the denoisers can't fuse: with denoising off, each row is
//! gathered, transformed and written inside one closure against a thread-local
//! scratch row, no full-resolution intermediate. Either denoiser needs cross-row
//! neighbourhood access, so on, the driver stages the whole resampled image as f32
//! at *output* resolution (24MB for a 1440² eyepiece, vs 108MB at native 3008²),
//! denoises it, then runs the per-row tail. Keeping the fused path for the off case
//! isn't just an optimization — it's what makes `DenoiseConfig::OFF` byte-identical
//! to the pre-denoise output, not merely equivalent.

use std::cell::RefCell;

use rayon::prelude::*;

use crate::render::denoise::{DenoiseConfig, DenoiseScratch};
use crate::render::output::{
    apply_shadow_floor_slice, write_row_rgb8, DisplayOutput, ShadowFloorTable,
};
use crate::server::state::RenderReadyFrame;

use super::axis_taps::AxisTaps;

thread_local! {
    /// One interleaved RGB row, reused across frames and payloads. The fused
    /// driver is the only user: the staged driver transforms rows of its own
    /// buffer in place.
    static ROW_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Convert a Frame to RGB8 data, area-averaging down to a bounding box if needed. No
/// debayering here: a 1-channel frame reaching this function is genuine monochrome,
/// never raw CFA — the stacking task demosaics colour sensors before the render path
/// sees a frame, and nothing between there and here changes channel count. So mono
/// channels are simply replicated across RGB. The old code instead ran
/// `detect_cfa_pattern` (never errors on a 1-channel frame, confidence discarded)
/// and debayered unconditionally — a full-resolution f32 RGB frame (3x the mono
/// source, ~196MB on an ASI1600MM) per payload per frame, with colour fringing on grey
/// data.
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

    let rgb8 = area_downsample_to_rgb8_fused(ready_frame, target_width, target_height, scratch);
    Ok((rgb8, target_width as u32, target_height as u32))
}

/// The exact size [`frame_to_rgb8_downsampled`] produces for a frame fitted into
/// a bounding box, without doing the conversion.
///
/// The render task keys its per-frame conversion cache on this: two payloads
/// whose resolutions are different boxes but resolve to the same output
/// size are the *same* conversion, and that conversion carries the
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

/// Area-average `frame` to `target_width` x `target_height` in **linear light**, then
/// apply the tone-curve stretch (+ saturation/contrast) to the averaged result.
/// Stretch happens after downsampling, not before (the pre-fusion order): the
/// stretch curves here (asinh, MTF) are concave, so Jensen's inequality guarantees
/// `curve(average(pixels)) >= average(curve(pixels))` for any source box — this
/// order can only preserve or brighten faint detail in a downsampled stream, never dim
/// it (see `test_downsample_then_stretch_is_at_least_as_bright_as_stretch_then_downsample`).
///
/// The kernel is [`AxisTaps`], not a whole-pixel box: see there for why.
///
/// `pub(crate)` for the same reason as [`expand_to_rgb8_fused`]: its `else` arm
/// indexes `plane_size * 2` unconditionally, and [`frame_to_rgb8_downsampled`] is
/// the guard.
pub(crate) fn area_downsample_to_rgb8_fused(
    ready_frame: &RenderReadyFrame,
    target_width: usize,
    target_height: usize,
    scratch: &mut DenoiseScratch,
) -> Vec<u8> {
    debug_assert!(
        matches!(ready_frame.linear_frame.channels(), 1 | 3),
        "area_downsample_to_rgb8_fused requires 1 or 3 channels; frame_to_rgb8_downsampled is the guard"
    );

    let frame = &ready_frame.linear_frame;
    let source = DownsampleSource {
        width: frame.width(),
        height: frame.height(),
        channels: frame.channels(),
        src: frame.data(),
        columns: AxisTaps::cached(frame.width(), target_width),
        rows: AxisTaps::cached(frame.height(), target_height),
    };
    render_rgb8(&source, ready_frame, scratch)
}

/// One interleaved RGB f32 row at output resolution.
///
/// Implementors own the planar → interleaved gather, which is the step
/// `frame/layout_tests.rs` guards: `Frame` is plane-major and every 8-bit output
/// format is interleaved, and crossing that boundary wrongly still compiles.
pub(super) trait RowSource: Sync {
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
    columns: std::sync::Arc<AxisTaps>,
    rows: std::sync::Arc<AxisTaps>,
}

thread_local! {
    /// Source rows combined vertically, one per channel, before the horizontal pass.
    static VERTICAL_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

impl RowSource for DownsampleSource<'_> {
    fn target_width(&self) -> usize {
        self.columns.start.len()
    }

    fn target_height(&self) -> usize {
        self.rows.start.len()
    }

    /// Separable: combine the tapped source rows per plane (contiguous, so the
    /// inner loop vectorises), then tap columns out of that one combined row.
    fn gather_row(&self, y: usize, out: &mut [f32]) {
        let (w, h) = (self.width, self.height);
        let (first_row, row_weights) = self.rows.of(y);
        VERTICAL_BUF.with(|cell| {
            let mut combined = cell.borrow_mut();
            combined.clear();
            combined.resize(w * self.channels, 0.0);
            for (c, plane_out) in combined.chunks_exact_mut(w).enumerate() {
                let plane = &self.src[c * w * h..(c + 1) * w * h];
                for (k, &wy) in row_weights.iter().enumerate() {
                    if wy == 0.0 {
                        continue;
                    }
                    let row = (first_row + k) * w;
                    for (o, &v) in plane_out.iter_mut().zip(&plane[row..row + w]) {
                        *o += wy * v;
                    }
                }
            }

            for (x, pixel) in out.chunks_exact_mut(3).enumerate() {
                let (first_col, col_weights) = self.columns.of(x);
                let mut acc = [0.0f32; 3];
                for (c, a) in acc.iter_mut().enumerate().take(self.channels) {
                    let plane = &combined[c * w..(c + 1) * w];
                    let taps = &plane[first_col..(first_col + col_weights.len()).min(w)];
                    for (&wx, &v) in col_weights.iter().zip(taps) {
                        *a += wx * v;
                    }
                }
                if self.channels == 1 {
                    pixel.fill(acc[0]);
                } else {
                    pixel.copy_from_slice(&acc);
                }
            }
        });
    }
}

/// An already resampled (and denoised) interleaved image, read back row by row.
struct StagedRows<'a> {
    rgb: &'a [f32],
    width: usize,
    height: usize,
}

impl RowSource for StagedRows<'_> {
    fn target_width(&self) -> usize {
        self.width
    }

    fn target_height(&self) -> usize {
        self.height
    }

    fn gather_row(&self, y: usize, out: &mut [f32]) {
        let row_len = self.width * 3;
        out.copy_from_slice(&self.rgb[y * row_len..(y + 1) * row_len]);
    }
}

/// The tone-curve half of both kernels, hoisted out of the per-row closure.
pub(super) struct RowTail<'a> {
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

    pub(super) fn apply(&self, f32_row: &mut [f32]) {
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

/// The frame's noise map, resampled onto the output grid this conversion produces.
///
/// **In quadrature, with the same taps the pixels went through.** An output pixel is
/// `sum(w_i * x_i)` with `sum(w_i) = 1`, so its variance is `sum(w_i^2 * sigma_i^2)`;
/// resampling the map like an image instead overstates output noise by roughly `sqrt(k)`
/// for a `k`-fold reduction, and every threshold built on it comes out that much too
/// aggressive. Nothing downstream reports a number that would show it, which is why the
/// factor is taken from `AxisTaps::sum_sq` on the *same cached instance* the pixels use
/// rather than recomputed here.
///
/// `None` when there is no map, or when it has nothing measured to say.
pub(super) fn output_noise_field(
    ready_frame: &RenderReadyFrame,
    target_width: usize,
    target_height: usize,
) -> Option<crate::frame::NoiseField> {
    let field = ready_frame.noise.as_deref()?;
    if !field.is_usable() {
        return None;
    }
    let frame = &ready_frame.linear_frame;
    let (width, height) = (frame.width(), frame.height());

    let _span = tracing::info_span!("noise_resample", target_width, target_height).entered();
    if (target_width, target_height) == (width, height) {
        // Not resampled, so the taps are the identity and carry all of the variance.
        return field.resampled(target_width, target_height, &[1.0], &[1.0]).ok();
    }
    let columns = AxisTaps::cached(width, target_width);
    let rows = AxisTaps::cached(height, target_height);
    field
        .resampled(target_width, target_height, columns.sum_sq(), rows.sum_sq())
        .ok()
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

    let sky_shadow = ready_frame
        .stretch_result
        .as_ref()
        .and_then(|sr| sr.sky_shadow);
    if let (false, Some(shadow)) = (denoise.is_enabled(), sky_shadow) {
        super::sky_shadow_rows::render(source, &tail, display, shadow, &mut output);
        return output;
    }
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

    // Only built for the staged path: it is the only one with a filter to feed, and on
    // the fused path the resample would be work with no reader.
    let noise = output_noise_field(ready_frame, target_width, target_height);
    stage_and_denoise(
        source,
        &tail,
        display,
        &denoise,
        sky_shadow,
        noise.as_ref(),
        &mut output,
        scratch,
    );
    output
}

/// The staged traversal: resample the whole frame to f32 at output resolution,
/// denoise it, then run the tone curve and the 8-bit write per row. A sky shadow
/// streams the denoised rows through the same driver as the fused path: applied to
/// the whole image it held two more full planes (~208 MB at 26 MP native).
#[allow(clippy::too_many_arguments)]
fn stage_and_denoise<S: RowSource>(
    source: &S,
    tail: &RowTail,
    display: DisplayOutput,
    denoise: &DenoiseConfig,
    sky_shadow: Option<crate::render::SkyShadow>,
    noise: Option<&crate::frame::NoiseField>,
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

    // `resample` and `row_tail` are split because only the staged path can tell them
    // apart: the gather scales with *input* pixel count, the tail with *output*.
    // `frame_to_rgb8` reported 75ms as one number (39 from `denoise`, 36 unexplained
    // between the two) — no way to predict what a smaller resolution would save. The fused
    // path above has no equivalent split: it gathers, transforms and writes one row
    // in a single closure against a thread-local scratch row, which is why it's
    // cheaper — splitting would mean per-row spans, which AGENTS.md rules out as
    // distorting what they measure.
    {
        let _span = tracing::info_span!(
            "resample",
            width = target_width,
            height = target_height
        )
        .entered();
        staged
            .par_chunks_mut(row_len)
            .with_min_len(32)
            .enumerate()
            .for_each(|(y, row)| source.gather_row(y, row));
    }

    crate::render::denoise::denoise_rgb_interleaved_with(
        staged,
        target_width,
        target_height,
        denoise,
        noise,
        scratch,
    );

    if let Some(shadow) = sky_shadow {
        let rows = StagedRows {
            rgb: staged,
            width: target_width,
            height: target_height,
        };
        super::sky_shadow_rows::render(&rows, tail, display, shadow, output);
        scratch.staged = owned;
        return;
    }

    {
        let _span = tracing::info_span!("row_tail", samples = staged_len).entered();
        output
            .par_chunks_mut(row_len)
            .zip(staged.par_chunks_mut(row_len))
            .with_min_len(32)
            .enumerate()
            .for_each(|(y, (row_out, row))| {
                tail.apply(row);
                write_row_rgb8(row_out, row, y, display);
            });
    }

    scratch.staged = owned;
}
