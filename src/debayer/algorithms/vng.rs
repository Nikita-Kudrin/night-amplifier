//! VNG (Variable Number of Gradients) debayering algorithm
//!
//! Uses gradient analysis to avoid interpolating across edges.
//! Higher quality than bilinear but slower.
//!
//! This implementation uses SIMD optimization via the `wide` crate to process
//! 4 pixels at a time for gradient computation, providing 2-3x speedup over
//! scalar implementation.

use rayon::prelude::*;
use wide::f32x4;

use crate::debayer::CfaPattern;
use crate::error::Result;
use crate::frame::Frame;

use super::bilinear::bilinear_at;
use super::get_raw;

/// Direction offsets for 8-way sampling: N, NE, E, SE, S, SW, W, NW
const DIRECTIONS: [(isize, isize); 8] = [
    (0, -1),  // N
    (1, -1),  // NE
    (1, 0),   // E
    (1, 1),   // SE
    (0, 1),   // S
    (-1, 1),  // SW
    (-1, 0),  // W
    (-1, -1), // NW
];

/// Perform VNG debayering on a single-channel Bayer frame
///
/// Uses SIMD-optimized gradient computation for interior pixels.
pub fn debayer_vng(frame: &Frame, pattern: CfaPattern) -> Result<Frame> {
    let width = frame.width();
    let height = frame.height();
    let input = frame.data();

    let mut output = vec![0.0f32; width * height * 3];

    output
        .par_chunks_mut(width * 3)
        .enumerate()
        .for_each(|(y, row)| {
            // Process border pixels with scalar fallback
            if y < 2 || y >= height - 2 {
                for x in 0..width {
                    let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                    let out_idx = x * 3;
                    row[out_idx] = r;
                    row[out_idx + 1] = g;
                    row[out_idx + 2] = b;
                }
                return;
            }

            // Process left border (x < 2)
            for x in 0..2 {
                let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                let out_idx = x * 3;
                row[out_idx] = r;
                row[out_idx + 1] = g;
                row[out_idx + 2] = b;
            }

            // Process interior pixels with SIMD (4 at a time)
            let interior_start = 2;
            let interior_end = width - 2;
            let simd_end = interior_start + ((interior_end - interior_start) / 4) * 4;

            // SIMD batch processing
            let mut x = interior_start;
            while x < simd_end {
                process_4_pixels_simd(input, width, height, x, y, pattern, row);
                x += 4;
            }

            // Handle remaining interior pixels (0-3 pixels)
            while x < interior_end {
                let (r, g, b) = vng_at(input, width, height, x, y, pattern);
                let out_idx = x * 3;
                row[out_idx] = r;
                row[out_idx + 1] = g;
                row[out_idx + 2] = b;
                x += 1;
            }

            // Process right border (x >= width - 2)
            for x in interior_end..width {
                let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                let out_idx = x * 3;
                row[out_idx] = r;
                row[out_idx + 1] = g;
                row[out_idx + 2] = b;
            }
        });

    Frame::from_f32_vec(output, width, height, 3)
}

/// Process 4 consecutive pixels using SIMD gradient computation
#[inline]
fn process_4_pixels_simd(
    data: &[f32],
    width: usize,
    height: usize,
    x_start: usize,
    y: usize,
    pattern: CfaPattern,
    row: &mut [f32],
) {
    // Compute gradients for all 4 pixels using SIMD
    let gradients = compute_gradients_simd_4(data, width, height, x_start, y);

    // Process each pixel with its SIMD-computed gradients
    for i in 0..4 {
        let x = x_start + i;
        let pixel_gradients = [
            gradients[0][i],
            gradients[1][i],
            gradients[2][i],
            gradients[3][i],
            gradients[4][i],
            gradients[5][i],
            gradients[6][i],
            gradients[7][i],
        ];

        let (r, g, b) = vng_at_with_gradients(data, width, height, x, y, pattern, &pixel_gradients);
        let out_idx = x * 3;
        row[out_idx] = r;
        row[out_idx + 1] = g;
        row[out_idx + 2] = b;
    }
}

/// Load 4 consecutive f32 values from data at given positions
#[inline]
fn load_4_horizontal(data: &[f32], width: usize, base_x: usize, y: usize) -> f32x4 {
    let idx = y * width + base_x;
    f32x4::new([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]])
}

/// Compute gradients for 4 consecutive pixels using SIMD
///
/// Returns an array of 8 f32x4 vectors, one for each direction.
/// Each f32x4 contains the gradient values for 4 consecutive x positions.
#[inline]
fn compute_gradients_simd_4(
    data: &[f32],
    width: usize,
    height: usize,
    x_start: usize,
    y: usize,
) -> [[f32; 4]; 8] {
    let yi = y as isize;
    let xi = x_start as isize;

    // Load center pixels (4 consecutive)
    let center = load_4_horizontal(data, width, x_start, y);

    // Pre-load all needed rows to minimize memory access
    // For gradient computation we need y-2 to y+2 rows

    // N direction: needs (x, y-2), (x, y-1), (x, y+1)
    let y_m2 = load_4_horizontal(data, width, x_start, (yi - 2) as usize);
    let y_m1 = load_4_horizontal(data, width, x_start, (yi - 1) as usize);
    let y_p1 = load_4_horizontal(data, width, x_start, (yi + 1) as usize);
    let y_p2 = load_4_horizontal(data, width, x_start, (yi + 2) as usize);

    // E direction: needs (x+2, y), (x+1, y), (x-1, y)
    let x_p2 = load_4_horizontal(data, width, (xi + 2) as usize, y);
    let x_p1 = load_4_horizontal(data, width, (xi + 1) as usize, y);
    let x_m1 = load_4_horizontal(data, width, (xi - 1) as usize, y);
    let x_m2 = load_4_horizontal(data, width, (xi - 2) as usize, y);

    // Diagonal points for NE, SE, SW, NW
    let ne_far = load_4_horizontal(data, width, (xi + 2) as usize, (yi - 2) as usize); // (x+2, y-2)
    let ne_near = load_4_horizontal(data, width, (xi + 1) as usize, (yi - 1) as usize); // (x+1, y-1)
    let sw_near = load_4_horizontal(data, width, (xi - 1) as usize, (yi + 1) as usize); // (x-1, y+1)

    let se_far = load_4_horizontal(data, width, (xi + 2) as usize, (yi + 2) as usize); // (x+2, y+2)
    let se_near = load_4_horizontal(data, width, (xi + 1) as usize, (yi + 1) as usize); // (x+1, y+1)
    let nw_near = load_4_horizontal(data, width, (xi - 1) as usize, (yi - 1) as usize); // (x-1, y-1)

    let sw_far = load_4_horizontal(data, width, (xi - 2) as usize, (yi + 2) as usize); // (x-2, y+2)
    let nw_far = load_4_horizontal(data, width, (xi - 2) as usize, (yi - 2) as usize); // (x-2, y-2)

    // Compute 8 gradients using SIMD
    // Each gradient = |far - center| + |near1 - near2|

    // N: |y-2 - center| + |y-1 - y+1|
    let grad_n = (y_m2 - center).abs() + (y_m1 - y_p1).abs();

    // NE: |x+2,y-2 - center| + |x+1,y-1 - x-1,y+1|
    let grad_ne = (ne_far - center).abs() + (ne_near - sw_near).abs();

    // E: |x+2 - center| + |x+1 - x-1|
    let grad_e = (x_p2 - center).abs() + (x_p1 - x_m1).abs();

    // SE: |x+2,y+2 - center| + |x+1,y+1 - x-1,y-1|
    let grad_se = (se_far - center).abs() + (se_near - nw_near).abs();

    // S: |y+2 - center| + |y+1 - y-1|
    let grad_s = (y_p2 - center).abs() + (y_p1 - y_m1).abs();

    // SW: |x-2,y+2 - center| + |x-1,y+1 - x+1,y-1|
    let grad_sw = (sw_far - center).abs() + (sw_near - ne_near).abs();

    // W: |x-2 - center| + |x-1 - x+1|
    let grad_w = (x_m2 - center).abs() + (x_m1 - x_p1).abs();

    // NW: |x-2,y-2 - center| + |x-1,y-1 - x+1,y+1|
    let grad_nw = (nw_far - center).abs() + (nw_near - se_near).abs();

    // Extract results to arrays
    [
        grad_n.to_array(),
        grad_ne.to_array(),
        grad_e.to_array(),
        grad_se.to_array(),
        grad_s.to_array(),
        grad_sw.to_array(),
        grad_w.to_array(),
        grad_nw.to_array(),
    ]
}

/// VNG interpolation at a single pixel with pre-computed gradients
///
/// This is the fast path used by SIMD processing - gradients are computed
/// in batches of 4 using SIMD, then this function handles the rest of the
/// VNG algorithm for each pixel.
#[inline]
fn vng_at_with_gradients(
    data: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    pattern: CfaPattern,
    gradients: &[f32; 8],
) -> (f32, f32, f32) {
    let color = pattern.color_at(x, y);
    let xi = x as isize;
    let yi = y as isize;

    let threshold = compute_threshold(gradients);

    let (r_sum, g_sum, b_sum, count) =
        accumulate_low_gradient_samples(data, width, height, xi, yi, pattern, gradients, threshold);

    let this = data[y * width + x];

    if count == 0 {
        return bilinear_at(data, width, height, x, y, pattern);
    }

    combine_with_actual(
        data, width, height, x, y, pattern, color, this, r_sum, g_sum, b_sum, count,
    )
}

/// VNG interpolation at a single pixel (scalar fallback)
fn vng_at(
    data: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    pattern: CfaPattern,
) -> (f32, f32, f32) {
    let color = pattern.color_at(x, y);
    let xi = x as isize;
    let yi = y as isize;

    let gradients = compute_gradients(data, width, height, xi, yi);
    let threshold = compute_threshold(&gradients);

    let (r_sum, g_sum, b_sum, count) = accumulate_low_gradient_samples(
        data, width, height, xi, yi, pattern, &gradients, threshold,
    );

    let this = data[y * width + x];

    if count == 0 {
        return bilinear_at(data, width, height, x, y, pattern);
    }

    combine_with_actual(
        data, width, height, x, y, pattern, color, this, r_sum, g_sum, b_sum, count,
    )
}

/// Compute gradients in 8 directions
fn compute_gradients(data: &[f32], width: usize, height: usize, xi: isize, yi: isize) -> [f32; 8] {
    let center = get_raw(data, width, height, xi, yi);

    [
        // N
        (get_raw(data, width, height, xi, yi - 2) - center).abs()
            + (get_raw(data, width, height, xi, yi - 1) - get_raw(data, width, height, xi, yi + 1))
                .abs(),
        // NE
        (get_raw(data, width, height, xi + 2, yi - 2) - center).abs()
            + (get_raw(data, width, height, xi + 1, yi - 1)
                - get_raw(data, width, height, xi - 1, yi + 1))
            .abs(),
        // E
        (get_raw(data, width, height, xi + 2, yi) - center).abs()
            + (get_raw(data, width, height, xi + 1, yi) - get_raw(data, width, height, xi - 1, yi))
                .abs(),
        // SE
        (get_raw(data, width, height, xi + 2, yi + 2) - center).abs()
            + (get_raw(data, width, height, xi + 1, yi + 1)
                - get_raw(data, width, height, xi - 1, yi - 1))
            .abs(),
        // S
        (get_raw(data, width, height, xi, yi + 2) - center).abs()
            + (get_raw(data, width, height, xi, yi + 1) - get_raw(data, width, height, xi, yi - 1))
                .abs(),
        // SW
        (get_raw(data, width, height, xi - 2, yi + 2) - center).abs()
            + (get_raw(data, width, height, xi - 1, yi + 1)
                - get_raw(data, width, height, xi + 1, yi - 1))
            .abs(),
        // W
        (get_raw(data, width, height, xi - 2, yi) - center).abs()
            + (get_raw(data, width, height, xi - 1, yi) - get_raw(data, width, height, xi + 1, yi))
                .abs(),
        // NW
        (get_raw(data, width, height, xi - 2, yi - 2) - center).abs()
            + (get_raw(data, width, height, xi - 1, yi - 1)
                - get_raw(data, width, height, xi + 1, yi + 1))
            .abs(),
    ]
}

/// Compute adaptive threshold from gradients
#[inline]
fn compute_threshold(gradients: &[f32; 8]) -> f32 {
    let min_grad = gradients.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_grad = gradients.iter().cloned().fold(0.0f32, f32::max);
    min_grad + (max_grad - min_grad) * 0.5
}

/// Accumulate color contributions from directions with low gradients
fn accumulate_low_gradient_samples(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    pattern: CfaPattern,
    gradients: &[f32; 8],
    threshold: f32,
) -> (f32, f32, f32, usize) {
    let mut r_sum = 0.0f32;
    let mut g_sum = 0.0f32;
    let mut b_sum = 0.0f32;
    let mut count = 0;

    for (i, &grad) in gradients.iter().enumerate() {
        if grad <= threshold {
            let (dx, dy) = DIRECTIONS[i];
            let nx = xi + dx;
            let ny = yi + dy;
            let neighbor_color = pattern.color_at(nx as usize, ny as usize);
            let val = get_raw(data, width, height, nx, ny);

            match neighbor_color {
                0 => r_sum += val,
                1 => g_sum += val,
                2 => b_sum += val,
                _ => {}
            }
            count += 1;
        }
    }

    (r_sum, g_sum, b_sum, count)
}

/// Combine actual pixel value with interpolated values
#[allow(clippy::too_many_arguments)]
fn combine_with_actual(
    data: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    pattern: CfaPattern,
    color: usize,
    this: f32,
    r_sum: f32,
    g_sum: f32,
    b_sum: f32,
    count: usize,
) -> (f32, f32, f32) {
    let count_f = count as f32;

    match color {
        0 => {
            // Red pixel
            let g = if g_sum > 0.0 {
                g_sum / count_f
            } else {
                bilinear_at(data, width, height, x, y, pattern).1
            };
            let b = if b_sum > 0.0 {
                b_sum / count_f
            } else {
                bilinear_at(data, width, height, x, y, pattern).2
            };
            (this, g, b)
        }
        1 => {
            // Green pixel
            let r = if r_sum > 0.0 {
                r_sum / count_f
            } else {
                bilinear_at(data, width, height, x, y, pattern).0
            };
            let b = if b_sum > 0.0 {
                b_sum / count_f
            } else {
                bilinear_at(data, width, height, x, y, pattern).2
            };
            (r, this, b)
        }
        2 => {
            // Blue pixel
            let r = if r_sum > 0.0 {
                r_sum / count_f
            } else {
                bilinear_at(data, width, height, x, y, pattern).0
            };
            let g = if g_sum > 0.0 {
                g_sum / count_f
            } else {
                bilinear_at(data, width, height, x, y, pattern).1
            };
            (r, g, this)
        }
        _ => unreachable!(),
    }
}
