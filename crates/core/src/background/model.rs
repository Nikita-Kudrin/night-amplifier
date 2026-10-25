use crate::error::Result;
use crate::frame::Frame;
use crate::statistics::MAD_TO_SIGMA;
use super::edges::EdgeProfiles;
use super::pchip::{self, Segment};
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
    /// Share of the gradient subtracted (0.0 to 1.0)
    aggressiveness: f32,
    /// Pixel X coordinates of grid columns: interpolate between nodes, not cell centres
    nodes_x: Option<Vec<usize>>,
    /// Pixel Y coordinates of grid rows
    nodes_y: Option<Vec<usize>>,
    /// Structure along the frame's edges, added on top of the grid ([`EdgeProfiles`]).
    edges: Option<EdgeProfiles>,
}

impl BackgroundModel {
    // One argument per model field: a parameter struct would be `BackgroundModel` again.
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
            edges: None,
        }
    }

    /// Create a model with explicit node coordinates: `subtract_from()` then joins the nodes
    /// by monotone cubic interpolation ([`super::pchip`]) instead of bilinear weights
    /// between cell centres.
    // As `new`, plus the node coordinates.
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
            edges: None,
        }
    }

    /// This model with `edges` added on top of its grid.
    pub fn with_edges(mut self, edges: EdgeProfiles) -> Self {
        self.edges = Some(edges);
        self
    }

    pub fn edges(&self) -> Option<&EdgeProfiles> {
        self.edges.as_ref()
    }

    /// The background at one pixel as [`Self::subtract_from`] removes it (before the
    /// reference offset), edge profiles included. It once read a node model between cell
    /// centres instead, which no subtraction used. Prepares the rows each call: many pixels
    /// are [`Self::render_plane`]'s job.
    pub fn get_background(&self, x: usize, y: usize, channel: usize) -> f32 {
        self.sampler(channel, &[y]).at(x, 0)
    }

    /// Subtract this background model from a frame, clamping to [0.0, 1.0] afterward.
    ///
    /// If `gradient_only`, only the gradient (variation from a reference level) is
    /// subtracted, preserving the base signal — important for low-signal targets. The
    /// reference level is `reference_percentile` (default 10th); `aggressiveness` controls
    /// how much of the gradient to subtract.
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        channels = frame.channels(),
        gradient_only = self.gradient_only,
        aggressiveness = self.aggressiveness
    ))]
    pub fn subtract_from(&self, frame: &mut Frame) {
        // The rows recover the channel index from `self.image_height` while writing
        // `frame.data_mut()`, so a frame that does not match the model would silently write
        // one plane's correction into another: refuse the mismatch.
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
            let sigma = mad * MAD_TO_SIGMA;

            (3.0 * sigma).max(0.001)
        };

        let aggressiveness = self.aggressiveness.clamp(0.0, 1.0);

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

        let _span = tracing::info_span!("apply_subtraction").entered();
        let rows = Rows::new(self);
        let height = self.image_height;
        // One dispatch over every row of every plane: per channel instead cost three rayon
        // barriers and measured 19 % slower.
        frame.data_mut().par_chunks_mut(self.image_width).enumerate().for_each_init(
            || rows.scratch(),
            |scratch, (idx, row)| {
                let (c, y) = (idx / height, idx % height);
                rows.render(c, y, scratch);
                for (slot, background) in row.iter_mut().zip(&scratch.row) {
                    let subtraction = (background - offsets[c]) * aggressiveness;
                    *slot = (*slot - subtraction + pedestal).max(0.0);
                }
            },
        );
    }

    /// The background of `channel` as subtracted (before the reference offset), into `out`.
    pub fn render_plane(&self, channel: usize, out: &mut [f32]) {
        let rows = Rows::new(self);
        out.par_chunks_mut(self.image_width).enumerate().for_each_init(
            || rows.scratch(),
            |scratch, (y, row)| {
                rows.render(channel, y, scratch);
                row.copy_from_slice(&scratch.row);
            },
        );
    }

    /// Channel `c`'s background (as [`Self::render_plane`] has it) at single pixels of rows
    /// `ys`: each row's curve is prepared once and read at any column. The edge profiles
    /// read ~300k pixels a channel; rendering their rows whole into a fresh buffer had 20
    /// threads page-faulting on one lock for a quarter of the estimate.
    pub(super) fn sampler(&self, c: usize, ys: &[usize]) -> Sampler<'_> {
        let rows = Rows::new(self);
        let mut scratch = rows.scratch();
        let segments = match &rows.kind {
            RowKind::Nodes { nodes_x, .. } => {
                let mut of = vec![0u16; self.image_width];
                for (i, pair) in nodes_x.windows(2).enumerate() {
                    of[pair[0]..=pair[1]].fill(i as u16);
                }
                let per_row = ys.iter().flat_map(|&y| {
                    rows.segments(c, y, &mut scratch);
                    scratch.segments.clone()
                });
                Some((of, per_row.collect()))
            }
            RowKind::Weights { .. } => None,
        };
        Sampler { rows, c, ys: ys.to_vec(), segments }
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

    /// The background as subtracted, as a new Frame (for visualization).
    pub fn to_frame(&self) -> Result<Frame> {
        let mut frame = Frame::zeros(self.image_width, self.image_height, self.channels)?;
        let channels = self.channels;

        for c in 0..channels {
            self.render_plane(c, frame.channel_data_mut(c));
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

/// Per-call state that writes one row of a model's background: node interpolation
/// ([`pchip`]) when the model carries node coordinates, bilinear weights between cell
/// centres otherwise (RBF's dense evaluated grid), then the edge profiles on top.
struct Rows<'a> {
    model: &'a BackgroundModel,
    kind: RowKind<'a>,
}

enum RowKind<'a> {
    Nodes {
        xs: Vec<f32>,
        ys: Vec<f32>,
        nodes_x: &'a [usize],
        /// Every node's slope down its column, per channel, laid out like the grid.
        column_slopes: Vec<Vec<f32>>,
        /// Band of every row: half-open `[ys[j], ys[j + 1])`, the last one closed.
        row_band: Vec<usize>,
    },
    Weights {
        gx: Vec<(usize, usize, f32)>,
        gy: Vec<(usize, usize, f32)>,
    },
}

struct Scratch {
    values: Vec<f32>,
    slopes: Vec<f32>,
    segments: Vec<Segment>,
    row: Vec<f32>,
}

impl<'a> Rows<'a> {
    fn new(model: &'a BackgroundModel) -> Self {
        let kind = match (&model.nodes_x, &model.nodes_y) {
            (Some(nodes_x), Some(nodes_y)) => Self::nodes(model, nodes_x, nodes_y),
            _ => Self::weights(model),
        };
        Self { model, kind }
    }

    fn nodes(model: &BackgroundModel, nodes_x: &'a [usize], nodes_y: &[usize]) -> RowKind<'a> {
        let (cols, rows) = (model.grid_width, model.grid_height);
        let ys: Vec<f32> = nodes_y.iter().map(|&y| y as f32).collect();
        let column_slopes = model
            .grid_values
            .iter()
            .map(|grid| {
                let mut out = vec![0.0; rows * cols];
                let (mut column, mut d) = (vec![0.0; rows], vec![0.0; rows]);
                for i in 0..cols {
                    column.iter_mut().enumerate().for_each(|(j, v)| *v = grid[j * cols + i]);
                    pchip::slopes(&ys, &column, &mut d);
                    d.iter().enumerate().for_each(|(j, &s)| out[j * cols + i] = s);
                }
                out
            })
            .collect();
        let mut row_band = Vec::with_capacity(model.image_height);
        let mut j = 0usize;
        for y in 0..model.image_height {
            while j + 2 < rows && y >= nodes_y[j + 1] {
                j += 1;
            }
            row_band.push(j);
        }
        RowKind::Nodes { xs: nodes_x.iter().map(|&x| x as f32).collect(), ys, nodes_x, column_slopes, row_band }
    }

    fn weights(model: &BackgroundModel) -> RowKind<'a> {
        // Clamped against each axis's own count: RBF's grid is taller than wide on a
        // portrait frame, and a row index clamped to the column count collapsed its rows.
        let axis = |pixels: usize, cells: usize| -> Vec<(usize, usize, f32)> {
            (0..pixels)
                .map(|p| {
                    let g = (p as f32 + 0.5) * cells as f32 / pixels as f32 - 0.5;
                    let g0 = (g.floor() as isize).clamp(0, cells as isize - 1) as usize;
                    (g0, (g0 + 1).min(cells - 1), (g - g0 as f32).clamp(0.0, 1.0))
                })
                .collect()
        };
        RowKind::Weights {
            gx: axis(model.image_width, model.grid_width),
            gy: axis(model.image_height, model.grid_height),
        }
    }

    fn scratch(&self) -> Scratch {
        let cols = self.model.grid_width;
        Scratch {
            values: vec![0.0; cols],
            slopes: vec![0.0; cols],
            segments: Vec::with_capacity(cols),
            row: vec![0.0; self.model.image_width],
        }
    }

    /// Row `y` of channel `c` across the node columns, into `scratch.segments`: a node
    /// model's cubic between each pair of node columns, without the edge profiles.
    fn segments(&self, c: usize, y: usize, scratch: &mut Scratch) {
        let RowKind::Nodes { xs, ys, column_slopes, row_band, .. } = &self.kind else {
            unreachable!("only a node model has segments");
        };
        let (grid, cols) = (&self.model.grid_values[c], self.model.grid_width);
        let (d, j) = (&column_slopes[c], row_band[y]);
        let (h, s) = (ys[j + 1] - ys[j], y as f32 - ys[j]);
        for (i, value) in scratch.values.iter_mut().enumerate() {
            let (upper, lower) = (j * cols + i, (j + 1) * cols + i);
            *value = Segment::new(h, (grid[upper], d[upper]), (grid[lower], d[lower])).at(s);
        }
        pchip::slopes(xs, &scratch.values, &mut scratch.slopes);
        let (values, slopes) = (&scratch.values, &scratch.slopes);
        scratch.segments.clear();
        scratch.segments.extend((0..cols - 1).map(|i| {
            Segment::new(xs[i + 1] - xs[i], (values[i], slopes[i]), (values[i + 1], slopes[i + 1]))
        }));
    }

    /// Row `y` of channel `c` into `scratch.row`.
    fn render(&self, c: usize, y: usize, scratch: &mut Scratch) {
        let grid = &self.model.grid_values[c];
        let cols = self.model.grid_width;
        let edges = self.model.edges.as_ref();
        // The top and bottom profiles are one value per row: added in the same pass.
        let vertical = edges.map_or(0.0, |e| e.vertical(c, y, self.model.image_height));
        match &self.kind {
            RowKind::Nodes { nodes_x, .. } => {
                self.segments(c, y, scratch);
                for (i, segment) in scratch.segments.iter().enumerate() {
                    // Half-open: the last segment also takes the last pixel.
                    let end = if i == cols - 2 { nodes_x[i + 1] + 1 } else { nodes_x[i + 1] };
                    let segment = segment.raised(vertical);
                    // An `i32` offset: converting a `usize` to `f32` does not vectorise, and
                    // was most of this loop (`cvtsi2ss` per lane).
                    for (k, out) in (0i32..).zip(&mut scratch.row[nodes_x[i]..end]) {
                        *out = segment.at(k as f32);
                    }
                }
            }
            RowKind::Weights { gx, gy } => {
                let (gy0, gy1, fy) = gy[y];
                let (row0, row1) = (gy0 * cols, gy1 * cols);
                for (out, &(gx0, gx1, fx)) in scratch.row.iter_mut().zip(gx) {
                    let v0 = grid[row0 + gx0] * (1.0 - fx) + grid[row0 + gx1] * fx;
                    let v1 = grid[row1 + gx0] * (1.0 - fx) + grid[row1 + gx1] * fx;
                    *out = v0 * (1.0 - fy) + v1 * fy + vertical;
                }
            }
        }
        if let Some(edges) = edges {
            edges.add_sides(c, &mut scratch.row);
        }
    }
}

/// [`BackgroundModel::sampler`]: one channel's background at single pixels of a few rows.
pub(super) struct Sampler<'a> {
    rows: Rows<'a>,
    c: usize,
    ys: Vec<usize>,
    /// A node model's segment for every column, and every row's segments one after another.
    segments: Option<(Vec<u16>, Vec<Segment>)>,
}

impl Sampler<'_> {
    /// The background at column `x` of the `slot`th row the sampler was made for.
    pub(super) fn at(&self, x: usize, slot: usize) -> f32 {
        let model = self.rows.model;
        let y = self.ys[slot];
        let grid = match (&self.segments, &self.rows.kind) {
            (Some((of, segments)), RowKind::Nodes { nodes_x, .. }) => {
                let i = of[x] as usize;
                segments[slot * (model.grid_width - 1) + i].at((x - nodes_x[i]) as f32)
            }
            (_, RowKind::Weights { gx, gy }) => {
                let (grid, cols) = (&model.grid_values[self.c], model.grid_width);
                let ((gx0, gx1, fx), (gy0, gy1, fy)) = (gx[x], gy[y]);
                let v0 = grid[gy0 * cols + gx0] * (1.0 - fx) + grid[gy0 * cols + gx1] * fx;
                let v1 = grid[gy1 * cols + gx0] * (1.0 - fx) + grid[gy1 * cols + gx1] * fx;
                v0 * (1.0 - fy) + v1 * fy
            }
            (None, RowKind::Nodes { .. }) => unreachable!("a node model's sampler holds its segments"),
        };
        let edges = model.edges.as_ref();
        grid + edges.map_or(0.0, |e| e.vertical(self.c, y, model.image_height) + e.side(self.c, x, model.image_width))
    }
}
