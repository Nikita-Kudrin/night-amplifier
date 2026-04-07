use super::format::PixelFormat;
use super::Frame;
use crate::debayer::{
    detect_cfa_pattern, CfaPattern, DebayerAlgorithm, DebayerConfig, Debayerer,
    PatternDetectionResult,
};
use crate::error::{Result, StackError};
use tracing::instrument;

impl Frame {
    /// Creates a new Frame from raw 8-bit or 16-bit image data
    #[instrument(skip(raw), fields(format = ?format, resolution = %format!("{}x{}x{}", width, height, channels), buffer_size = raw.len()))]
    pub fn from_raw(
        raw: &[u8],
        width: usize,
        height: usize,
        channels: usize,
        format: PixelFormat,
    ) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        let pixel_count = width * height * channels;
        let expected_bytes = pixel_count * format.bytes_per_channel();

        if raw.len() != expected_bytes {
            return Err(StackError::BufferSizeMismatch {
                expected: expected_bytes,
                actual: raw.len(),
            });
        }

        let mut data = Vec::with_capacity(pixel_count);
        let max_value = format.max_value();
        let inv_max = 1.0 / max_value;

        match format {
            PixelFormat::Rgb8 | PixelFormat::Bayer8 => {
                data.extend(raw.iter().map(|&v| v as f32 * inv_max));
            }
            PixelFormat::Rgb16 | PixelFormat::Bayer16 => {
                for chunk in raw.chunks_exact(2) {
                    let value = u16::from_le_bytes([chunk[0], chunk[1]]);
                    data.push(value as f32 * inv_max);
                }
            }
            PixelFormat::Rgb16Be | PixelFormat::Bayer16Be => {
                for chunk in raw.chunks_exact(2) {
                    let value = u16::from_be_bytes([chunk[0], chunk[1]]);
                    data.push(value as f32 * inv_max);
                }
            }
        }

        Ok(Self {
            data,
            width,
            height,
            channels,
        })
    }

    /// Creates a new RGB Frame from raw Bayer pattern data with debayering
    #[instrument(skip(raw), fields(format = ?format, pattern = ?pattern, resolution = %format!("{}x{}", width, height)))]
    pub fn from_bayer(
        raw: &[u8],
        width: usize,
        height: usize,
        format: PixelFormat,
        pattern: CfaPattern,
    ) -> Result<Self> {
        if !format.is_bayer() {
            return Err(StackError::InvalidConfiguration(
                "from_bayer requires a Bayer pixel format".to_string(),
            ));
        }

        let mono_frame = Self::from_raw(raw, width, height, 1, format)?;
        let debayerer = Debayerer::new(DebayerConfig::new(pattern));
        debayerer.debayer(&mono_frame)
    }

    /// Creates a new RGB Frame from raw Bayer pattern data with custom debayer config
    pub fn from_bayer_with_config(
        raw: &[u8],
        width: usize,
        height: usize,
        format: PixelFormat,
        config: DebayerConfig,
    ) -> Result<Self> {
        if !format.is_bayer() {
            return Err(StackError::InvalidConfiguration(
                "from_bayer_with_config requires a Bayer pixel format".to_string(),
            ));
        }

        let mono_frame = Self::from_raw(raw, width, height, 1, format)?;
        let debayerer = Debayerer::new(config);
        debayerer.debayer(&mono_frame)
    }

    /// Creates a new RGB Frame from raw Bayer data with automatic pattern detection
    #[instrument(skip(raw), fields(format = ?format, resolution = %format!("{}x{}", width, height)))]
    pub fn from_bayer_auto(
        raw: &[u8],
        width: usize,
        height: usize,
        format: PixelFormat,
    ) -> Result<(Self, PatternDetectionResult)> {
        if !format.is_bayer() {
            return Err(StackError::InvalidConfiguration(
                "from_bayer_auto requires a Bayer pixel format".to_string(),
            ));
        }

        let mono_frame = Self::from_raw(raw, width, height, 1, format)?;
        let detection = detect_cfa_pattern(&mono_frame)?;
        let debayerer = Debayerer::new(DebayerConfig::new(detection.pattern));
        let rgb_frame = debayerer.debayer(&mono_frame)?;

        Ok((rgb_frame, detection))
    }

    /// Creates a new RGB Frame from raw Bayer data with auto-detection and specified algorithm
    pub fn from_bayer_auto_with_algorithm(
        raw: &[u8],
        width: usize,
        height: usize,
        format: PixelFormat,
        algorithm: DebayerAlgorithm,
    ) -> Result<(Self, PatternDetectionResult)> {
        if !format.is_bayer() {
            return Err(StackError::InvalidConfiguration(
                "from_bayer_auto_with_algorithm requires a Bayer pixel format".to_string(),
            ));
        }

        let mono_frame = Self::from_raw(raw, width, height, 1, format)?;
        let detection = detect_cfa_pattern(&mono_frame)?;
        let config = DebayerConfig::new(detection.pattern).with_algorithm(algorithm);
        let debayerer = Debayerer::new(config);
        let rgb_frame = debayerer.debayer(&mono_frame)?;

        Ok((rgb_frame, detection))
    }

    /// Creates a new Frame filled with zeros (black frame)
    pub fn zeros(width: usize, height: usize, channels: usize) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        Ok(Self {
            data: vec![0.0; width * height * channels],
            width,
            height,
            channels,
        })
    }

    /// Creates a new Frame filled with a constant value
    pub fn filled(width: usize, height: usize, channels: usize, value: f32) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        Ok(Self {
            data: vec![value; width * height * channels],
            width,
            height,
            channels,
        })
    }

    /// Creates a Frame from existing f32 data
    pub fn from_f32_vec(
        data: Vec<f32>,
        width: usize,
        height: usize,
        channels: usize,
    ) -> Result<Self> {
        let expected = width * height * channels;
        if data.len() != expected {
            return Err(StackError::BufferSizeMismatch {
                expected,
                actual: data.len(),
            });
        }

        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        Ok(Self {
            data,
            width,
            height,
            channels,
        })
    }
}
