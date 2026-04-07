/// Pixel format of the input raw data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 8-bit unsigned integer per channel (0-255)
    Rgb8,
    /// 16-bit unsigned integer per channel, little-endian (0-65535)
    Rgb16,
    /// 16-bit unsigned integer per channel, big-endian (0-65535)
    Rgb16Be,
    /// 8-bit Bayer pattern (single channel, requires debayering)
    Bayer8,
    /// 16-bit Bayer pattern, little-endian (single channel, requires debayering)
    Bayer16,
    /// 16-bit Bayer pattern, big-endian (single channel, requires debayering)
    Bayer16Be,
}

impl PixelFormat {
    /// Returns the number of bytes per channel for this format
    #[inline]
    pub const fn bytes_per_channel(self) -> usize {
        match self {
            PixelFormat::Rgb8 | PixelFormat::Bayer8 => 1,
            PixelFormat::Rgb16
            | PixelFormat::Rgb16Be
            | PixelFormat::Bayer16
            | PixelFormat::Bayer16Be => 2,
        }
    }

    /// Returns the maximum value for this format (used for normalization)
    #[inline]
    pub const fn max_value(self) -> f32 {
        match self {
            PixelFormat::Rgb8 | PixelFormat::Bayer8 => 255.0,
            PixelFormat::Rgb16
            | PixelFormat::Rgb16Be
            | PixelFormat::Bayer16
            | PixelFormat::Bayer16Be => 65535.0,
        }
    }

    /// Returns true if this format is a Bayer pattern (requires debayering)
    #[inline]
    pub const fn is_bayer(self) -> bool {
        matches!(
            self,
            PixelFormat::Bayer8 | PixelFormat::Bayer16 | PixelFormat::Bayer16Be
        )
    }

    /// Returns the number of channels in the raw data for this format
    #[inline]
    pub const fn raw_channels(self) -> usize {
        match self {
            PixelFormat::Bayer8 | PixelFormat::Bayer16 | PixelFormat::Bayer16Be => 1,
            _ => 0, // Caller specifies channels for RGB formats
        }
    }
}
