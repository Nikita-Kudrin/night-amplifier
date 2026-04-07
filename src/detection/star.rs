/// A detected star with sub-pixel position and brightness
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
    /// Sub-pixel X coordinate (centroid)
    pub x: f32,
    /// Sub-pixel Y coordinate (centroid)
    pub y: f32,
    /// Total flux (sum of background-subtracted pixel values)
    pub flux: f32,
    /// Peak pixel value (useful for saturation detection)
    pub peak: f32,
    /// Signal-to-noise ratio estimate
    pub snr: f32,
    /// Full Width at Half Maximum in pixels (optional, computed from second moment)
    pub fwhm: Option<f32>,
}

impl Star {
    pub fn new(x: f32, y: f32, flux: f32, peak: f32, snr: f32) -> Self {
        Self {
            x,
            y,
            flux,
            peak,
            snr,
            fwhm: None,
        }
    }

    /// Creates a star with all metrics including FWHM.
    pub fn with_fwhm(x: f32, y: f32, flux: f32, peak: f32, snr: f32, fwhm: f32) -> Self {
        Self {
            x,
            y,
            flux,
            peak,
            snr,
            fwhm: Some(fwhm),
        }
    }

    /// Returns the integer pixel coordinates (rounded)
    pub fn pixel_coords(&self) -> (usize, usize) {
        (self.x.round() as usize, self.y.round() as usize)
    }

    /// Calculates the distance to another star
    pub fn distance_to(&self, other: &Star) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_star_struct() {
        let star = Star::new(30.5, 40.2, 100.0, 0.8, 15.0);
        assert!((star.x - 30.5).abs() < 1e-6);
        assert!((star.y - 40.2).abs() < 1e-6);
        assert_eq!(star.pixel_coords(), (31, 40));
    }

    #[test]
    fn test_star_distance() {
        let star1 = Star::new(0.0, 0.0, 100.0, 0.8, 15.0);
        let star2 = Star::new(3.0, 4.0, 100.0, 0.8, 15.0);
        assert!((star1.distance_to(&star2) - 5.0).abs() < 1e-6);
    }
}
