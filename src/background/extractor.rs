use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;
use tracing::{debug, instrument, warn};

use super::config::{BackgroundConfig, BackgroundExtractionAlgorithm};
use super::model::BackgroundModel;

/// Box size as a percentage of image width (1.5%)
const BOX_SIZE_PERCENTAGE: f32 = 0.015;

/// Minimum box size in pixels for reliable median estimation
const MIN_BOX_SIZE: usize = 9;

/// Number of iterations for sigma clipping star rejection
const SIGMA_CLIP_ITERATIONS: usize = 3;

/// Sigma threshold for star rejection within a sample box
const SIGMA_CLIP_THRESHOLD: f32 = 3.0;

/// Sigma threshold for global nebulosity rejection
const GLOBAL_PRUNING_SIGMA: f32 = 2.5;

/// Threshold for nebulosity rejection: node value must not exceed
/// the neighbor median by more than this factor (5%)
const NEBULOSITY_THRESHOLD: f32 = 1.05;

/// Minimum surviving nodes before falling back to flat-field subtraction
const MIN_VALID_NODES: usize = 4;

/// A grid sample node for background estimation
#[derive(Debug, Clone, Copy)]
struct GridNode {
    /// Center x coordinate in pixels
    x: usize,
    /// Center y coordinate in pixels
    y: usize,
    /// Grid column index (for neighbor lookup)
    col: usize,
    /// Grid row index (for neighbor lookup)
    row: usize,
    /// Estimated background value (`None` if rejected)
    value: Option<f32>,
}

/// Completed bilinear grid model ready for evaluation
struct BilinearModel {
    /// Grid values per channel: [channel][row * cols + col]
    grid: Vec<Vec<f32>>,
    /// Pixel X coordinates of grid columns
    nodes_x: Vec<usize>,
    /// Pixel Y coordinates of grid rows
    nodes_y: Vec<usize>,
}

/// Background extractor for light pollution removal
pub struct BackgroundExtractor {
    pub(crate) config: BackgroundConfig,
}

impl BackgroundExtractor {
    /// Create a new background extractor with the given configuration
    pub fn new(config: BackgroundConfig) -> Self {
        Self { config }
    }

    /// Create a new background extractor with default configuration
    pub fn with_defaults() -> Self {
        Self::new(BackgroundConfig::default())
    }

    /// Estimate the background model from the frame
    ///
    /// Returns a `BackgroundModel` that can be used to subtract the background
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        channels = frame.channels(),
        algorithm = %self.config.algorithm,
        grid = %format!("{}x{}", self.config.grid_width, self.config.grid_height),
        gradient_only = self.config.gradient_only
    ))]
    pub fn estimate(&self, frame: &Frame) -> Result<BackgroundModel> {
        match self.config.algorithm {
            BackgroundExtractionAlgorithm::GridBilinear => self.estimate_bilinear(frame),
            BackgroundExtractionAlgorithm::Rbf => {
                if let Some(plugin) = super::BACKGROUND_PLUGIN.get() {
                    plugin.estimate_rbf(frame, &self.config)
                } else {
                    Err(StackError::InvalidConfiguration(
                        "RBF background extraction is only available in the Pro version. \
                         Please upgrade to Pro or switch to Grid/Bilinear mode in settings."
                            .into(),
                    ))
                }
            }
        }
    }

    /// Bilinear background estimation using a boundary-hugging sample grid.
    ///
    /// Pipeline:
    /// 1. Overlay a grid with small sample boxes (1.5% of image width)
    /// 2. Extract star-rejected medians per box via iterative sigma clipping
    /// 3. Prune nodes on nebulosity via global + local neighbor comparison
    /// 4. Inpaint rejected nodes via iterative 4-connected averaging
    /// 5. Return a `BackgroundModel` with node coordinates for fast delta-stepping subtraction
    fn estimate_bilinear(&self, frame: &Frame) -> Result<BackgroundModel> {
        let width = frame.width();
        let height = frame.height();
        let channels = frame.channels();
        let grid_cols = self.config.grid_width;
        let grid_rows = self.config.grid_height;

        if grid_cols < 2 || grid_rows < 2 {
            return Err(StackError::InvalidConfiguration(
                "Grid dimensions must be >= 2 for bilinear interpolation".into(),
            ));
        }

        if width < grid_cols || height < grid_rows {
            return Err(StackError::InvalidConfiguration(
                "Image too small for the configured grid size".into(),
            ));
        }

        let box_size = compute_box_size(width);
        let (grid_template, nodes_x, nodes_y) = initialize_grid(width, height, grid_cols, grid_rows);

        // Extract node values per channel (parallelized)
        let per_channel_grids: Vec<Vec<GridNode>> = {
            let _span = tracing::info_span!("bilinear_node_extraction", box_size = box_size).entered();
            (0..channels)
                .into_par_iter()
                .map(|channel| {
                    let mut grid = grid_template.clone();
                    grid.par_iter_mut().for_each(|node| {
                        node.value = extract_node_value(frame, node, box_size, channel);
                    });
                    grid
                })
                .collect()
        };

        // Build the completed model (pruning + inpainting)
        let _span = tracing::info_span!("bilinear_build_model").entered();
        let model = build_bilinear_model(
            per_channel_grids,
            nodes_x,
            nodes_y,
            grid_cols,
            grid_rows,
            channels,
        );

        Ok(BackgroundModel::with_node_coords(
            model.grid,
            self.config.grid_width,
            self.config.grid_height,
            width,
            height,
            channels,
            self.config.gradient_only,
            self.config.reference_percentile,
            self.config.aggressiveness,
            model.nodes_x,
            model.nodes_y,
        ))
    }

    /// Compute the median of a block, rejecting bright stars.
    /// Exposed as pub(crate) so the RBF module can reuse this for sample block medians.
    pub(crate) fn compute_block_median(
        &self,
        frame: &Frame,
        x_start: usize,
        y_start: usize,
        x_end: usize,
        y_end: usize,
        channel: usize,
        buffer: &mut Vec<f32>,
        mad_buffer: &mut Vec<f32>,
    ) -> f32 {
        buffer.clear();

        let channels_count = frame.channels();
        let width = frame.width();
        let data = frame.data();

        for y in y_start..y_end {
            let row_offset = y * width * channels_count;
            buffer.extend((x_start..x_end).map(|x| data[row_offset + x * channels_count + channel]));
        }

        if buffer.is_empty() {
            return 0.0;
        }

        // First pass: compute initial median and MAD
        let initial_median = Self::median(buffer);
        let mad = Self::median_absolute_deviation(buffer, initial_median, mad_buffer);

        // Second pass: reject pixels above threshold (likely stars)
        let threshold = initial_median + self.config.star_rejection_sigma * mad * 1.4826; // 1.4826 scales MAD to std dev

        buffer.retain(|&v| v <= threshold);

        if buffer.is_empty() {
            initial_median
        } else {
            Self::median(buffer)
        }
    }

    /// Compute median of a slice using O(N) selection instead of O(N log N) sort
    pub(crate) fn median(values: &mut [f32]) -> f32 {
        if values.is_empty() {
            return 0.0;
        }
        let mid = values.len() / 2;
        let cmp = |a: &f32, b: &f32| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
        values.select_nth_unstable_by(mid, cmp);
        let median_val = values[mid];
        if values.len() % 2 == 0 {
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

    /// Compute Median Absolute Deviation
    pub(crate) fn median_absolute_deviation(values: &[f32], median: f32, deviations: &mut Vec<f32>) -> f32 {
        if values.is_empty() {
            return 0.0;
        }
        deviations.clear();
        deviations.extend(values.iter().map(|&v| (v - median).abs()));
        Self::median(deviations)
    }

    /// Estimate and subtract background in one step
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        channels = frame.channels(),
        algorithm = %self.config.algorithm
    ))]
    pub fn subtract(&self, frame: &mut Frame) -> Result<()> {
        let model = self.estimate(frame)?;
        model.subtract_from(frame);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Private pipeline functions for bilinear estimation
// ---------------------------------------------------------------------------

/// Compute the box size for sampling, ensuring it is odd and at least `MIN_BOX_SIZE`.
fn compute_box_size(image_width: usize) -> usize {
    let raw = (image_width as f32 * BOX_SIZE_PERCENTAGE) as usize;
    let clamped = raw.max(MIN_BOX_SIZE);
    if clamped % 2 == 0 { clamped + 1 } else { clamped }
}

/// Initialize a boundary-hugging grid.
///
/// Generates coordinates linearly spaced from 0 to dimension-1 so the outermost
/// nodes lie exactly on the image boundaries, enabling branchless delta-stepping.
fn initialize_grid(
    width: usize,
    height: usize,
    grid_cols: usize,
    grid_rows: usize,
) -> (Vec<GridNode>, Vec<usize>, Vec<usize>) {
    let nodes_x: Vec<usize> = (0..grid_cols)
        .map(|i| {
            if grid_cols == 1 { width / 2 }
            else { i * (width - 1) / (grid_cols - 1) }
        })
        .collect();

    let nodes_y: Vec<usize> = (0..grid_rows)
        .map(|j| {
            if grid_rows == 1 { height / 2 }
            else { j * (height - 1) / (grid_rows - 1) }
        })
        .collect();

    let mut nodes = Vec::with_capacity(grid_cols * grid_rows);
    for (row, &y) in nodes_y.iter().enumerate() {
        for (col, &x) in nodes_x.iter().enumerate() {
            nodes.push(GridNode { x, y, col, row, value: None });
        }
    }

    (nodes, nodes_x, nodes_y)
}

/// Extract the background value for a single node using iterative sigma clipping.
fn extract_node_value(
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

    let channels = frame.channels();
    let data = frame.data();

    let capacity = (x_end - x_start) * (y_end - y_start);
    let mut pixels = Vec::with_capacity(capacity);

    for y in y_start..y_end {
        let row_offset = y * width * channels;
        for x in x_start..x_end {
            pixels.push(data[row_offset + x * channels + channel]);
        }
    }

    if pixels.is_empty() {
        return None;
    }

    let mut mad_buf = Vec::with_capacity(pixels.len());

    for _ in 0..SIGMA_CLIP_ITERATIONS {
        let median = BackgroundExtractor::median(&mut pixels);
        let mad = BackgroundExtractor::median_absolute_deviation(&pixels, median, &mut mad_buf);

        if mad < 1e-9 {
            break;
        }

        let threshold = median + SIGMA_CLIP_THRESHOLD * mad * 1.4826;
        let before = pixels.len();
        pixels.retain(|&v| v <= threshold);

        if pixels.is_empty() {
            return Some(median);
        }
        if pixels.len() == before {
            break;
        }
    }

    Some(BackgroundExtractor::median(&mut pixels))
}

/// O(N) median on a mutable slice (used by pipeline helpers).
fn fast_median(values: &mut [f32]) -> f32 {
    BackgroundExtractor::median(values)
}

/// Median Absolute Deviation (allocating variant for pruning).
fn fast_mad(values: &[f32], median: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut deviations: Vec<f32> = values.iter().map(|&v| (v - median).abs()).collect();
    fast_median(&mut deviations)
}

/// Prune nodes that landed on nebulosity using a two-stage approach:
///
/// 1. **Global rejection**: reject nodes above `global_median + sigma * MAD * 1.4826`.
/// 2. **Neighbor rejection**: reject nodes exceeding the local 8-neighbor median by 5%.
fn prune_nebulosity(nodes: &mut [GridNode], grid_cols: usize, grid_rows: usize) {
    // Stage 1: Global sigma-based rejection
    let mut all_values: Vec<f32> = nodes.iter().filter_map(|n| n.value).collect();
    if all_values.len() < 4 {
        return;
    }

    let global_median = fast_median(&mut all_values);
    let global_mad = fast_mad(&all_values, global_median);
    let global_threshold = global_median + GLOBAL_PRUNING_SIGMA * global_mad * 1.4826;

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
        let val = match node.value {
            Some(v) => v,
            None => continue,
        };

        let mut neighbor_values = Vec::with_capacity(8);
        for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let nr = node.row as i32 + dr;
                let nc = node.col as i32 + dc;
                if nr >= 0
                    && nr < grid_rows as i32
                    && nc >= 0
                    && nc < grid_cols as i32
                {
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

        let local_median = fast_median(&mut neighbor_values);
        if val > local_median * NEBULOSITY_THRESHOLD {
            node.value = None;
        }
    }
}

/// Iteratively fill `None` nodes using the average of valid 4-connected neighbors.
///
/// Sequential — the grid is small (e.g. 16×16 = 256 elements), so rayon overhead
/// would exceed the computation cost.
fn inpaint_grid(grid: &mut [Option<f32>], rows: usize, cols: usize) {
    loop {
        let mut any_filled = false;
        let mut temp: Vec<(usize, f32)> = Vec::new();

        for r in 0..rows {
            for c in 0..cols {
                let idx = r * cols + c;
                if grid[idx].is_some() {
                    continue;
                }

                let mut sum = 0.0f32;
                let mut count = 0u32;

                // Up
                if r > 0 {
                    if let Some(v) = grid[(r - 1) * cols + c] {
                        sum += v;
                        count += 1;
                    }
                }
                // Down
                if r + 1 < rows {
                    if let Some(v) = grid[(r + 1) * cols + c] {
                        sum += v;
                        count += 1;
                    }
                }
                // Left
                if c > 0 {
                    if let Some(v) = grid[r * cols + c - 1] {
                        sum += v;
                        count += 1;
                    }
                }
                // Right
                if c + 1 < cols {
                    if let Some(v) = grid[r * cols + c + 1] {
                        sum += v;
                        count += 1;
                    }
                }

                if count > 0 {
                    temp.push((idx, sum / count as f32));
                    any_filled = true;
                }
            }
        }

        for (idx, val) in &temp {
            grid[*idx] = Some(*val);
        }

        if !any_filled {
            break;
        }
    }
}

/// Build the completed bilinear model from per-channel grids.
///
/// Uses the green channel (highest SNR, lowest atmospheric scattering) for
/// nebulosity rejection and applies the same mask to all channels.
fn build_bilinear_model(
    mut per_channel_grids: Vec<Vec<GridNode>>,
    nodes_x: Vec<usize>,
    nodes_y: Vec<usize>,
    grid_cols: usize,
    grid_rows: usize,
    channels: usize,
) -> BilinearModel {
    let total_nodes = grid_cols * grid_rows;

    // Prune using green channel (or first channel for mono)
    let stats_channel = if channels > 1 { 1 } else { 0 };
    prune_nebulosity(&mut per_channel_grids[stats_channel], grid_cols, grid_rows);

    // Build rejection mask from the reference channel
    let rejection_mask: Vec<bool> = per_channel_grids[stats_channel]
        .iter()
        .map(|n| n.value.is_none())
        .collect();

    // Apply mask to all other channels
    for (ch, grid) in per_channel_grids.iter_mut().enumerate() {
        if ch == stats_channel {
            continue;
        }
        for (i, node) in grid.iter_mut().enumerate() {
            if rejection_mask[i] {
                node.value = None;
            }
        }
    }

    // Count surviving nodes
    let valid_count = per_channel_grids[stats_channel]
        .iter()
        .filter(|n| n.value.is_some())
        .count();

    debug!(
        valid_nodes = valid_count,
        total_nodes = total_nodes,
        "Bilinear grid after nebulosity pruning"
    );

    // Safety: flat-field fallback if too few nodes survive
    if valid_count < MIN_VALID_NODES {
        warn!(
            valid_count = valid_count,
            "Too few nodes survived pruning, falling back to flat-field subtraction"
        );

        // Compute global median from the stats channel's original values
        let mut all_values: Vec<f32> = per_channel_grids[stats_channel]
            .iter()
            .filter_map(|n| n.value)
            .collect();

        // If even those are empty, collect from any channel
        if all_values.is_empty() {
            for grid in &per_channel_grids {
                all_values.extend(grid.iter().filter_map(|n| n.value));
            }
        }

        let fallback_value = if all_values.is_empty() {
            0.0
        } else {
            fast_median(&mut all_values)
        };

        let grid = vec![vec![fallback_value; total_nodes]; channels];
        return BilinearModel { grid, nodes_x, nodes_y };
    }

    // Inpaint per channel, then flatten to completed f32 grids
    let grid: Vec<Vec<f32>> = per_channel_grids
        .into_iter()
        .map(|channel_nodes| {
            let mut opt_grid: Vec<Option<f32>> = channel_nodes.iter().map(|n| n.value).collect();
            inpaint_grid(&mut opt_grid, grid_rows, grid_cols);
            opt_grid.into_iter().map(|v| v.unwrap_or(0.0)).collect()
        })
        .collect();

    BilinearModel { grid, nodes_x, nodes_y }
}
