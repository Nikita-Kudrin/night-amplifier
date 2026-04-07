//! Configuration types for planetary stacking.

/// Configuration for planetary stacking
#[derive(Debug, Clone)]
pub struct PlanetaryConfig {
    /// Percentage of best frames to stack (0.0-1.0, default: 0.1 = 10%)
    pub selection_percentage: f32,
    /// Minimum number of frames to stack regardless of percentage
    pub min_frames: usize,
    /// Maximum frames to stack (0 = unlimited)
    pub max_frames: usize,
    /// Search radius for alignment in pixels (default: 50)
    pub search_radius: usize,
    /// Region of interest for alignment (None = center crop)
    pub alignment_roi: Option<AlignmentRoi>,
    /// Stacking method (default: Percentile)
    pub stacking_method: PlanetaryStackMethod,
    /// Percentile to use for percentile stacking (default: 0.5 = median)
    pub percentile: f32,
    /// Quality metric to use for frame selection
    pub quality_metric: QualityMetric,
    /// Subpixel alignment precision (1 = pixel, 2 = half-pixel, 4 = quarter-pixel)
    pub subpixel_factor: usize,
}

impl Default for PlanetaryConfig {
    fn default() -> Self {
        Self {
            selection_percentage: 0.1,
            min_frames: 10,
            max_frames: 0,
            search_radius: 50,
            alignment_roi: None,
            stacking_method: PlanetaryStackMethod::Percentile,
            percentile: 0.5,
            quality_metric: QualityMetric::Laplacian,
            subpixel_factor: 2,
        }
    }
}

impl PlanetaryConfig {
    /// Creates a new configuration with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the selection percentage (e.g., 0.1 for best 10%)
    pub fn with_selection(mut self, percentage: f32) -> Self {
        self.selection_percentage = percentage.clamp(0.01, 1.0);
        self
    }

    /// Sets the search radius for alignment
    pub fn with_search_radius(mut self, radius: usize) -> Self {
        self.search_radius = radius;
        self
    }

    /// Sets the stacking method
    pub fn with_method(mut self, method: PlanetaryStackMethod) -> Self {
        self.stacking_method = method;
        self
    }

    /// Sets the quality metric
    pub fn with_quality_metric(mut self, metric: QualityMetric) -> Self {
        self.quality_metric = metric;
        self
    }

    /// Preset for lunar imaging (larger features, lower percentage)
    pub fn lunar() -> Self {
        Self {
            selection_percentage: 0.05,
            min_frames: 50,
            search_radius: 30,
            quality_metric: QualityMetric::Laplacian,
            ..Default::default()
        }
    }

    /// Preset for planetary imaging (Jupiter, Saturn, Mars)
    pub fn planetary() -> Self {
        Self {
            selection_percentage: 0.10,
            min_frames: 100,
            search_radius: 50,
            quality_metric: QualityMetric::Sobel,
            ..Default::default()
        }
    }

    /// Preset for solar imaging
    pub fn solar() -> Self {
        Self {
            selection_percentage: 0.15,
            min_frames: 30,
            search_radius: 40,
            quality_metric: QualityMetric::Laplacian,
            ..Default::default()
        }
    }

    /// Computes the number of frames to select based on config and total count.
    pub fn compute_selected_count(&self, total_frames: usize) -> usize {
        let count = ((total_frames as f32 * self.selection_percentage).ceil() as usize)
            .max(self.min_frames)
            .min(total_frames);

        if self.max_frames > 0 {
            count.min(self.max_frames)
        } else {
            count
        }
    }
}

/// Region of interest for alignment
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct AlignmentRoi {
    /// X coordinate of top-left corner
    pub x: usize,
    /// Y coordinate of top-left corner
    pub y: usize,
    /// Width of the ROI
    pub width: usize,
    /// Height of the ROI
    pub height: usize,
}

impl AlignmentRoi {
    /// Creates a new ROI
    pub fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates a centered ROI with the given size
    pub fn centered(frame_width: usize, frame_height: usize, roi_size: usize) -> Self {
        let x = (frame_width.saturating_sub(roi_size)) / 2;
        let y = (frame_height.saturating_sub(roi_size)) / 2;
        let width = roi_size.min(frame_width);
        let height = roi_size.min(frame_height);
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates an ROI centered at a specific coordinate
    pub fn centered_at(
        x_center: f32,
        y_center: f32,
        roi_width: usize,
        roi_height: usize,
        frame_width: usize,
        frame_height: usize,
    ) -> Self {
        let half_w = roi_width as f32 / 2.0;
        let half_h = roi_height as f32 / 2.0;

        let start_x = (x_center - half_w).max(0.0) as usize;
        let start_y = (y_center - half_h).max(0.0) as usize;

        let w = roi_width.min(frame_width.saturating_sub(start_x));
        let h = roi_height.min(frame_height.saturating_sub(start_y));

        Self {
            x: start_x,
            y: start_y,
            width: w,
            height: h,
        }
    }
}

/// Stacking method for planetary images
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanetaryStackMethod {
    /// Mean of all frames (fast but sensitive to outliers)
    Mean,
    /// Median of all frames (robust but memory intensive)
    Median,
    /// Specified percentile (default: 0.5 = median)
    Percentile,
    /// Quality-weighted mean
    WeightedMean,
}

/// Quality metric for frame selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityMetric {
    /// Laplacian variance (sharpness)
    Laplacian,
    /// Sobel gradient magnitude (edge strength)
    Sobel,
    /// Tenengrad (Sobel-based gradient)
    Tenengrad,
    /// Standard deviation (contrast)
    StdDev,
}

/// Statistics about the planetary stack
#[derive(Debug, Clone, Default)]
pub struct PlanetaryStackStats {
    /// Total number of frames collected
    pub total_frames: usize,
    /// Number of frames that will be stacked
    pub selected_frames: usize,
    /// Minimum quality score
    pub min_quality: f32,
    /// Maximum quality score
    pub max_quality: f32,
    /// Mean quality score
    pub mean_quality: f32,
    /// Maximum alignment offset (pixels)
    pub max_offset: f32,
}
