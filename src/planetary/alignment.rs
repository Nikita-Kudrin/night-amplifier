//! Frame alignment using cross-correlation for planetary stacking.

use crate::frame::Frame;

use super::config::AlignmentRoi;
use super::quality::{compute_std_dev, frame_to_luminance};

/// Computes alignment offset using cross-correlation.
///
/// Returns (dx, dy) offset to align frame to reference.
pub fn compute_alignment(
    reference: &Frame,
    frame: &Frame,
    roi: &AlignmentRoi,
    search_radius: usize,
    subpixel_factor: usize,
) -> (f32, f32) {
    let ref_lum = frame_to_luminance(reference);
    let frame_lum = frame_to_luminance(frame);

    let width = reference.width();
    let height = reference.height();

    let clamped_roi = clamp_roi(roi, width, height);
    let (roi_x, roi_y, roi_w, roi_h) = clamped_roi;

    let ref_roi = extract_roi(&ref_lum, width, roi_x, roi_y, roi_w, roi_h);
    let ref_mean = ref_roi.iter().sum::<f32>() / ref_roi.len() as f32;
    let ref_std = compute_std_dev(&ref_roi);

    if ref_std < 1e-6 {
        return (0.0, 0.0);
    }

    let (best_dx, best_dy) = search_best_correlation(
        &frame_lum,
        &ref_roi,
        ref_mean,
        ref_std,
        width,
        height,
        roi_x,
        roi_y,
        roi_w,
        roi_h,
        search_radius,
    );

    if subpixel_factor > 1
        && best_dx.abs() < search_radius as i32
        && best_dy.abs() < search_radius as i32
    {
        let (sub_dx, sub_dy) = subpixel_refine(
            &frame_lum, &ref_roi, ref_mean, ref_std, width, height, roi_x, roi_y, roi_w, roi_h,
            best_dx, best_dy,
        );
        return (best_dx as f32 + sub_dx, best_dy as f32 + sub_dy);
    }

    (best_dx as f32, best_dy as f32)
}

/// Clamps ROI to valid bounds and returns (x, y, w, h).
fn clamp_roi(roi: &AlignmentRoi, width: usize, height: usize) -> (usize, usize, usize, usize) {
    let roi_x = roi.x.min(width.saturating_sub(roi.width));
    let roi_y = roi.y.min(height.saturating_sub(roi.height));
    let roi_w = roi.width.min(width - roi_x);
    let roi_h = roi.height.min(height - roi_y);
    (roi_x, roi_y, roi_w, roi_h)
}

/// Searches for the best correlation within the search radius.
fn search_best_correlation(
    frame_lum: &[f32],
    ref_roi: &[f32],
    ref_mean: f32,
    ref_std: f32,
    width: usize,
    height: usize,
    roi_x: usize,
    roi_y: usize,
    roi_w: usize,
    roi_h: usize,
    search_radius: usize,
) -> (i32, i32) {
    let mut best_corr = f32::MIN;
    let mut best_dx = 0i32;
    let mut best_dy = 0i32;
    let search = search_radius as i32;

    for dy in -search..=search {
        for dx in -search..=search {
            let shifted_x = (roi_x as i32 + dx).max(0) as usize;
            let shifted_y = (roi_y as i32 + dy).max(0) as usize;

            if shifted_x + roi_w > width || shifted_y + roi_h > height {
                continue;
            }

            let frame_roi = extract_roi(frame_lum, width, shifted_x, shifted_y, roi_w, roi_h);

            if let Some(ncc) = compute_ncc(ref_roi, &frame_roi, ref_mean, ref_std) {
                if ncc > best_corr {
                    best_corr = ncc;
                    best_dx = dx;
                    best_dy = dy;
                }
            }
        }
    }

    (best_dx, best_dy)
}

/// Computes normalized cross-correlation between two ROIs.
fn compute_ncc(ref_roi: &[f32], frame_roi: &[f32], ref_mean: f32, ref_std: f32) -> Option<f32> {
    let frame_mean = frame_roi.iter().sum::<f32>() / frame_roi.len() as f32;
    let frame_std = compute_std_dev(frame_roi);

    if frame_std < 1e-6 {
        return None;
    }

    let mut ncc = 0.0f32;
    for i in 0..ref_roi.len() {
        ncc += (ref_roi[i] - ref_mean) * (frame_roi[i] - frame_mean);
    }
    ncc /= ref_roi.len() as f32 * ref_std * frame_std;

    Some(ncc)
}

/// Extracts a region of interest from luminance data.
fn extract_roi(
    lum: &[f32],
    width: usize,
    x: usize,
    y: usize,
    roi_w: usize,
    roi_h: usize,
) -> Vec<f32> {
    let mut roi = Vec::with_capacity(roi_w * roi_h);
    for ry in 0..roi_h {
        for rx in 0..roi_w {
            roi.push(lum[(y + ry) * width + (x + rx)]);
        }
    }
    roi
}

/// Subpixel refinement using parabolic interpolation.
fn subpixel_refine(
    frame_lum: &[f32],
    ref_roi: &[f32],
    ref_mean: f32,
    ref_std: f32,
    width: usize,
    height: usize,
    roi_x: usize,
    roi_y: usize,
    roi_w: usize,
    roi_h: usize,
    best_dx: i32,
    best_dy: i32,
) -> (f32, f32) {
    let mut corr_grid = [[0.0f32; 3]; 3];

    for (iy, dy_offset) in [-1i32, 0, 1].iter().enumerate() {
        for (ix, dx_offset) in [-1i32, 0, 1].iter().enumerate() {
            let dx = best_dx + dx_offset;
            let dy = best_dy + dy_offset;

            let shifted_x = (roi_x as i32 + dx).max(0) as usize;
            let shifted_y = (roi_y as i32 + dy).max(0) as usize;

            if shifted_x + roi_w <= width && shifted_y + roi_h <= height {
                let frame_roi = extract_roi(frame_lum, width, shifted_x, shifted_y, roi_w, roi_h);
                if let Some(ncc) = compute_ncc(ref_roi, &frame_roi, ref_mean, ref_std) {
                    corr_grid[iy][ix] = ncc;
                }
            }
        }
    }

    let sub_x = parabolic_fit(corr_grid[1][0], corr_grid[1][1], corr_grid[1][2]);
    let sub_y = parabolic_fit(corr_grid[0][1], corr_grid[1][1], corr_grid[2][1]);

    (sub_x.clamp(-0.5, 0.5), sub_y.clamp(-0.5, 0.5))
}

/// Fits a parabola to three samples and returns the subpixel offset.
fn parabolic_fit(c0: f32, c1: f32, c2: f32) -> f32 {
    let denom = c0 + c2 - 2.0 * c1;
    if denom.abs() > 1e-6 {
        (c0 - c2) / (2.0 * denom)
    } else {
        0.0
    }
}
