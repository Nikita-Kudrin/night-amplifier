//! Grid-node sampling shared by the bilinear and RBF background extractors: lay a grid
//! over the frame, take a star-rejected median in a box around each node, then prune
//! nodes that landed on nebulosity. Extracted after Community and Pro each carried a
//! byte-identical copy of `GridNode` and friends, both reaching pixels through
//! `frame.data()` plus a hand-computed `channel * area` offset — the pattern
//! `AGENTS.md` asks reviewers to flag.
//!
//! Grid *placement* stays unshared: Community hugs the frame boundary so delta-stepping
//! can march between nodes branchlessly; Pro centres nodes in each cell because its TPS
//! solve wants interior samples. Each keeps its own `initialize_grid`.

use crate::frame::Frame;

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

/// Extract the background value for a single node using iterative sigma clipping.
///
/// Collects pixels from a `box_size x box_size` window centred on the node, then applies
/// up to [`SIGMA_CLIP_ITERATIONS`] rounds of sigma clipping to reject bright stars,
/// returning the median of what survives.
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

        let threshold = med + SIGMA_CLIP_THRESHOLD * dispersion * 1.4826;
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

/// A node's star-rejected level plus how rough the sky inside its box is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeSample {
    /// Same value [`extract_node_value`] returns.
    pub value: f32,
    /// Robust sigma of the clip survivors about a plane fitted to them. The plane takes
    /// out a gradient across the box, so what is left is noise plus unresolved
    /// structure — a crowded star field reads rougher than sky at the same level.
    pub scatter: f32,
    /// Clip survivors the two figures were measured on.
    pub samples: usize,
}

/// [`extract_node_value`] plus [`NodeSample::scatter`], for the one channel pruning
/// reads. Kept separate so the per-channel value extraction pays for no plane fit.
pub fn extract_node_sample(
    frame: &Frame,
    node: &GridNode,
    box_size: usize,
    channel: usize,
) -> Option<NodeSample> {
    let (width, height) = (frame.width(), frame.height());
    let half = box_size / 2;
    let (x_start, y_start) = (node.x.saturating_sub(half), node.y.saturating_sub(half));
    let (x_end, y_end) = ((node.x + half + 1).min(width), (node.y + half + 1).min(height));
    if x_start >= x_end || y_start >= y_end {
        return None;
    }

    let plane = frame.channel_data(channel);
    let mut pixels: Vec<[f32; 3]> = (y_start..y_end)
        .flat_map(|y| (x_start..x_end).map(move |x| (x, y)))
        .map(|(x, y)| [(x - x_start) as f32, (y - y_start) as f32, plane[y * width + x]])
        .collect();

    let mut values = Vec::with_capacity(pixels.len());
    let mut mad_buf = Vec::with_capacity(pixels.len());
    let mut early = None;
    for _ in 0..SIGMA_CLIP_ITERATIONS {
        values.clear();
        values.extend(pixels.iter().map(|p| p[2]));
        let med = median(&mut values);
        let dispersion = mad_with_scratch(&values, med, &mut mad_buf);
        if dispersion < 1e-9 {
            break;
        }
        let threshold = med + SIGMA_CLIP_THRESHOLD * dispersion * 1.4826;
        let before = pixels.len();
        pixels.retain(|p| p[2] <= threshold);
        if pixels.is_empty() {
            early = Some(med);
            break;
        }
        if pixels.len() == before {
            break;
        }
    }
    if let Some(value) = early {
        return Some(NodeSample {
            value,
            scatter: 0.0,
            samples: 0,
        });
    }

    values.clear();
    values.extend(pixels.iter().map(|p| p[2]));
    let value = median(&mut values);
    let residuals = plane_residuals(&pixels);
    let centre = median(&mut residuals.clone());
    let scatter = mad(&residuals, centre) * 1.4826;
    Some(NodeSample {
        value,
        scatter,
        samples: pixels.len(),
    })
}

/// Residuals of `[x, y, v]` points about their least-squares plane; about their mean
/// when the fit is degenerate.
fn plane_residuals(points: &[[f32; 3]]) -> Vec<f32> {
    let n = points.len() as f64;
    let mean = |i: usize| points.iter().map(|p| p[i] as f64).sum::<f64>() / n.max(1.0);
    let (mx, my, mv) = (mean(0), mean(1), mean(2));
    let (mut sxx, mut syy, mut sxy, mut sxv, mut syv) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for p in points {
        let (dx, dy, dv) = (p[0] as f64 - mx, p[1] as f64 - my, p[2] as f64 - mv);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
        sxv += dx * dv;
        syv += dy * dv;
    }
    let det = sxx * syy - sxy * sxy;
    let (a, b) = if det.abs() > 1e-12 {
        ((sxv * syy - syv * sxy) / det, (syv * sxx - sxv * sxy) / det)
    } else {
        (0.0, 0.0)
    };
    points
        .iter()
        .map(|p| (p[2] as f64 - mv - a * (p[0] as f64 - mx) - b * (p[1] as f64 - my)) as f32)
        .collect()
}

/// Thresholds for [`prune_nebulosity`].
///
/// Parameterised because the two extractors deliberately disagree: the bilinear grid
/// prunes at 2.5 sigma and a 5 % neighbour excess, RBF at 1.0 sigma and 2 %. RBF is
/// stricter because a thin-plate spline will happily bend through a nebulosity node,
/// where bilinear interpolation only smears it into the two cells that touch it.
#[derive(Debug, Clone, Copy)]
pub struct PruneConfig {
    /// Sigma above the global median at which a node is rejected outright.
    pub global_sigma: f32,
    /// Multiple of the local 8-neighbour median above which a node is rejected.
    pub neighbour_threshold: f32,
}

/// Prune nodes that landed on nebulosity using a two-stage approach:
///
/// 1. **Global rejection**: reject nodes above
///    `global_median + global_sigma * MAD * 1.4826`.
/// 2. **Neighbor rejection**: reject nodes exceeding the local 8-neighbor median by
///    `neighbour_threshold`.
///
/// Stage 2 reads a snapshot taken before it starts, so a node's fate does not depend on
/// whether its neighbours have already been visited.
pub fn prune_nebulosity(
    nodes: &mut [GridNode],
    grid_cols: usize,
    grid_rows: usize,
    config: PruneConfig,
) {
    // Stage 1: Global sigma-based rejection
    let mut all_values: Vec<f32> = nodes.iter().filter_map(|n| n.value).collect();
    if all_values.len() < 4 {
        return;
    }

    let global_median = median(&mut all_values);
    let global_mad = mad(&all_values, global_median);
    let global_threshold = global_median + config.global_sigma * global_mad * 1.4826;

    for node in nodes.iter_mut() {
        if let Some(v) = node.value {
            if v > global_threshold {
                node.value = None;
            }
        }
    }

    // Stage 2: Neighbor-based rejection on survivors
    let snapshot: Vec<Option<f32>> = nodes.iter().map(|n| n.value).collect();

    for node in nodes.iter_mut() {
        let Some(val) = node.value else {
            continue;
        };

        let mut neighbor_values = Vec::with_capacity(8);
        for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let nr = node.row as i32 + dr;
                let nc = node.col as i32 + dc;
                if nr >= 0 && nr < grid_rows as i32 && nc >= 0 && nc < grid_cols as i32 {
                    let idx = nr as usize * grid_cols + nc as usize;
                    if let Some(nv) = snapshot[idx] {
                        neighbor_values.push(nv);
                    }
                }
            }
        }

        if neighbor_values.is_empty() {
            continue;
        }

        let local_median = median(&mut neighbor_values);
        if val > local_median * config.neighbour_threshold {
            node.value = None;
        }
    }
}

#[cfg(test)]
mod tests {
    include!("grid_tests.rs");
}
