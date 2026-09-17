//! The sky shadow, without a whole-image luminance and guide plane.
//!
//! The shadow reads a 3x3 neighbourhood of the *tail's* output, which is why it used to
//! force the staged traversal: a 1440² frame as f32 plus two guide planes, ~40 MB, and
//! +2.5 ms per encode on x86 with denoise off. Here each parallel chunk renders its rows
//! plus one context row either side, so a worker holds ~34 rows. The denoised path feeds
//! its staged image in as a row source too. The sky is measured on the same fixed sample
//! rows as `apply_sky_shadow_interleaved`, the whole-image reference, so the bytes agree
//! (`sky_shadow_streaming_matches_staged`, `sky_shadow_after_denoise_matches_the_whole_image_reference`).

use std::cell::RefCell;

use rayon::prelude::*;

use crate::render::output::{
    guide_row, luma_row, sky_sample_rows, sky_sample_stride, write_row_rgb8, DisplayOutput,
};
use crate::render::SkyShadow;

use super::fused::{RowSource, RowTail};

/// Output rows per parallel unit; each renders two more for context.
const CHUNK_ROWS: usize = 32;

#[derive(Default)]
struct Rows {
    rgb: Vec<f32>,
    luma: Vec<f32>,
    guide: Vec<f32>,
}

thread_local! {
    static ROWS: RefCell<Rows> = RefCell::new(Rows::default());
}

pub(super) fn render<S: RowSource>(
    source: &S,
    tail: &RowTail,
    display: DisplayOutput,
    shadow: SkyShadow,
    output: &mut [u8],
) {
    let (width, height) = (source.target_width(), source.target_height());
    if width == 0 || height == 0 {
        return;
    }
    let row_len = width * 3;
    let _span = tracing::info_span!("sky_shadow_rows", width, height).entered();

    let render_rows = |first: usize, last: usize, rows: &mut Rows| {
        let count = last - first;
        rows.rgb.resize(count * row_len, 0.0);
        rows.luma.resize(count * width, 0.0);
        rows.guide.resize(width, 0.0);
        for (k, y) in (first..last).enumerate() {
            let rgb = &mut rows.rgb[k * row_len..(k + 1) * row_len];
            source.gather_row(y, rgb);
            tail.apply(rgb);
            luma_row(rgb, &mut rows.luma[k * width..(k + 1) * width]);
        }
    };
    let neighbours = |y: usize| (y.saturating_sub(1), (y + 1).min(height - 1));

    let stride = sky_sample_stride(width);
    let samples: Vec<f32> = sky_sample_rows(height)
        .par_iter()
        .flat_map_iter(|&y| {
            ROWS.with(|cell| {
                let rows = &mut *cell.borrow_mut();
                let (first, last) = (y.saturating_sub(1), (y + 2).min(height));
                render_rows(first, last, rows);
                let (up, down) = neighbours(y);
                let luma = |r: usize| &rows.luma[(r - first) * width..(r - first + 1) * width];
                guide_row(luma(up), luma(y), luma(down), &mut rows.guide);
                rows.guide.iter().step_by(stride).copied().collect::<Vec<_>>()
            })
        })
        .collect();
    let shadow = shadow.with_measured_sky(&samples);

    output
        .par_chunks_mut(row_len * CHUNK_ROWS)
        .enumerate()
        .for_each(|(chunk, out)| {
            ROWS.with(|cell| {
                let rows = &mut *cell.borrow_mut();
                let y0 = chunk * CHUNK_ROWS;
                let y1 = y0 + out.len() / row_len;
                let first = y0.saturating_sub(1);
                render_rows(first, (y1 + 1).min(height), rows);
                let at = |r: usize| (r - first) * width..(r - first + 1) * width;
                for y in y0..y1 {
                    let (up, down) = neighbours(y);
                    guide_row(&rows.luma[at(up)], &rows.luma[at(y)], &rows.luma[at(down)], &mut rows.guide);
                    let pixels = &mut rows.rgb[(y - first) * row_len..(y - first + 1) * row_len];
                    shadow.apply_row(pixels, &rows.luma[at(y)], &rows.guide);
                    write_row_rgb8(&mut out[(y - y0) * row_len..(y - y0 + 1) * row_len], pixels, y, display);
                }
            })
        });
}
