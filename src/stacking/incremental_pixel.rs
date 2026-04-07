//! Incremental pixel statistics using Welford's Online Algorithm.

/// Maintains running statistics and the final blended pixel value in O(1) space.
#[derive(Clone, Copy)]
pub struct IncrementalPixel {
    /// Total accumulated weight (W_n)
    pub weight_sum: f32,
    /// Running weighted mean (M_n) - This IS your final pixel value!
    pub mean: f32,
    /// Running sum of squared differences (S_n) - Used for variance estimation
    pub m2: f32,
    /// Number of valid frames blended into this pixel
    pub count: u16,
}

impl IncrementalPixel {
    #[inline]
    pub fn new() -> Self {
        Self {
            weight_sum: 0.0,
            mean: 0.0,
            m2: 0.0,
            count: 0,
        }
    }

    /// West's algorithm for incrementally updating weighted variance and mean in O(1)
    #[inline]
    pub fn blend(&mut self, value: f32, weight: f32) {
        self.count += 1;
        let temp_weight_sum = self.weight_sum + weight;

        let diff = value - self.mean;
        let r = diff * weight / temp_weight_sum;

        self.mean += r;
        self.m2 += self.weight_sum * diff * r; // Update running variance
        self.weight_sum = temp_weight_sum;
    }

    #[inline]
    pub fn reset(&mut self) {
        self.count = 0;
        self.weight_sum = 0.0;
        self.mean = 0.0;
        self.m2 = 0.0;
    }
}

impl Default for IncrementalPixel {
    fn default() -> Self {
        Self::new()
    }
}
