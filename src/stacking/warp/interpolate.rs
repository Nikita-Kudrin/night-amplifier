/// Performs bilinear interpolation with direct data access.
///
/// Takes raw data slice instead of Frame reference to avoid
/// per-pixel index recalculation overhead.
#[inline]
pub fn bilinear_interpolate_direct(
    data: &[f32],
    width: usize,
    channels: usize,
    sx: f32,
    sy: f32,
    output: &mut [f32],
) {
    let x0 = sx.floor() as usize;
    let y0 = sy.floor() as usize;

    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;

    // Pre-compute weights
    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;

    // Pre-compute row offsets
    let row_stride = width * channels;
    let row0_offset = y0 * row_stride;
    let row1_offset = row0_offset + row_stride;
    let col0_offset = x0 * channels;
    let col1_offset = col0_offset + channels;

    // Pre-compute base indices for 4 corner pixels
    let base00 = row0_offset + col0_offset;
    let base10 = row0_offset + col1_offset;
    let base01 = row1_offset + col0_offset;
    let base11 = row1_offset + col1_offset;

    for c in 0..channels {
        output[c] = w00 * data[base00 + c]
            + w10 * data[base10 + c]
            + w01 * data[base01 + c]
            + w11 * data[base11 + c];
    }
}

/// Specialized RGB row warping with unrolled interpolation.
#[inline]
pub fn warp_row_rgb(
    src_data: &[f32],
    width: usize,
    _height: usize,
    row: &mut [f32],
    border_value: f32,
    sx_start: f32,
    sy_start: f32,
    sx_step: f32,
    sy_step: f32,
    max_sx: f32,
    max_sy: f32,
) {
    let row_stride = width * 3;

    // Calculate the valid x-range where source coordinates are in bounds
    let (x_start, x_end) =
        calculate_valid_x_range(width, sx_start, sy_start, sx_step, sy_step, max_sx, max_sy);

    // Fill border for [0, x_start)
    for dx in 0..x_start {
        let out_idx = dx * 3;
        row[out_idx] = border_value;
        row[out_idx + 1] = border_value;
        row[out_idx + 2] = border_value;
    }

    // Process valid range without bounds checks
    if x_start < x_end {
        let mut sx = sx_start + (x_start as f32) * sx_step;
        let mut sy = sy_start + (x_start as f32) * sy_step;

        for dx in x_start..x_end {
            let out_idx = dx * 3;

            // Compute integer and fractional parts
            let x0 = sx.floor() as usize;
            let y0 = sy.floor() as usize;

            let fx = sx - x0 as f32;
            let fy = sy - y0 as f32;

            // Pre-compute weights
            let w00 = (1.0 - fx) * (1.0 - fy);
            let w10 = fx * (1.0 - fy);
            let w01 = (1.0 - fx) * fy;
            let w11 = fx * fy;

            // Compute base indices (row-major, 3 channels)
            let base00 = y0 * row_stride + x0 * 3;
            let base10 = base00 + 3;
            let base01 = base00 + row_stride;
            let base11 = base01 + 3;

            // Unrolled RGB interpolation
            row[out_idx] = w00 * src_data[base00]
                + w10 * src_data[base10]
                + w01 * src_data[base01]
                + w11 * src_data[base11];

            row[out_idx + 1] = w00 * src_data[base00 + 1]
                + w10 * src_data[base10 + 1]
                + w01 * src_data[base01 + 1]
                + w11 * src_data[base11 + 1];

            row[out_idx + 2] = w00 * src_data[base00 + 2]
                + w10 * src_data[base10 + 2]
                + w01 * src_data[base01 + 2]
                + w11 * src_data[base11 + 2];

            sx += sx_step;
            sy += sy_step;
        }
    }

    // Fill border for [x_end, width)
    for dx in x_end..width {
        let out_idx = dx * 3;
        row[out_idx] = border_value;
        row[out_idx + 1] = border_value;
        row[out_idx + 2] = border_value;
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
