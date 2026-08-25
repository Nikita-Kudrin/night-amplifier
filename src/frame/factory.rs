use super::format::PixelFormat;
use super::Frame;
use crate::debayer::{CfaPattern, DebayerConfig, Debayerer};
use crate::error::{Result, StackError};
use tracing::instrument;

/// Writes `sample(i, c)` into plane-major `data`, one contiguous run per channel.
///
/// The three `PixelFormat` arms differ only in how a sample is decoded, so they share
/// this traversal rather than repeating the channel ladder three times.
#[inline]
fn scatter_to_planes(
    data: &mut [f32],
    pixels: usize,
    channels: usize,
    sample: impl Fn(usize, usize) -> f32,
) {
    if channels == 1 {
        for (i, slot) in data[..pixels].iter_mut().enumerate() {
            *slot = sample(i, 0);
        }
        return;
    }

    for (c, plane) in data.chunks_exact_mut(pixels).enumerate() {
        for (i, slot) in plane.iter_mut().enumerate() {
            *slot = sample(i, c);
        }
    }
}

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

        let mut data = vec![0.0f32; pixel_count];
        let max_value = format.max_value();
        let inv_max = 1.0 / max_value;

        // Split into plane slices once, so each channel is a sequential store stream.
        // Indexing `data[c * area + i]` per sample made this three bounds-checked
        // scatter writes per pixel on every incoming camera frame.
        match format {
            PixelFormat::Rgb8 | PixelFormat::Bayer8 => {
                scatter_to_planes(&mut data, raw.len() / channels, channels, |i, c| {
                    raw[i * channels + c] as f32 * inv_max
                });
            }
            PixelFormat::Rgb16 | PixelFormat::Bayer16 => {
                let u16_raw = raw.as_chunks::<2>().0;
                scatter_to_planes(&mut data, u16_raw.len() / channels, channels, |i, c| {
                    let s = u16_raw[i * channels + c];
                    u16::from_le_bytes([s[0], s[1]]) as f32 * inv_max
                });
            }
            PixelFormat::Rgb16Be | PixelFormat::Bayer16Be => {
                let u16_raw = raw.as_chunks::<2>().0;
                scatter_to_planes(&mut data, u16_raw.len() / channels, channels, |i, c| {
                    let s = u16_raw[i * channels + c];
                    u16::from_be_bytes([s[0], s[1]]) as f32 * inv_max
                });
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
