use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::plugins::Plugins;
use tracing::{debug, instrument, warn};

use super::classify::{classify, NodeLayout};
use super::config::{BackgroundConfig, BackgroundExtractionAlgorithm};
use super::grid::{compute_box_size, median, sample_nodes};
use super::model::BackgroundModel;

/// Background extractor for light pollution removal
pub struct BackgroundExtractor {
    pub(crate) config: BackgroundConfig,
    /// Where RBF comes from: the process's [installed](Plugins::installed) set unless
    /// [`Self::with_plugins`] says otherwise.
    plugins: Plugins,
}

impl BackgroundExtractor {
    /// Create a new background extractor with the given configuration
    pub fn new(config: BackgroundConfig) -> Self {
        Self {
            config,
            plugins: Plugins::installed(),
        }
    }

    pub fn with_plugins(mut self, plugins: Plugins) -> Self {
        self.plugins = plugins;
        self
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
                if let Some(plugin) = self.plugins.background() {
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

    /// Bilinear background estimation through the classified node grid.
    ///
    /// Nodes hug both edges and add an inset line ([`NodeLayout`]); [`classify`] decides
    /// which are sky and completes the rest; the model interpolates the completed grid by
    /// delta-stepping. Too few sky nodes to fit a plane falls back to a flat median.
    fn estimate_bilinear(&self, frame: &Frame) -> Result<BackgroundModel> {
        let (width, height, channels) = (frame.width(), frame.height(), frame.channels());
        let (cols, rows) = (self.config.grid_width, self.config.grid_height);
        if cols < 2 || rows < 2 {
            return Err(StackError::InvalidConfiguration(
                "Grid dimensions must be >= 2 for bilinear interpolation".into(),
            ));
        }
        if width < cols || height < rows {
            return Err(StackError::InvalidConfiguration(
                "Image too small for the configured grid size".into(),
            ));
        }

        let layout = NodeLayout::new(width, height, cols, rows, self.config.edge_inset);
        let samples = sample_nodes(frame, &layout.nodes(), compute_box_size(width));
        let stats_channel = if channels > 1 { 1 } else { 0 };
        let grid = {
            let _span = tracing::info_span!("bilinear_classify").entered();
            match classify(&layout, &samples, stats_channel, self.config.protection.as_deref()) {
                Some(classified) => {
                    debug!(
                        sky = classified.sky.iter().filter(|&&s| s).count(),
                        nodes = classified.sky.len(),
                        "Bilinear background nodes"
                    );
                    classified.values
                }
                None => {
                    warn!("Too few sky nodes to fit; falling back to flat-field subtraction");
                    flat_fallback(&samples)
                }
            }
        };

        Ok(BackgroundModel::with_node_coords(
            grid,
            layout.cols(),
            layout.rows(),
            width,
            height,
            channels,
            self.config.gradient_only,
            self.config.reference_percentile,
            self.config.aggressiveness,
            layout.xs,
            layout.ys,
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

/// Each channel's median node value at every node.
fn flat_fallback(samples: &[Vec<Option<f32>>]) -> Vec<Vec<f32>> {
    samples
        .iter()
        .map(|channel| {
            let mut known: Vec<f32> = channel.iter().flatten().copied().collect();
            vec![median(&mut known); channel.len()]
        })
        .collect()
}
