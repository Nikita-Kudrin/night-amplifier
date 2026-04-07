/// Configuration for star detection
#[derive(Debug, Clone)]
pub struct DetectionConfig {
    /// Detection threshold in sigma above background (default: 5.0)
    pub sigma_threshold: f32,
    /// Radius for local maximum search (default: 3 pixels)
    pub search_radius: usize,
    /// Radius for centroid calculation (default: 5 pixels)
    pub centroid_radius: usize,
    /// Maximum fraction of flux in peak pixel to reject hot pixels (default: 0.9)
    pub hot_pixel_threshold: f32,
    /// Minimum number of pixels above threshold to be a valid star (default: 3)
    pub min_star_pixels: usize,
    /// Border margin to ignore (avoid edge effects)
    pub border_margin: usize,
    /// Maximum number of stars to return (brightest first)
    pub max_stars: Option<usize>,
    /// Minimum SNR for a valid detection
    pub min_snr: f32,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            sigma_threshold: 5.0,
            search_radius: 3,
            centroid_radius: 5,
            hot_pixel_threshold: 0.9,
            min_star_pixels: 3,
            border_margin: 10,
            max_stars: Some(200),
            min_snr: 5.0,
        }
    }
}

impl DetectionConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_sigma(mut self, sigma: f32) -> Self {
        self.sigma_threshold = sigma;
        self
    }

    pub fn with_search_radius(mut self, radius: usize) -> Self {
        self.search_radius = radius;
        self
    }

    pub fn with_centroid_radius(mut self, radius: usize) -> Self {
        self.centroid_radius = radius;
        self
    }

    pub fn with_max_stars(mut self, max: usize) -> Self {
        self.max_stars = Some(max);
        self
    }

    pub fn unlimited_stars(mut self) -> Self {
        self.max_stars = None;
        self
    }

    pub fn with_min_snr(mut self, snr: f32) -> Self {
        self.min_snr = snr;
        self
    }

    pub fn with_hot_pixel_threshold(mut self, threshold: f32) -> Self {
        self.hot_pixel_threshold = threshold;
        self
    }

    pub fn with_min_star_pixels(mut self, pixels: usize) -> Self {
        self.min_star_pixels = pixels;
        self
    }

    /// Sensitive configuration for faint images
    pub fn sensitive() -> Self {
        Self {
            sigma_threshold: 3.0,
            search_radius: 3,
            centroid_radius: 5,
            hot_pixel_threshold: 0.85,
            min_star_pixels: 2,
            border_margin: 10,
            max_stars: Some(200),
            min_snr: 2.0,
        }
    }

    /// Aggressive configuration for very faint images
    pub fn aggressive() -> Self {
        Self {
            sigma_threshold: 2.5,
            search_radius: 2,
            centroid_radius: 4,
            hot_pixel_threshold: 0.80,
            min_star_pixels: 2,
            border_margin: 8,
            max_stars: Some(300),
            min_snr: 1.5,
        }
    }

    /// Fast configuration optimized for speed
    pub fn fast() -> Self {
        Self {
            sigma_threshold: 4.0,
            search_radius: 2,
            centroid_radius: 3,
            hot_pixel_threshold: 0.85,
            min_star_pixels: 2,
            border_margin: 10,
            max_stars: Some(30),
            min_snr: 3.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detection_config_builder() {
        let config = DetectionConfig::new()
            .with_sigma(4.0)
            .with_search_radius(5)
            .with_centroid_radius(7)
            .with_max_stars(100);

        assert!((config.sigma_threshold - 4.0).abs() < 1e-6);
        assert_eq!(config.search_radius, 5);
        assert_eq!(config.centroid_radius, 7);
        assert_eq!(config.max_stars, Some(100));
    }
}
