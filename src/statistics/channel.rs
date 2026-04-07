/// Statistics for a single color channel
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelStats {
    /// Median value (robust center estimate)
    pub median: f32,
    /// Median Absolute Deviation (raw, before scaling)
    pub mad: f32,
    /// MAD scaled to Gaussian sigma equivalent (σ = 1.4826 × MAD)
    pub sigma: f32,
    /// Minimum value in sampled data
    pub min: f32,
    /// Maximum value in sampled data
    pub max: f32,
}

impl ChannelStats {
    /// Create new channel statistics
    pub fn new(median: f32, mad: f32, min: f32, max: f32) -> Self {
        // Scale MAD to sigma: for Gaussian distribution, σ = 1.4826 × MAD
        let sigma = mad * 1.4826;
        Self {
            median,
            mad,
            sigma,
            min,
            max,
        }
    }

    /// Returns the suggested black point for autostretch
    /// Typically: median - 2.8 * sigma (clips ~0.5% of background)
    pub fn suggested_black_point(&self, sigma_factor: f32) -> f32 {
        (self.median - sigma_factor * self.sigma).max(0.0)
    }

    /// Returns the data range above the noise floor
    pub fn signal_range(&self) -> f32 {
        self.max - self.median
    }
}
