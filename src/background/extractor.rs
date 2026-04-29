use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;
use tracing::instrument;

use super::config::{BackgroundConfig, BackgroundExtractionAlgorithm};
use super::model::BackgroundModel;

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
        let width = frame.width();
        let height = frame.height();
        let channels = frame.channels();

        if self.config.grid_width == 0 || self.config.grid_height == 0 {
            return Err(StackError::InvalidConfiguration(
                "Grid dimensions must be > 0".into(),
            ));
        }

        let block_width = width / self.config.grid_width;
        let block_height = height / self.config.grid_height;

        if block_width == 0 || block_height == 0 {
            return Err(StackError::InvalidConfiguration(
                "Image too small for grid size".into(),
            ));
        }

        // Compute grid medians for each channel
        let mut grid_values =
            vec![vec![0.0f32; self.config.grid_width * self.config.grid_height]; channels];

        {
            let _span = tracing::info_span!("compute_grid_medians").entered();
            // Process channels in parallel, and rows within each channel in parallel
            grid_values
                .par_iter_mut()
                .enumerate()
                .for_each(|(channel, channel_grid)| {
                    channel_grid
                        .par_chunks_mut(self.config.grid_width)
                        .enumerate()
                        .for_each(|(gy, row)| {
                            let mut buffer = Vec::with_capacity(block_width * block_height);
                            let mut mad_buffer = Vec::with_capacity(block_width * block_height);
                            for gx in 0..self.config.grid_width {
                                let x_start = gx * block_width;
                                let y_start = gy * block_height;
                                let x_end = if gx == self.config.grid_width - 1 {
                                    width
                                } else {
                                    x_start + block_width
                                };
                                let y_end = if gy == self.config.grid_height - 1 {
                                    height
                                } else {
                                    y_start + block_height
                                };

                                row[gx] = self.compute_block_median(
                                    frame, x_start, y_start, x_end, y_end, channel, &mut buffer, &mut mad_buffer
                                );
                            }
                        });
                });
        }

        match self.config.algorithm {
            BackgroundExtractionAlgorithm::GridBilinear => {
                let span = tracing::info_span!("grid_bilinear_estimate").entered();
                let model = BackgroundModel::new(
                    grid_values,
                    self.config.grid_width,
                    self.config.grid_height,
                    width,
                    height,
                    channels,
                    self.config.gradient_only,
                    self.config.reference_percentile,
                    self.config.aggressiveness,
                );
                drop(span);
                Ok(model)
            }
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
