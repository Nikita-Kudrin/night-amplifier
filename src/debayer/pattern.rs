//! CFA (Color Filter Array) pattern definitions for Bayer sensors

/// Color Filter Array (CFA) pattern for Bayer sensors
///
/// The pattern describes which color filter is over each pixel in a 2x2 grid,
/// starting from the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CfaPattern {
    /// Red-Green / Green-Blue pattern (most common)
    /// ```text
    /// R G
    /// G B
    /// ```
    #[default]
    Rggb,

    /// Blue-Green / Green-Red pattern
    /// ```text
    /// B G
    /// G R
    /// ```
    Bggr,

    /// Green-Red / Blue-Green pattern
    /// ```text
    /// G R
    /// B G
    /// ```
    Grbg,

    /// Green-Blue / Red-Green pattern
    /// ```text
    /// G B
    /// R G
    /// ```
    Gbrg,
}

impl CfaPattern {
    /// Returns the color channel (0=R, 1=G, 2=B) for a pixel at (x, y)
    #[inline]
    pub fn color_at(&self, x: usize, y: usize) -> usize {
        let x_odd = x & 1;
        let y_odd = y & 1;

        match (self, y_odd, x_odd) {
            // RGGB pattern
            (CfaPattern::Rggb, 0, 0) => 0, // R
            (CfaPattern::Rggb, 0, 1) => 1, // G
            (CfaPattern::Rggb, 1, 0) => 1, // G
            (CfaPattern::Rggb, 1, 1) => 2, // B

            // BGGR pattern
            (CfaPattern::Bggr, 0, 0) => 2, // B
            (CfaPattern::Bggr, 0, 1) => 1, // G
            (CfaPattern::Bggr, 1, 0) => 1, // G
            (CfaPattern::Bggr, 1, 1) => 0, // R

            // GRBG pattern
            (CfaPattern::Grbg, 0, 0) => 1, // G
            (CfaPattern::Grbg, 0, 1) => 0, // R
            (CfaPattern::Grbg, 1, 0) => 2, // B
            (CfaPattern::Grbg, 1, 1) => 1, // G

            // GBRG pattern
            (CfaPattern::Gbrg, 0, 0) => 1, // G
            (CfaPattern::Gbrg, 0, 1) => 2, // B
            (CfaPattern::Gbrg, 1, 0) => 0, // R
            (CfaPattern::Gbrg, 1, 1) => 1, // G

            _ => unreachable!(),
        }
    }

    /// Returns whether the pixel at (x, y) is a green pixel
    #[inline]
    pub fn is_green(&self, x: usize, y: usize) -> bool {
        self.color_at(x, y) == 1
    }

    /// Returns all four CFA patterns for iteration
    pub fn all() -> [CfaPattern; 4] {
        [
            CfaPattern::Rggb,
            CfaPattern::Bggr,
            CfaPattern::Grbg,
            CfaPattern::Gbrg,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfa_pattern_rggb() {
        let pattern = CfaPattern::Rggb;
        assert_eq!(pattern.color_at(0, 0), 0); // R
        assert_eq!(pattern.color_at(1, 0), 1); // G
        assert_eq!(pattern.color_at(0, 1), 1); // G
        assert_eq!(pattern.color_at(1, 1), 2); // B
    }

    #[test]
    fn test_cfa_pattern_bggr() {
        let pattern = CfaPattern::Bggr;
        assert_eq!(pattern.color_at(0, 0), 2); // B
        assert_eq!(pattern.color_at(1, 0), 1); // G
        assert_eq!(pattern.color_at(0, 1), 1); // G
        assert_eq!(pattern.color_at(1, 1), 0); // R
    }

    #[test]
    fn test_cfa_pattern_all() {
        let patterns = CfaPattern::all();
        assert_eq!(patterns.len(), 4);
        assert!(patterns.contains(&CfaPattern::Rggb));
        assert!(patterns.contains(&CfaPattern::Bggr));
        assert!(patterns.contains(&CfaPattern::Grbg));
        assert!(patterns.contains(&CfaPattern::Gbrg));
    }

    #[test]
    fn test_is_green() {
        let pattern = CfaPattern::Rggb;
        assert!(!pattern.is_green(0, 0)); // R
        assert!(pattern.is_green(1, 0)); // G
        assert!(pattern.is_green(0, 1)); // G
        assert!(!pattern.is_green(1, 1)); // B
    }
}
