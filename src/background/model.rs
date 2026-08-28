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
        // `subtract_weight_based` recovers the channel index from `self.image_height`
        // while writing `frame.data_mut()`, so a frame that does not match the model
        // would silently write one plane's correction into another. `subtract_delta_stepping`
        // reads the same geometry off the frame instead. Rather than leave the two paths
        // disagreeing about which is authoritative, refuse the mismatch here.
        if frame.width() != self.image_width
            || frame.height() != self.image_height
            || frame.channels() != self.channels
        {
            tracing::warn!(
                frame = %format!("{}x{}x{}", frame.width(), frame.height(), frame.channels()),
                model = %format!("{}x{}x{}", self.image_width, self.image_height, self.channels),
                "Background model does not match the frame; skipping subtraction"
            );
            return;
        }

        // Calculate dynamic pedestal
        let pedestal = {
            let _span = tracing::info_span!("compute_pedestal").entered();
            let w = frame.width();
            let h = frame.height();
            let c = frame.channels();

            let crop_w = (w / 4).max(10).min(w);
            let crop_h = (h / 4).max(10).min(h);
            let x_start = (w - crop_w) / 2;
            let y_start = (h - crop_h) / 2;

            let sample_c = if c > 1 { 1 } else { 0 };
            let num_pixels = crop_w * crop_h;
            let max_samples = 4096;
            let step = (num_pixels / max_samples).max(1);

            let mut sample = Vec::with_capacity(max_samples.min(num_pixels));
            // One plane, indexed `y * w + x`. The interleaved form (`y * w * c` plus
            // `x * c + sample_c`) survived the planar migration and stayed silent because
            // for `c == 3` the arithmetic happens to land inside the green plane — it just
            // sampled the wrong rows, sweeping most of the frame height at a 3-pixel
            // horizontal stride instead of the centre crop.
            let plane = frame.channel_data(sample_c);
            let mut count = 0;

            for y in y_start..(y_start + crop_h) {
                let row_offset = y * w;
                for x in x_start..(x_start + crop_w) {
                    if count % step == 0 {
                        sample.push(plane[row_offset + x]);
                    }
                    count += 1;
                }
            }

            let median = crate::statistics::fast_median(&mut sample);
            let mut deviations = Vec::with_capacity(sample.len());
            for &v in &sample {
                deviations.push((v - median).abs());
            }
            let mad = crate::statistics::fast_median(&mut deviations);
            let sigma = mad * 1.4826;

            (3.0 * sigma).max(0.001)
        };

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
            let mut raw_offsets: Vec<f32> = (0..self.channels)
                .map(|c| self.compute_reference_level(c))
                .collect();

            // If RGB, align all channels to the same minimum reference level
            // This preserves the color neutrality achieved by white balance.
            if self.channels == 3 {
                let min_offset = raw_offsets[0].min(raw_offsets[1]).min(raw_offsets[2]);
                raw_offsets = vec![min_offset; 3];
            }
            raw_offsets
        } else {
            vec![0.0; self.channels]
        };

        // Dispatch to the optimal subtraction path
        if let (Some(nodes_x), Some(nodes_y)) = (&self.nodes_x, &self.nodes_y) {
            let _span = tracing::info_span!("delta_stepping_subtraction").entered();
            self.subtract_delta_stepping(
                frame,
                &offsets,
                aggressiveness,
                pedestal,
                nodes_x,
                nodes_y,
            );
        } else {
            let _span = tracing::info_span!("weight_based_subtraction").entered();
            self.subtract_weight_based(frame, &offsets, aggressiveness, pedestal);
        }
    }

    /// Fast delta-stepping subtraction using boundary-hugging node coordinates.
    ///
    /// Iterates over grid bands, computing left/right edge values per scanline
    /// and advancing via a constant delta — no per-pixel divisions or weight lookups.
    ///
    /// # Why this is parallel per row rather than per band
    ///
    /// The predecessor split the work across `grid_rows - 1` bands — eleven tasks for
    /// the default 12x12 grid, on a machine with twenty cores — and reached the pixels
    /// through `(data as *const [f32] as *mut f32)`: a `*mut` derived from a shared
    /// reference, written from inside a `for_each`. The bands genuinely did not overlap,
    /// but that is not what makes it sound; writing through a pointer derived from a
    /// shared borrow is undefined behaviour under Stacked Borrows regardless.
    ///
    /// Planar layout makes the safe version the simpler one. Each plane is a contiguous
    /// run, so `par_chunks_mut(width)` hands out one owned row per task, and the band a
    /// row belongs to is a property of its `y` alone — precomputed once below rather
    /// than rediscovered per band per channel. No `unsafe`, and `height * channels`
    /// tasks instead of eleven.
    fn subtract_delta_stepping(
        &self,
        frame: &mut Frame,
        offsets: &[f32],
        aggressiveness: f32,
        pedestal: f32,
        nodes_x: &[usize],
        nodes_y: &[usize],
    ) {
        let width = frame.width();
        let height = frame.height();
        let grid_cols = self.grid_width;
        let grid_rows = self.grid_height;

        // (band index, interpolation fraction down that band) for every output row.
        //
        // Bands are half-open `[nodes_y[j], nodes_y[j + 1])`, except the last, which
        // includes its bottom node — same partition the band loop used. `initialize_grid`
        // hugs the boundaries (`nodes_y[0] == 0`, `nodes_y[last] == height - 1`), so
        // every row lands in exactly one band.
        let mut row_band: Vec<(usize, f32)> = Vec::with_capacity(height);
        let mut j = 0usize;
        for y in 0..height {
            while j + 2 < grid_rows && y >= nodes_y[j + 1] {
                j += 1;
            }
            let inv_dy = 1.0 / (nodes_y[j + 1] - nodes_y[j]) as f32;
            row_band.push((j, (y - nodes_y[j]) as f32 * inv_dy));
        }

        // One dispatch over every row of every plane, as `subtract_weight_based` does.
        // The shape matters: dispatching once per channel instead costs three rayon
        // barriers and measured 19 % slower than the band-parallel predecessor. This
        // form lands inside its noise band (12.2-12.6 ms against 12.6-13.9 ms over
        // repeated runs of `background_subtract/subtract_from_x5`), so the `unsafe` goes
        // away for free rather than being paid for.
        let grids = &self.grid_values;
        frame
            .data_mut()
            .par_chunks_mut(width)
            .enumerate()
            .for_each(|(idx, row)| {
                let c = idx / height;
                let y = idx % height;
                let grid = &grids[c];
                let offset = offsets[c];
                let (j, ty) = row_band[y];

                for i in 0..grid_cols - 1 {
                    let x_start = nodes_x[i];
                    // Half-open: include the last pixel only in the final band.
                    let x_end_loop = if i == grid_cols - 2 {
                        nodes_x[i + 1] + 1
                    } else {
                        nodes_x[i + 1]
                    };
                    let inv_dx = 1.0 / (nodes_x[i + 1] - x_start) as f32;

                    let v_tl = grid[j * grid_cols + i];
                    let v_bl = grid[(j + 1) * grid_cols + i];
                    let v_tr = grid[j * grid_cols + i + 1];
                    let v_br = grid[(j + 1) * grid_cols + i + 1];

                    let v_left = v_tl + (v_bl - v_tl) * ty;
                    let v_right = v_tr + (v_br - v_tr) * ty;
                    let delta_x = (v_right - v_left) * inv_dx;

                    let mut current_bg = v_left;
                    for slot in &mut row[x_start..x_end_loop] {
                        let subtraction = (current_bg - offset) * aggressiveness;
                        *slot = (*slot - subtraction + pedestal).max(0.0);
                        current_bg += delta_x;
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
        pedestal: f32,
    ) {
        let width = frame.width();

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
                let gy =
                    (y as f32 + 0.5) * self.grid_height as f32 / self.image_height as f32 - 0.5;
                // `grid_height`, not `grid_width`: clamping the row index against the
                // column count is harmless while the grid is square or wider than it is
                // tall (the extractor's 12x12/16x16 defaults, and RBF on a landscape
                // frame), but RBF derives `eval_height = 256 * h / w`, so a portrait
                // frame gets more grid rows than columns and every row past
                // `grid_width - 1` collapsed onto that one.
                let gy0 = (gy.floor() as isize).clamp(0, self.grid_height as isize - 1) as usize;
                let gy1 = (gy0 + 1).min(self.grid_height - 1);
                let fy = (gy - gy0 as f32).clamp(0.0, 1.0);
                gy_weights.push((gy0, gy1, fy));
            }
            (gx_weights, gy_weights)
        };

        {
            let _span = tracing::info_span!("apply_subtraction").entered();
            // One dispatch for the whole buffer instead of one per channel. Planes are
            // contiguous and every plane has `height` rows, so a flat walk over
            // `width`-sized chunks yields both the channel and the row from the chunk
            // index — no zip, no per-channel barrier.
            let height = self.image_height;
            let grid_width = self.grid_width;
            let grids = &self.grid_values;

            frame
                .data_mut()
                .par_chunks_mut(width)
                .enumerate()
                .for_each(|(i, row)| {
                    let c = i / height;
                    let y = i % height;
                    let grid = &grids[c];
                    let offset = offsets[c];

                    let (gy0, gy1, fy) = gy_weights[y];
                    // Row offsets into the grid are fixed for the whole row.
                    let row0 = gy0 * grid_width;
                    let row1 = gy1 * grid_width;

                    for x in 0..width {
                        let (gx0, gx1, fx) = gx_weights[x];

                        let v0 = grid[row0 + gx0] * (1.0 - fx) + grid[row0 + gx1] * fx;
                        let v1 = grid[row1 + gx0] * (1.0 - fx) + grid[row1 + gx1] * fx;
                        let bg = v0 * (1.0 - fy) + v1 * fy;

                        let subtraction = (bg - offset) * aggressiveness;
                        row[x] = (row[x] - subtraction + pedestal).max(0.0);
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

        if cv < 0.03 {
            0.7
        } else if cv > 0.15 {
            0.15
        } else {
            // Linear interpolation: 0.7 at cv=0.03, 0.15 at cv=0.15
            0.7 - (cv - 0.03) / (0.15 - 0.03) * 0.55
        }
    }

    /// Generate the background as a new Frame (useful for visualization)
    pub fn to_frame(&self) -> Result<Frame> {
        let mut frame = Frame::zeros(self.image_width, self.image_height, self.channels)?;
        let width = self.image_width;
        let channels = self.channels;

        for c in 0..channels {
            let plane = frame.channel_data_mut(c);
            plane
                .par_chunks_mut(width)
                .enumerate()
                .for_each(|(y, row)| {
                    for x in 0..width {
                        row[x] = self.get_background(x, y, c);
                    }
                });
        }

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
