use super::channel::ChannelStats;

/// Complete image statistics for all channels
#[derive(Debug, Clone)]
pub struct ImageStats {
    /// Per-channel statistics (R, G, B or single luminance)
    pub channels: Vec<ChannelStats>,
    /// Number of pixels sampled
    pub sample_count: usize,
}

impl ImageStats {
    /// Get statistics for a specific channel
    pub fn channel(&self, index: usize) -> Option<&ChannelStats> {
        self.channels.get(index)
    }

    /// Get the average median across all channels (useful for luminance-based stretch)
    pub fn mean_median(&self) -> f32 {
        if self.channels.is_empty() {
            return 0.0;
        }
        self.channels.iter().map(|c| c.median).sum::<f32>() / self.channels.len() as f32
    }

    /// Get the average sigma across all channels
    pub fn mean_sigma(&self) -> f32 {
        if self.channels.is_empty() {
            return 0.0;
        }
        self.channels.iter().map(|c| c.sigma).sum::<f32>() / self.channels.len() as f32
    }

    /// Get the minimum of all channel minimums
    pub fn global_min(&self) -> f32 {
        self.channels.iter().map(|c| c.min).fold(f32::MAX, f32::min)
    }

    /// Get the maximum of all channel maximums
    pub fn global_max(&self) -> f32 {
        self.channels.iter().map(|c| c.max).fold(f32::MIN, f32::max)
    }

    /// Check if the image appears to be mostly noise (low signal)
    pub fn is_low_signal(&self) -> bool {
        // If the signal range is less than 10x the noise, it's mostly noise
        let avg_sigma = self.mean_sigma();
        let max_signal = self.global_max() - self.mean_median();
        avg_sigma > 0.0 && max_signal < avg_sigma * 10.0
    }
}
