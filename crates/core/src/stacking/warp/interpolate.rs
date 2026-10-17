use wide::f32x4;

/// Performs bilinear interpolation on a single channel.
#[inline]
pub fn bilinear_interpolate_direct_1ch(data: &[f32], width: usize, sx: f32, sy: f32) -> f32 {
    let x0 = sx.floor() as usize;
    let y0 = sy.floor() as usize;

    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;

    let weights = f32x4::new([
        (1.0 - fx) * (1.0 - fy),
        fx * (1.0 - fy),
        (1.0 - fx) * fy,
        fx * fy,
    ]);

    let row_stride = width;
    let row0_offset = y0 * row_stride;
    let row1_offset = row0_offset + row_stride;

    let base00 = row0_offset + x0;
    let base10 = base00 + 1;
    let base01 = row1_offset + x0;
    let base11 = base01 + 1;

    let corners = f32x4::new([data[base00], data[base10], data[base01], data[base11]]);
    (corners * weights).reduce_add()
}

/// Specialized RGB row warping with unrolled interpolation.
#[inline]
pub fn warp_row_rgb(
    src_r: &[f32],
    src_g: &[f32],
    src_b: &[f32],
    width: usize,
    _height: usize,
    r_row: &mut [f32],
    g_row: &mut [f32],
    b_row: &mut [f32],
    border_value: f32,
    sx_start: f32,
    sy_start: f32,
    sx_step: f32,
    sy_step: f32,
    max_sx: f32,
    max_sy: f32,
) {
    let row_stride = width;

    let (x_start, x_end) =
        calculate_valid_x_range(width, sx_start, sy_start, sx_step, sy_step, max_sx, max_sy);

    for dx in 0..x_start {
        r_row[dx] = border_value;
        g_row[dx] = border_value;
        b_row[dx] = border_value;
    }

    if x_start < x_end {
        let mut sx = sx_start + (x_start as f32) * sx_step;
        let mut sy = sy_start + (x_start as f32) * sy_step;

        for dx in x_start..x_end {
            let x0 = sx.floor() as usize;
            let y0 = sy.floor() as usize;

            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;

            let w00 = (1.0 - fx) * (1.0 - fy);
            let w10 = fx * (1.0 - fy);
            let w01 = (1.0 - fx) * fy;
            let w11 = fx * fy;
            let weights = f32x4::new([w00, w10, w01, w11]);

            let base00 = y0 * row_stride + x0;
            let base10 = base00 + 1;
            let base01 = base00 + row_stride;
            let base11 = base01 + 1;

            let r_corners =
                f32x4::new([src_r[base00], src_r[base10], src_r[base01], src_r[base11]]);
            r_row[dx] = (r_corners * weights).reduce_add();

            let g_corners =
                f32x4::new([src_g[base00], src_g[base10], src_g[base01], src_g[base11]]);
            g_row[dx] = (g_corners * weights).reduce_add();

            let b_corners =
                f32x4::new([src_b[base00], src_b[base10], src_b[base01], src_b[base11]]);
            b_row[dx] = (b_corners * weights).reduce_add();

            sx += sx_step;
            sy += sy_step;
        }
    }

    for dx in x_end..width {
        r_row[dx] = border_value;
        g_row[dx] = border_value;
        b_row[dx] = border_value;
    }
}

/// Calculate the valid x-range where source coordinates are within bounds.
#[inline]
pub fn calculate_valid_x_range(
    width: usize,
    sx_start: f32,
    sy_start: f32,
    sx_step: f32,
    sy_step: f32,
    max_sx: f32,
    max_sy: f32,
) -> (usize, usize) {
    let mut x_min = 0.0f32;
    let mut x_max = width as f32;

    // Handle sx constraints
    if sx_step.abs() > 1e-10 {
        let x_for_sx_zero = -sx_start / sx_step;
        let x_for_sx_max = (max_sx - sx_start) / sx_step;

        if sx_step > 0.0 {
            x_min = x_min.max(x_for_sx_zero);
            x_max = x_max.min(x_for_sx_max);
        } else {
            x_min = x_min.max(x_for_sx_max);
            x_max = x_max.min(x_for_sx_zero);
        }
    } else {
        if sx_start < 0.0 || sx_start >= max_sx {
            return (0, 0);
        }
    }

    // Handle sy constraints
    if sy_step.abs() > 1e-10 {
        let x_for_sy_zero = -sy_start / sy_step;
        let x_for_sy_max = (max_sy - sy_start) / sy_step;

        if sy_step > 0.0 {
            x_min = x_min.max(x_for_sy_zero);
            x_max = x_max.min(x_for_sy_max);
        } else {
            x_min = x_min.max(x_for_sy_max);
            x_max = x_max.min(x_for_sy_zero);
        }
    } else {
        if sy_start < 0.0 || sy_start >= max_sy {
            return (0, 0);
        }
    }

    let x_start = (x_min.ceil() as usize).min(width);
    let x_end = (x_max.floor() as usize).min(width);

    if x_start >= x_end {
        (0, 0)
    } else {
        (x_start, x_end)
    }
}
