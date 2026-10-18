//! Grid-node sampling shared by the bilinear and RBF background extractors: lay nodes
//! over the frame ([`node_axis`]) and take a star-rejected median in a box around each.
//! Which nodes are sky is [`super::classify`]'s job.

use crate::frame::Frame;
use crate::statistics::MAD_TO_SIGMA;

/// Box side length as a fraction of image width.
const BOX_SIZE_PERCENTAGE: f32 = 0.015;

/// Lower bound on the box side, for small frames.
const MIN_BOX_SIZE: usize = 9;

/// Sigma-clipping rounds applied inside one node's box.
const SIGMA_CLIP_ITERATIONS: usize = 3;

/// Sigma threshold for rejecting stars inside a node's box.
const SIGMA_CLIP_THRESHOLD: f32 = 3.0;

/// A grid sample node for background estimation
#[derive(Debug, Clone, Copy)]
pub struct GridNode {
    /// Center x coordinate in pixels
    pub x: usize,
    /// Center y coordinate in pixels
    pub y: usize,
    /// Grid column index (for neighbor lookup)
    pub col: usize,
    /// Grid row index (for neighbor lookup)
    pub row: usize,
    /// Estimated background value (`None` if rejected)
    pub value: Option<f32>,
}

impl GridNode {
    /// A node at `(x, y)` occupying grid cell `(col, row)`, not yet sampled.
    pub const fn new(x: usize, y: usize, col: usize, row: usize) -> Self {
        Self {
            x,
            y,
            col,
            row,
            value: None,
        }
    }
}

/// Odd box side length for a node's sampling window, scaled to the image width.
pub fn compute_box_size(image_width: usize) -> usize {
    let raw = (image_width as f32 * BOX_SIZE_PERCENTAGE) as usize;
    let clamped = raw.max(MIN_BOX_SIZE);
    if clamped.is_multiple_of(2) {
        clamped + 1
    } else {
        clamped
    }
}

/// Median of a mutable slice using O(N) selection instead of an O(N log N) sort.
///
/// Distinct from [`crate::statistics::fast_median`], which orders NaN differently and
/// switches strategy above 4096 elements. The two are not interchangeable and this one
/// is what both extractors have always used; keep them separate.
pub fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mid = values.len() / 2;
    let cmp = |a: &f32, b: &f32| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
    values.select_nth_unstable_by(mid, cmp);
    let median_val = values[mid];
    if values.len().is_multiple_of(2) {
        // The element at mid-1 is the max of the lower partition
        let max_lower = values[..mid]
            .iter()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .copied()
            .unwrap_or(median_val);
        (median_val + max_lower) / 2.0
    } else {
        median_val
    }
}

/// Median Absolute Deviation, reusing `deviations` as scratch.
pub fn mad_with_scratch(values: &[f32], median_value: f32, deviations: &mut Vec<f32>) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    deviations.clear();
    deviations.extend(values.iter().map(|&v| (v - median_value).abs()));
    median(deviations)
}

/// Median Absolute Deviation, allocating its own scratch.
pub fn mad(values: &[f32], median_value: f32) -> f32 {
    let mut deviations = Vec::new();
    mad_with_scratch(values, median_value, &mut deviations)
}

/// Extract the background value for a single node using iterative sigma clipping: collects
/// pixels from a `box_size x box_size` window, applies up to [`SIGMA_CLIP_ITERATIONS`]
/// rounds to reject bright stars, and returns the median of what survives.
///
/// Reads through [`Frame::channel_data`] rather than `frame.data()` plus a
/// `channel * width * height` offset: the plane is a contiguous run, so its rows are
/// slices, and no offset arithmetic has to be got right at the call site.
pub fn extract_node_value(
    frame: &Frame,
    node: &GridNode,
    box_size: usize,
    channel: usize,
) -> Option<f32> {
    let width = frame.width();
    let height = frame.height();
    let half = box_size / 2;

    let x_start = node.x.saturating_sub(half);
    let y_start = node.y.saturating_sub(half);
    let x_end = (node.x + half + 1).min(width);
    let y_end = (node.y + half + 1).min(height);

    if x_start >= x_end || y_start >= y_end {
        return None;
    }

    let plane = frame.channel_data(channel);

    let mut pixels = Vec::with_capacity((x_end - x_start) * (y_end - y_start));
    for y in y_start..y_end {
        let row = y * width;
        pixels.extend_from_slice(&plane[row + x_start..row + x_end]);
    }

    if pixels.is_empty() {
        return None;
    }

    let mut mad_buf = Vec::with_capacity(pixels.len());

    for _ in 0..SIGMA_CLIP_ITERATIONS {
        let med = median(&mut pixels);
        let dispersion = mad_with_scratch(&pixels, med, &mut mad_buf);

        if dispersion < 1e-9 {
            break;
        }

        let threshold = med + SIGMA_CLIP_THRESHOLD * dispersion * MAD_TO_SIGMA;
        let before = pixels.len();
        pixels.retain(|&v| v <= threshold);

        if pixels.is_empty() {
            return Some(med);
        }
        if pixels.len() == before {
            break;
        }
    }

    Some(median(&mut pixels))
}

/// Node positions along one axis of `len` pixels: `nodes` evenly spaced lines hugging
/// both edges (so corners are sampled, not extrapolated), plus one line `inset` (frame
/// fraction) in from each edge. Optical falloff and edge glow live in the outer 3-8 %
/// of real stacks, which a 9 %-spaced grid cannot follow.
pub fn node_axis(len: usize, nodes: usize, inset: f32) -> Vec<usize> {
    let nodes = nodes.max(2);
    let mut axis: Vec<usize> = (0..nodes).map(|i| i * (len - 1) / (nodes - 1)).collect();
    let d = (inset * len as f32) as usize;
    if d > 0 && d < axis[1] && len - 1 - d > axis[nodes - 2] {
        axis.insert(1, d);
        axis.insert(nodes, len - 1 - d);
    }
    axis.dedup();
    axis
}

/// Every channel's star-rejected value at every node: `[channel][node]`.
pub fn sample_nodes(frame: &Frame, nodes: &[GridNode], box_size: usize) -> Vec<Vec<Option<f32>>> {
    use rayon::prelude::*;
    let _span = tracing::info_span!("background_node_sampling", nodes = nodes.len(), box_size).entered();
    (0..frame.channels())
        .into_par_iter()
        .map(|channel| {
            nodes
                .par_iter()
                .map(|node| extract_node_value(frame, node, box_size, channel))
                .collect()
        })
        .collect()
}

/// Every `(x, y)` of `xs × ys`, row-major, as unsampled nodes.
pub fn layout_nodes(xs: &[usize], ys: &[usize]) -> Vec<GridNode> {
    ys.iter()
        .enumerate()
        .flat_map(|(row, &y)| xs.iter().enumerate().map(move |(col, &x)| GridNode::new(x, y, col, row)))
        .collect()
}

#[cfg(test)]
mod tests {
    include!("grid_tests.rs");
}
