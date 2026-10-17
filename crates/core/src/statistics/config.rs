/// Configuration for statistics computation
#[derive(Debug, Clone, Copy)]
pub struct StatsConfig {
    /// Maximum number of pixels to sample (for performance)
    /// Default: 100_000 (sufficient for accurate statistics)
    pub max_samples: usize,
    /// Minimum number of samples required for valid statistics
    /// Default: 1000
    pub min_samples: usize,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self {
            max_samples: 100_000,
            min_samples: 1000,
        }
    }
}

impl StatsConfig {
    /// Create a new configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum sample count
    pub fn with_max_samples(mut self, samples: usize) -> Self {
        self.max_samples = samples;
        self
    }

    /// Use all pixels (no sampling) - slower but more accurate
    pub fn full_precision(mut self) -> Self {
        self.max_samples = usize::MAX;
        self
    }
}
