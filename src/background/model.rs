use crate::error::Result;
use crate::frame::Frame;
use rayon::prelude::*;
use tracing::instrument;

/// A 2D background model for an image
#[derive(Debug, Clone)]
pub struct BackgroundModel {
    /// Grid median values per channel [channel][grid_y * grid_width + grid_x]
    grid_values: Vec<Vec<f32>>,
    /// Number of grid cells horizontally
    grid_width: usize,
    /// Number of grid cells vertically
    grid_height: usize,
    /// Original image width
    image_width: usize,
    /// Original image height
    image_height: usize,
    /// Number of channels
    channels: usize,
    /// If true, subtract only the gradient (variation from reference level)
    gradient_only: bool,
    /// Percentile to use as reference level (0.0 to 1.0)
    reference_percentile: f32,
    /// Aggressiveness of subtraction (0.0 to 1.0, or -1.0 for auto)
    aggressiveness: f32,
    /// Pixel X coordinates of grid columns (enables fast delta-stepping subtraction)
    nodes_x: Option<Vec<usize>>,
    /// Pixel Y coordinates of grid rows (enables fast delta-stepping subtraction)
    nodes_y: Option<Vec<usize>>,
}

impl BackgroundModel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        grid_values: Vec<Vec<f32>>,
        grid_width: usize,
        grid_height: usize,
        image_width: usize,
        image_height: usize,
        channels: usize,
        gradient_only: bool,
        reference_percentile: f32,
        aggressiveness: f32,
    ) -> Self {
        Self {
            grid_values,
            grid_width,
            grid_height,
            image_width,
            image_height,
            channels,
            gradient_only,
            reference_percentile,
            aggressiveness,
            nodes_x: None,
            nodes_y: None,
        }
    }

    /// Create a model with explicit node coordinates for fast delta-stepping subtraction.
    ///
    /// When `nodes_x` and `nodes_y` are present, `subtract_from()` uses a scanline
    /// delta-stepping inner loop instead of per-pixel weight lookups.
    #[allow(clippy::too_many_arguments)]
    pub fn with_node_coords(
        grid_values: Vec<Vec<f32>>,
        grid_width: usize,
        grid_height: usize,
        image_width: usize,
        image_height: usize,
        channels: usize,
        gradient_only: bool,
        reference_percentile: f32,
        aggressiveness: f32,
        nodes_x: Vec<usize>,
        nodes_y: Vec<usize>,
    ) -> Self {
        Self {
            grid_values,
            grid_width,
            grid_height,
            image_width,
            image_height,
            channels,
            gradient_only,
            reference_percentile,
            aggressiveness,
            nodes_x: Some(nodes_x),
            nodes_y: Some(nodes_y),
        }
    }

    /// Get the interpolated background value at a pixel position
    pub fn get_background(&self, x: usize, y: usize, channel: usize) -> f32 {
        // Map pixel coordinates to grid coordinates (as floats for interpolation)
        let gx = (x as f32 + 0.5) * self.grid_width as f32 / self.image_width as f32 - 0.5;
        let gy = (y as f32 + 0.5) * self.grid_height as f32 / self.image_height as f32 - 0.5;

        // Bilinear interpolation
        let gx0 = (gx.floor() as isize).clamp(0, self.grid_width as isize - 1) as usize;
        let gy0 = (gy.floor() as isize).clamp(0, self.grid_height as isize - 1) as usize;
        let gx1 = (gx0 + 1).min(self.grid_width - 1);
        let gy1 = (gy0 + 1).min(self.grid_height - 1);

        let fx = (gx - gx0 as f32).clamp(0.0, 1.0);
        let fy = (gy - gy0 as f32).clamp(0.0, 1.0);

        let grid = &self.grid_values[channel];

        let v00 = grid[gy0 * self.grid_width + gx0];
        let v10 = grid[gy0 * self.grid_width + gx1];
        let v01 = grid[gy1 * self.grid_width + gx0];
        let v11 = grid[gy1 * self.grid_width + gx1];

        // Bilinear interpolation formula
        let v0 = v00 * (1.0 - fx) + v10 * fx;
        let v1 = v01 * (1.0 - fx) + v11 * fx;

        v0 * (1.0 - fy) + v1 * fy
    }

    /// Subtract this background model from a frame
    ///
    /// Values are clamped to [0.0, 1.0] after subtraction.
    ///
    /// If `gradient_only` is true, only the gradient (variation from a reference level)
    /// is subtracted. This preserves the base signal level while removing gradients caused
    /// by light pollution. This is important for low-signal astronomical images.
    ///
    /// The reference level is determined by `reference_percentile` (default 10th percentile).
    /// The `aggressiveness` parameter controls how much of the gradient to subtract.
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        channels = frame.channels(),
        gradient_only = self.gradient_only,
        aggressiveness = self.aggressiveness
    ))]
    pub fn subtract_from(&self, frame: &mut Frame) {
        // Determine actual aggressiveness (auto-detect if -1.0)
        let aggressiveness = if self.aggressiveness < 0.0 {
            let _span = tracing::info_span!("compute_auto_aggressiveness").entered();
            self.compute_auto_aggressiveness()
        } else {
            self.aggressiveness
        };

        // In gradient-only mode, find the reference level per channel using percentile
        // and subtract only the difference from that reference, scaled by aggressiveness
        let offsets: Vec<f32> = if self.gradient_only {
            let _span = tracing::info_span!("compute_reference_levels").entered();
            (0..self.channels)
                .map(|c| self.compute_reference_level(c))
                .collect()
        } else {
            vec![0.0; self.channels]
        };

        // Dispatch to the optimal subtraction path
        if let (Some(nodes_x), Some(nodes_y)) = (&self.nodes_x, &self.nodes_y) {
            let _span = tracing::info_span!("delta_stepping_subtraction").entered();
            self.subtract_delta_stepping(frame, &offsets, aggressiveness, nodes_x, nodes_y);
        } else {
            let _span = tracing::info_span!("weight_based_subtraction").entered();
            self.subtract_weight_based(frame, &offsets, aggressiveness);
        }
    }

    /// Fast delta-stepping subtraction using boundary-hugging node coordinates.
    ///
    /// Iterates over grid bands, computing left/right edge values per scanline
    /// and advancing via a constant delta — no per-pixel divisions or weight lookups.
    fn subtract_delta_stepping(
        &self,
        frame: &mut Frame,
        offsets: &[f32],
        aggressiveness: f32,
        nodes_x: &[usize],
        nodes_y: &[usize],
    ) {
        let width = frame.width();
        let channels = frame.channels();
        let grid_cols = self.grid_width;
        let grid_rows = self.grid_height;
        let data = frame.data_mut();

        // Process row bands in parallel
        let band_indices: Vec<usize> = (0..grid_rows - 1).collect();
        band_indices.into_par_iter().for_each(|j| {
            let y_start = nodes_y[j];
            // Half-open: include last pixel only in the final band
            let y_end_loop = if j == grid_rows - 2 {
                nodes_y[j + 1] + 1
            } else {
                nodes_y[j + 1]
            };
            let dy = (nodes_y[j + 1] - y_start) as f32;
            let inv_dy = 1.0 / dy;

            for y in y_start..y_end_loop {
                let ty = (y - y_start) as f32 * inv_dy;

                for i in 0..grid_cols - 1 {
                    let x_start = nodes_x[i];
                    let x_end_loop = if i == grid_cols - 2 {
                        nodes_x[i + 1] + 1
                    } else {
                        nodes_x[i + 1]
                    };
                    let dx = (nodes_x[i + 1] - x_start) as f32;
                    let inv_dx = 1.0 / dx;

                    for c in 0..channels {
                        let grid = &self.grid_values[c];
                        let offset = offsets[c];

                        let v_tl = grid[j * grid_cols + i];
                        let v_bl = grid[(j + 1) * grid_cols + i];
                        let v_tr = grid[j * grid_cols + i + 1];
                        let v_br = grid[(j + 1) * grid_cols + i + 1];

                        let v_left = v_tl + (v_bl - v_tl) * ty;
                        let v_right = v_tr + (v_br - v_tr) * ty;
                        let delta_x = (v_right - v_left) * inv_dx;

                        let mut current_bg = v_left;
                        for x in x_start..x_end_loop {
                            let pixel_idx = y * width * channels + x * channels + c;
                            let gradient = current_bg - offset;
                            let subtraction = gradient * aggressiveness;
                            // SAFETY: parallel bands don't overlap, so no data race.
                            // We use raw pointer access to work around the borrow checker
                            // since data is borrowed mutably once and partitioned by y.
                            unsafe {
                                let ptr = (data as *const [f32] as *mut f32).add(pixel_idx);
                                *ptr = (*ptr - subtraction).max(0.0);
                            }
                            current_bg += delta_x;
                        }
                    }
                }
            }
        });
    }

    /// Weight-based subtraction (existing approach, used by RBF path)
    fn subtract_weight_based(
        &self,
        frame: &mut Frame,
        offsets: &[f32],
        aggressiveness: f32,
    ) {
        let width = frame.width();
        let channels = frame.channels();

        let (gx_weights, gy_weights) = {
            let _span = tracing::info_span!("prepare_interpolation_weights").entered();
            let mut gx_weights = Vec::with_capacity(width);
            for x in 0..width {
                let gx = (x as f32 + 0.5) * self.grid_width as f32 / self.image_width as f32 - 0.5;
                let gx0 = (gx.floor() as isize).clamp(0, self.grid_width as isize - 1) as usize;
                let gx1 = (gx0 + 1).min(self.grid_width - 1);
                let fx = (gx - gx0 as f32).clamp(0.0, 1.0);
                gx_weights.push((gx0, gx1, fx));
            }

            let mut gy_weights = Vec::with_capacity(self.image_height);
            for y in 0..self.image_height {
                let gy = (y as f32 + 0.5) * self.grid_height as f32 / self.image_height as f32 - 0.5;
                let gy0 = (gy.floor() as isize).clamp(0, self.grid_width as isize - 1) as usize;
                let gy1 = (gy0 + 1).min(self.grid_height - 1);
                let fy = (gy - gy0 as f32).clamp(0.0, 1.0);
                gy_weights.push((gy0, gy1, fy));
            }
            (gx_weights, gy_weights)
        };

        let data = frame.data_mut();

        {
            let _span = tracing::info_span!("apply_subtraction").entered();
            data.par_chunks_mut(width * channels)
                .enumerate()
                .for_each(|(y, row)| {
                    let (gy0, gy1, fy) = gy_weights[y];

                    for x in 0..width {
                        let (gx0, gx1, fx) = gx_weights[x];

                        for c in 0..channels {
                            let idx = x * channels + c;

                            let grid = &self.grid_values[c];
                            let v00 = grid[gy0 * self.grid_width + gx0];
                            let v10 = grid[gy0 * self.grid_width + gx1];
                            let v01 = grid[gy1 * self.grid_width + gx0];
                            let v11 = grid[gy1 * self.grid_width + gx1];

                            let v0 = v00 * (1.0 - fx) + v10 * fx;
                            let v1 = v01 * (1.0 - fx) + v11 * fx;
                            let bg = v0 * (1.0 - fy) + v1 * fy;

                            let gradient = bg - offsets[c];
                            let subtraction = gradient * aggressiveness;
                            row[idx] = (row[idx] - subtraction).max(0.0);
                        }
                    }
                });
        }
    }

    /// Compute reference level for a channel using the configured percentile
    fn compute_reference_level(&self, channel: usize) -> f32 {
        let mut sorted: Vec<f32> = self.grid_values[channel].clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        if sorted.is_empty() {
            return 0.0;
        }

        // Compute the percentile index
        let idx = ((sorted.len() as f32 - 1.0) * self.reference_percentile) as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    /// Automatically compute aggressiveness based on background uniformity.
    /// High variation suggests extended objects; use lower aggressiveness.
    /// Low variation suggests pure gradients; use higher aggressiveness.
    fn compute_auto_aggressiveness(&self) -> f32 {
        // Use the green channel (or first channel) for analysis
        let channel = if self.channels > 1 { 1 } else { 0 };
        let grid = &self.grid_values[channel];

        if grid.is_empty() {
            return 0.5;
        }

        // Compute coefficient of variation (CV = std_dev / mean)
        let mean: f32 = grid.iter().sum::<f32>() / grid.len() as f32;
        if mean < 1e-9 {
            return 0.5;
        }

        let variance: f32 =
            grid.iter().map(|&v| (v - mean).powi(2)).sum::<f32>() / grid.len() as f32;
        let std_dev = variance.sqrt();
        let cv = std_dev / mean;

        // Map CV to aggressiveness (conservative approach to preserve nebulae):
        // CV < 0.03: Very uniform -> aggressiveness = 0.7 (mostly gradient, subtract most)
        // CV > 0.15: Highly non-uniform (likely nebulae) -> aggressiveness = 0.15
        // Linear interpolation in between
        // This is more conservative than before to better preserve extended objects
        let aggressiveness = if cv < 0.03 {
            0.7
        } else if cv > 0.15 {
            0.15
        } else {
            // Linear interpolation: 0.7 at cv=0.03, 0.15 at cv=0.15
            0.7 - (cv - 0.03) / (0.15 - 0.03) * 0.55
        };

        aggressiveness
    }

    /// Generate the background as a new Frame (useful for visualization)
    pub fn to_frame(&self) -> Result<Frame> {
        let mut frame = Frame::zeros(self.image_width, self.image_height, self.channels)?;
        let width = self.image_width;
        let channels = self.channels;

        let data = frame.data_mut();

        data.par_chunks_mut(width * channels)
            .enumerate()
            .for_each(|(y, row)| {
                for x in 0..width {
                    for c in 0..channels {
                        let idx = x * channels + c;
                        row[idx] = self.get_background(x, y, c);
                    }
                }
            });

        Ok(frame)
    }

    /// Get the grid dimensions
    pub fn grid_dimensions(&self) -> (usize, usize) {
        (self.grid_width, self.grid_height)
    }

    /// Get the raw grid values for a channel
    pub fn grid_values(&self, channel: usize) -> &[f32] {
        &self.grid_values[channel]
    }
}
