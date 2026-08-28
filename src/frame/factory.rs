use super::format::PixelFormat;
use super::Frame;
use crate::debayer::{CfaPattern, DebayerConfig, Debayerer};
use crate::error::{Result, StackError};
use tracing::instrument;

/// Writes `sample(i, c)` into plane-major `data`, one contiguous run per channel.
///
/// The three `PixelFormat` arms differ only in how a sample is decoded, so they share
/// this traversal rather than repeating the channel ladder three times.
///
/// # One pass over the source, not one per plane
///
/// The obvious shape — walk each output plane in turn, reading `raw[i * channels + c]` —
/// reads the *whole* source buffer once per channel. For a 2712x1538 RGB frame that is
/// three passes over 12.5 MB, each touching every cache line, where the interleaved
/// predecessor did one. This is the camera ingest path, so it runs per frame.
///
/// Instead the source is walked once and each sample is scattered to its plane. The
/// planes are split apart up front so the write is a sequential store stream per channel
/// rather than a bounds-checked `data[c * area + i]` per sample.
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

    if channels == 3 {
        let (r, rest) = data.split_at_mut(pixels);
        let (g, b) = rest.split_at_mut(pixels);
        for i in 0..pixels {
            r[i] = sample(i, 0);
            g[i] = sample(i, 1);
            b[i] = sample(i, 2);
        }
        return;
    }

    // Uncommon channel counts: still one pass over the source, collecting the plane
    // slices first so the inner write does not re-derive an offset per sample.
    let mut planes: Vec<&mut [f32]> = Vec::with_capacity(channels);
    let mut rest = &mut data[..pixels * channels];
    for _ in 0..channels {
        let (plane, tail) = rest.split_at_mut(pixels);
        planes.push(plane);
        rest = tail;
    }
    for i in 0..pixels {
        for (c, plane) in planes.iter_mut().enumerate() {
            plane[i] = sample(i, c);
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
