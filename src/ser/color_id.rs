//! SER color format identifiers.

use crate::debayer::CfaPattern;

/// SER color format identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SerColorId {
    /// Grayscale (1 channel)
    Mono = 0,
    /// Raw Bayer RGGB pattern
    BayerRggb = 8,
    /// Raw Bayer GRBG pattern
    BayerGrbg = 9,
    /// Raw Bayer GBRG pattern
    BayerGbrg = 10,
    /// Raw Bayer BGGR pattern
    BayerBggr = 11,
    /// RGB color (3 channels, R-G-B order)
    Rgb = 100,
    /// BGR color (3 channels, B-G-R order)
    Bgr = 101,
}

impl SerColorId {
    /// Creates a SerColorId from a raw u32 value.
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Mono),
            8 => Some(Self::BayerRggb),
            9 => Some(Self::BayerGrbg),
            10 => Some(Self::BayerGbrg),
            11 => Some(Self::BayerBggr),
            100 => Some(Self::Rgb),
            101 => Some(Self::Bgr),
            _ => None,
        }
    }

    /// Returns the number of channels for this color format.
    pub fn channels(&self) -> usize {
        match self {
            Self::Mono | Self::BayerRggb | Self::BayerGrbg | Self::BayerGbrg | Self::BayerBggr => 1,
            Self::Rgb | Self::Bgr => 3,
        }
    }

    /// Returns true if this is a Bayer format.
    pub fn is_bayer(&self) -> bool {
        matches!(
            self,
            Self::BayerRggb | Self::BayerGrbg | Self::BayerGbrg | Self::BayerBggr
        )
    }

    /// Converts to CfaPattern if this is a Bayer format.
    pub fn to_cfa_pattern(&self) -> Option<CfaPattern> {
        match self {
            Self::BayerRggb => Some(CfaPattern::Rggb),
            Self::BayerGrbg => Some(CfaPattern::Grbg),
            Self::BayerGbrg => Some(CfaPattern::Gbrg),
            Self::BayerBggr => Some(CfaPattern::Bggr),
            _ => None,
        }
    }

    /// Creates from a CfaPattern.
    pub fn from_cfa_pattern(pattern: CfaPattern) -> Self {
        match pattern {
            CfaPattern::Rggb => Self::BayerRggb,
            CfaPattern::Grbg => Self::BayerGrbg,
            CfaPattern::Gbrg => Self::BayerGbrg,
            CfaPattern::Bggr => Self::BayerBggr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ser_color_id_channels() {
        assert_eq!(SerColorId::Mono.channels(), 1);
        assert_eq!(SerColorId::BayerRggb.channels(), 1);
        assert_eq!(SerColorId::Rgb.channels(), 3);
        assert_eq!(SerColorId::Bgr.channels(), 3);
    }

    #[test]
    fn test_ser_bayer_color_id() {
        assert!(SerColorId::BayerRggb.is_bayer());
        assert!(SerColorId::BayerGrbg.is_bayer());
        assert!(!SerColorId::Mono.is_bayer());
        assert!(!SerColorId::Rgb.is_bayer());

        assert_eq!(
            SerColorId::BayerRggb.to_cfa_pattern(),
            Some(CfaPattern::Rggb)
        );
        assert_eq!(
            SerColorId::from_cfa_pattern(CfaPattern::Bggr),
            SerColorId::BayerBggr
        );
    }

    #[test]
    fn test_from_u32() {
        assert_eq!(SerColorId::from_u32(0), Some(SerColorId::Mono));
        assert_eq!(SerColorId::from_u32(8), Some(SerColorId::BayerRggb));
        assert_eq!(SerColorId::from_u32(100), Some(SerColorId::Rgb));
        assert_eq!(SerColorId::from_u32(999), None);
    }
}
