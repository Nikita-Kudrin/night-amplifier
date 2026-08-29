use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;
use tracing::{debug, instrument, warn};

use super::config::{BackgroundConfig, BackgroundExtractionAlgorithm};
use super::grid::{
    compute_box_size, extract_node_value, mad, median, prune_nebulosity, GridNode, PruneConfig,
};
use super::model::BackgroundModel;

/// Nebulosity pruning thresholds for the bilinear grid.
///
/// Looser than the RBF extractor's, which prunes at 1.0 sigma and 2 %: a thin-plate
/// spline bends through a nebulosity node, while bilinear interpolation only smears it
/// into the cells that touch it.
const PRUNE: PruneConfig = PruneConfig {
    global_sigma: 2.5,
    neighbour_threshold: 1.05,
};

/// Minimum surviving nodes before falling back to flat-field subtraction
const MIN_VALID_NODES: usize = 4;

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
                if let Some(plugin) = crate::license::pro_plugin(&super::BACKGROUND_PLUGIN) {
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
        let (grid_template, nodes_x, nodes_y) =
            initialize_grid(width, height, grid_cols, grid_rows);

        // Extract node values per channel (parallelized)
        let per_channel_grids: Vec<Vec<GridNode>> = {
            let _span =
                tracing::info_span!("bilinear_node_extraction", box_size = box_size).entered();
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
            if grid_cols == 1 {
                width / 2
            } else {
                i * (width - 1) / (grid_cols - 1)
            }
        })
        .collect();

    let nodes_y: Vec<usize> = (0..grid_rows)
        .map(|j| {
            if grid_rows == 1 {
                height / 2
            } else {
                j * (height - 1) / (grid_rows - 1)
            }
        })
        .collect();

    let mut nodes = Vec::with_capacity(grid_cols * grid_rows);
    for (row, &y) in nodes_y.iter().enumerate() {
        for (col, &x) in nodes_x.iter().enumerate() {
            nodes.push(GridNode::new(x, y, col, row));
        }
    }

    (nodes, nodes_x, nodes_y)
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
    prune_nebulosity(
        &mut per_channel_grids[stats_channel],
        grid_cols,
        grid_rows,
        PRUNE,
    );

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
            median(&mut all_values)
        };

        let grid = vec![vec![fallback_value; total_nodes]; channels];
        return BilinearModel {
            grid,
            nodes_x,
            nodes_y,
        };
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

    BilinearModel {
        grid,
        nodes_x,
        nodes_y,
    }
}
