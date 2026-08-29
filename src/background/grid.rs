//! Grid-node sampling shared by the bilinear and RBF background extractors.
//!
//! Both extractors do the same first two phases — lay a grid over the frame, take a
//! star-rejected median in a box around each node, then prune the nodes that landed on
//! nebulosity — and differ only afterwards, in how they interpolate between the
//! survivors. Until this module existed, Community's `background::extractor` and Pro's
//! `plugins::rbf` each carried their own byte-identical copy of `GridNode`,
//! [`compute_box_size`], [`extract_node_value`], [`median`], [`mad`] and
//! [`prune_nebulosity`], and both reached the pixels through `frame.data()` plus a
//! hand-computed `channel * area` offset — the pattern `AGENTS.md` asks reviewers to
//! flag. A fix to one would have had to be found and repeated in the other repository.
//!
//! What is *not* shared is grid placement. Community hugs the frame boundaries so its
//! delta-stepping subtraction can march between nodes branchlessly; Pro centres a node
//! in each cell because the TPS solve wants interior samples. Each keeps its own
//! `initialize_grid`.

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
    use super::*;

    #[test]
    fn box_size_is_always_odd_and_at_least_the_floor() {
        // 2712 * 0.015 = 40.68 -> 40 -> 41
        assert_eq!(compute_box_size(2712), 41);
        // Below the floor, the floor wins and is already odd.
        assert_eq!(compute_box_size(100), MIN_BOX_SIZE);
        for width in [64usize, 200, 640, 1936, 3008, 4144] {
            assert!(!compute_box_size(width).is_multiple_of(2));
        }
    }

    #[test]
    fn median_handles_both_parities_and_the_empty_case() {
        assert_eq!(median(&mut []), 0.0);
        assert_eq!(median(&mut [5.0]), 5.0);
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn mad_and_mad_with_scratch_agree() {
        let values = [1.0f32, 2.0, 3.0, 10.0];
        let med = median(&mut values.to_vec());
        let mut scratch = vec![0.0; 99];
        assert_eq!(mad(&values, med), mad_with_scratch(&values, med, &mut scratch));
        assert_eq!(mad(&[], 0.0), 0.0);
    }

    /// A star inside the box must be clipped away, leaving the sky level.
    #[test]
    fn node_extraction_rejects_a_star_in_the_box() {
        let mut frame = Frame::filled(64, 64, 1, 0.1).unwrap();
        for y in 30..34 {
            for x in 30..34 {
                frame.set_pixel(x, y, 0, 0.95);
            }
        }
        let node = GridNode::new(32, 32, 0, 0);
        let value = extract_node_value(&frame, &node, 21, 0).unwrap();
        assert!(
            (value - 0.1).abs() < 1e-4,
            "sigma clipping should have left the 0.1 sky level, got {value}"
        );
    }

    /// The node reads the plane it was asked for, not plane 0 with an offset slip.
    /// A colour fixture is what makes that observable at all.
    #[test]
    fn node_extraction_reads_the_requested_plane() {
        let mut frame = Frame::zeros(32, 32, 3).unwrap();
        for y in 0..32 {
            for x in 0..32 {
                frame.set_pixel(x, y, 0, 0.10);
                frame.set_pixel(x, y, 1, 0.50);
                frame.set_pixel(x, y, 2, 0.90);
            }
        }
        let node = GridNode::new(16, 16, 0, 0);
        for (channel, want) in [(0usize, 0.10f32), (1, 0.50), (2, 0.90)] {
            let got = extract_node_value(&frame, &node, 9, channel).unwrap();
            assert!((got - want).abs() < 1e-6, "channel {channel}: {got} != {want}");
        }
    }

    /// A node whose box falls entirely outside the frame has nothing to sample.
    #[test]
    fn a_node_outside_the_frame_yields_nothing() {
        let frame = Frame::filled(16, 16, 1, 0.2).unwrap();
        let node = GridNode::new(64, 64, 0, 0);
        assert!(extract_node_value(&frame, &node, 9, 0).is_none());
    }

    #[test]
    fn pruning_rejects_a_bright_node_and_keeps_the_sky() {
        let cols = 4;
        let rows = 4;
        let mut nodes: Vec<GridNode> = (0..rows)
            .flat_map(|row| (0..cols).map(move |col| GridNode::new(col, row, col, row)))
            .collect();
        for node in nodes.iter_mut() {
            node.value = Some(0.1);
        }
        nodes[5].value = Some(0.9);

        prune_nebulosity(
            &mut nodes,
            cols,
            rows,
            PruneConfig {
                global_sigma: 2.5,
                neighbour_threshold: 1.05,
            },
        );

        assert!(nodes[5].value.is_none(), "the bright node should be pruned");
        assert_eq!(
            nodes.iter().filter(|n| n.value.is_some()).count(),
            cols * rows - 1,
            "only the bright node should be pruned"
        );
    }

    /// A stricter config prunes strictly more. Pins that the thresholds are actually
    /// wired through rather than shadowed by a constant.
    #[test]
    fn a_stricter_config_prunes_at_least_as_much() {
        let cols = 6;
        let rows = 6;
        let build = || {
            let mut nodes: Vec<GridNode> = (0..rows)
                .flat_map(|row| (0..cols).map(move |col| GridNode::new(col, row, col, row)))
                .collect();
            for (i, node) in nodes.iter_mut().enumerate() {
                node.value = Some(0.1 + (i % 5) as f32 * 0.01);
            }
            nodes
        };

        let mut lenient = build();
        prune_nebulosity(
            &mut lenient,
            cols,
            rows,
            PruneConfig {
                global_sigma: 2.5,
                neighbour_threshold: 1.05,
            },
        );

        let mut strict = build();
        prune_nebulosity(
            &mut strict,
            cols,
            rows,
            PruneConfig {
                global_sigma: 1.0,
                neighbour_threshold: 1.02,
            },
        );

        let survivors = |n: &[GridNode]| n.iter().filter(|g| g.value.is_some()).count();
        assert!(
            survivors(&strict) < survivors(&lenient),
            "strict {} vs lenient {} — the thresholds are not reaching the algorithm",
            survivors(&strict),
            survivors(&lenient)
        );
    }
}
