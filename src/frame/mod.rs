//! Core Frame data structure for astronomy image processing
//!
//! The Frame struct holds image data as a contiguous Vec<f32> for high-precision
//! arithmetic operations required in image stacking.

mod factory;
mod format;
mod ops;

pub use format::PixelFormat;

/// A frame of image data stored as normalized f32 values in [0.0, 1.0]
#[derive(Debug, Clone)]
pub struct Frame {
    /// Pixel data as normalized f32 values
    data: Vec<f32>,
    /// Image width in pixels
    width: usize,
    /// Image height in pixels
    height: usize,
    /// Number of channels (typically 1 for mono, 3 for RGB)
    channels: usize,
}

impl Frame {
    /// Returns the image width in pixels
    #[inline]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Returns the image height in pixels
    #[inline]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Returns the number of channels
    #[inline]
    pub const fn channels(&self) -> usize {
        self.channels
    }

    /// Returns the total number of pixels (width * height)
    #[inline]
    pub const fn pixel_count(&self) -> usize {
        self.width * self.height
    }

    /// Returns the total number of samples (width * height * channels)
    #[inline]
    pub fn sample_count(&self) -> usize {
        self.data.len()
    }

    /// Returns the memory size in bytes used by the pixel data
    #[inline]
    pub fn memory_size(&self) -> usize {
        self.data.len() * std::mem::size_of::<f32>()
    }

    /// Returns an immutable reference to the underlying data
    #[inline]
    pub fn data(&self) -> &[f32] {
        &self.data
    }

    /// Returns a mutable reference to the underlying data
    #[inline]
    pub fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    /// Consumes the Frame and returns the underlying Vec<f32>
    #[inline]
    pub fn into_data(self) -> Vec<f32> {
        self.data
    }

    /// Returns the pixel value at the given coordinates and channel
    #[inline]
    pub fn get_pixel(&self, x: usize, y: usize, channel: usize) -> f32 {
        debug_assert!(x < self.width && y < self.height && channel < self.channels);
        let idx = (y * self.width + x) * self.channels + channel;
        self.data[idx]
    }

    /// Sets the pixel value at the given coordinates and channel
    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, channel: usize, value: f32) {
        debug_assert!(x < self.width && y < self.height && channel < self.channels);
        let idx = (y * self.width + x) * self.channels + channel;
        self.data[idx] = value;
    }

    /// Checks if this frame has the same dimensions as another
    #[inline]
    pub fn dimensions_match(&self, other: &Frame) -> bool {
        self.width == other.width && self.height == other.height && self.channels == other.channels
    }

    /// Area-average downsample by a given factor
    pub fn downsample(&self, factor: usize) -> crate::error::Result<Self> {
        use rayon::prelude::*;

        if factor == 0 {
            return Err(crate::error::StackError::InvalidConfiguration(
                "Downsample factor must be > 0".into(),
            ));
        }

        if factor == 1 {
            return Ok(self.clone());
        }

        let src_width = self.width;
        let src_height = self.height;
        let channels = self.channels;

        let dst_width = src_width / factor;
        let dst_height = src_height / factor;

        if dst_width == 0 || dst_height == 0 {
            return Err(crate::error::StackError::InvalidConfiguration(
                "Downsample factor too large for image dimensions".into(),
            ));
        }

        let inv_area = 1.0 / (factor * factor) as f32;
        let mut output = vec![0.0f32; dst_width * dst_height * channels];
        let src_data = self.data.as_slice();
        let src_width = self.width;

        output
            .par_chunks_mut(dst_width * channels)
            .enumerate()
            .for_each(|(dst_y, row)| {
                let src_y_start = dst_y * factor;
                for dst_x in 0..dst_width {
                    let src_x_start = dst_x * factor;
                    
                    if channels == 1 {
                        let mut sum = 0.0f32;
                        for sy in 0..factor {
                            let row_start = (src_y_start + sy) * src_width;
                            for sx in 0..factor {
                                sum += src_data[row_start + src_x_start + sx];
                            }
                        }
                        row[dst_x] = sum * inv_area;
                    } else if channels == 3 {
                        let mut sum_r = 0.0f32;
                        let mut sum_g = 0.0f32;
                        let mut sum_b = 0.0f32;
                        for sy in 0..factor {
                            let row_start = (src_y_start + sy) * src_width * 3;
                            for sx in 0..factor {
                                let pixel_start = row_start + (src_x_start + sx) * 3;
                                sum_r += src_data[pixel_start];
                                sum_g += src_data[pixel_start + 1];
                                sum_b += src_data[pixel_start + 2];
                            }
                        }
                        let out_idx = dst_x * 3;
                        row[out_idx] = sum_r * inv_area;
                        row[out_idx + 1] = sum_g * inv_area;
                        row[out_idx + 2] = sum_b * inv_area;
                    } else {
                        // Fallback for other channel counts
                        let mut sums = vec![0.0f32; channels];
                        for sy in 0..factor {
                            let row_start = (src_y_start + sy) * src_width * channels;
                            for sx in 0..factor {
                                let pixel_start = row_start + (src_x_start + sx) * channels;
                                for c in 0..channels {
                                    sums[c] += src_data[pixel_start + c];
                                }
                            }
                        }
                        let out_idx = dst_x * channels;
                        for c in 0..channels {
                            row[out_idx + c] = sums[c] * inv_area;
                        }
                    }
                }
            });

        Self::from_f32_vec(output, dst_width, dst_height, channels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_from_rgb8() {
        let raw = vec![0u8, 128, 255];
        let frame = Frame::from_raw(&raw, 1, 1, 3, PixelFormat::Rgb8).unwrap();

        assert_eq!(frame.width(), 1);
        assert_eq!(frame.height(), 1);
        assert_eq!(frame.channels(), 3);

        assert!((frame.get_pixel(0, 0, 0) - 0.0).abs() < 1e-6);
        assert!((frame.get_pixel(0, 0, 1) - 128.0 / 255.0).abs() < 1e-6);
        assert!((frame.get_pixel(0, 0, 2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_frame_from_rgb16_le() {
        let raw = vec![0x00, 0x00, 0x00, 0x80, 0xFF, 0xFF];
        let frame = Frame::from_raw(&raw, 1, 1, 3, PixelFormat::Rgb16).unwrap();

        assert!((frame.get_pixel(0, 0, 0) - 0.0).abs() < 1e-6);
        assert!((frame.get_pixel(0, 0, 1) - 0.5).abs() < 0.001);
        assert!((frame.get_pixel(0, 0, 2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_frame_zeros() {
        let frame = Frame::zeros(10, 10, 3).unwrap();
        assert!(frame.data().iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_frame_filled() {
        let frame = Frame::filled(10, 10, 3, 0.5).unwrap();
        assert!(frame.data().iter().all(|&v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn test_frame_clamp() {
        let data = vec![-0.5, 0.5, 1.5];
        let mut frame = Frame::from_f32_vec(data, 1, 1, 3).unwrap();
        frame.clamp();
        assert_eq!(frame.data(), &[0.0, 0.5, 1.0]);
    }

    #[test]
    fn test_memory_size() {
        let frame = Frame::zeros(1920, 1080, 3).unwrap();
        assert_eq!(frame.memory_size(), 1920 * 1080 * 3 * 4);
    }
}
