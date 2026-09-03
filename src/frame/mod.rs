//! Core Frame data structure: holds image data as a contiguous `Vec<f32>` for
//! high-precision stacking arithmetic, stored **plane-major**
//! (`idx = channel*(width*height) + y*width + x`) so SIMD and spatial filters read a
//! channel as a contiguous run, not a stride-3 gather.
//!
//! **`Frame` is planar; every 8-bit output format is interleaved** (JPEG/LZ4/PNG/SER
//! `Rgb`/`Bgr`; FITS NAXIS3=3 is the one exception, staying planar). Crossing that
//! boundary wrongly compiles cleanly and collapses colour toward grey — it has
//! already happened once, across eight output paths at once.
//!
//! Rules: use [`Frame::planes`]/[`planes_mut`]/[`channel_data`]/[`get_pixel`], never
//! `frame.data()` with `* channels` math (review flag); build fixtures with
//! [`Frame::set_pixel`], never hand-computed offsets (an offset-encoded fixture can't
//! catch a layout bug); `src/frame/layout_tests.rs` covers every output path — add a
//! row per format, per traversal, and sweep the whole interior, not one pixel
//! (`c * area + p` coincides with the planar read at `p % 3 == 0`). 8-bit conversion
//! always rounds via [`sample_to_u8`]; 16-bit writers truncate. Reuse
//! [`Frame::write_rgb8_into`] rather than re-deriving the gather.

mod factory;
mod format;
#[cfg(test)]
mod layout_tests;
mod ops;

pub use format::PixelFormat;
pub(crate) use ops::sample_to_u8;

/// A frame of image data stored as normalized f32 values in [0.0, 1.0]
#[derive(Clone)]
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

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("channels", &self.channels)
            .field("data_len", &self.data.len())
            .finish()
    }
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
        let idx = channel * (self.width * self.height) + y * self.width + x;
        self.data[idx]
    }

    /// Sets the pixel value at the given coordinates and channel
    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, channel: usize, value: f32) {
        debug_assert!(x < self.width && y < self.height && channel < self.channels);
        let idx = channel * (self.width * self.height) + y * self.width + x;
        self.data[idx] = value;
    }

    /// Returns a slice of the pixel data for a specific channel
    #[inline]
    pub fn channel_data(&self, channel: usize) -> &[f32] {
        debug_assert!(channel < self.channels);
        let area = self.width * self.height;
        let start = channel * area;
        &self.data[start..start + area]
    }

    /// Returns a mutable slice of the pixel data for a specific channel
    #[inline]
    pub fn channel_data_mut(&mut self, channel: usize) -> &mut [f32] {
        debug_assert!(channel < self.channels);
        let area = self.width * self.height;
        let start = channel * area;
        &mut self.data[start..start + area]
    }

    /// Returns three distinct mutable slices for the Red, Green, and Blue planes.
    /// This is required to satisfy the borrow checker when parallelizing across planes.
    ///
    /// # Panics
    /// Panics unless `channels() == 3`. A `debug_assert` would be worse than useless
    /// here: in release it would fall through to `split_at_mut` and panic anyway,
    /// with "mid > len" and no hint about the real cause. The check costs nothing next
    /// to the full-plane work every caller does.
    #[inline]
    pub fn planes_mut(&mut self) -> (&mut [f32], &mut [f32], &mut [f32]) {
        assert_eq!(
            self.channels, 3,
            "planes_mut() requires a 3-channel frame, got {}",
            self.channels
        );
        let area = self.width * self.height;
        let (r_plane, rest) = self.data.split_at_mut(area);
        let (g_plane, b_plane) = rest.split_at_mut(area);
        (r_plane, g_plane, b_plane)
    }

    /// Returns three distinct immutable slices for the Red, Green, and Blue planes.
    ///
    /// # Panics
    /// Panics unless `channels() == 3`; see [`Frame::planes_mut`].
    #[inline]
    pub fn planes(&self) -> (&[f32], &[f32], &[f32]) {
        assert_eq!(
            self.channels, 3,
            "planes() requires a 3-channel frame, got {}",
            self.channels
        );
        let area = self.width * self.height;
        let (r_plane, rest) = self.data.split_at(area);
        let (g_plane, b_plane) = rest.split_at(area);
        (r_plane, g_plane, b_plane)
    }

    /// Checks if this frame has the same dimensions as another
    #[inline]
    pub fn dimensions_match(&self, other: &Frame) -> bool {
        self.width == other.width && self.height == other.height && self.channels == other.channels
    }

    /// Area-average downsample by an integer factor, preserving f32 precision.
    /// **No call sites in this crate — used by the Pro plate-solve plugin** to bin a
    /// frame before ASTAP. Once deleted as "dead code", which broke the Pro build; a
    /// dead-code claim about a `pub` item isn't verifiable from this repo alone. Keep
    /// it, or update `night-amplifier-pro` in the same change.
    ///
    /// Distinct from the streaming encoder's box filter (`server::encoding`), which
    /// takes an arbitrary target size and emits `u8` — this takes an integer factor
    /// and stays f32 for solver precision. Trailing pixels are dropped: output is
    /// `width / factor` by `height / factor`.
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

        let src_area = src_width * src_height;

        // One flat dispatch over output rows. The previous nesting put a
        // `par_chunks_mut` inside a `par_chunks_mut`, and for the mono plate-solve case
        // the outer iterator had exactly one item — pure overhead.
        output
            .par_chunks_mut(dst_width)
            .enumerate()
            .for_each(|(row_idx, row_out)| {
                let c = row_idx / dst_height;
                let dst_y = row_idx % dst_height;
                let src_plane = &src_data[c * src_area..(c + 1) * src_area];
                let src_y_start = dst_y * factor;

                for dst_x in 0..dst_width {
                    let src_x_start = dst_x * factor;
                    let mut sum = 0.0f32;
                    for sy in 0..factor {
                        let row_start = (src_y_start + sy) * src_width;
                        for sx in 0..factor {
                            sum += src_plane[row_start + src_x_start + sx];
                        }
                    }
                    row_out[dst_x] = sum * inv_area;
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

    // ------------------------------------------------------------------
    // downsample
    //
    // No call sites in this crate (the Pro plate solver is the only caller),
    // so without these tests `cargo test` here exercises none of the three
    // channel branches. A silent indexing error would corrupt astrometry.
    // ------------------------------------------------------------------

    /// Distinct value per (x, y, channel) so a transposition or a channel mix-up
    /// cannot survive. Deliberately not a simple ramp in `x`: a channel swap on
    /// `c * 0.1` alone would be caught, but a row/column swap would not.
    fn gradient_frame(width: usize, height: usize, channels: usize) -> Frame {
        let mut data = vec![0.0f32; width * height * channels];
        for y in 0..height {
            for x in 0..width {
                for c in 0..channels {
                    let idx = c * (width * height) + y * width + x;
                    data[idx] = (x as f32 * 3.0 + y as f32 * 7.0 + c as f32 * 100.0) / 1000.0;
                }
            }
        }
        Frame::from_f32_vec(data, width, height, channels).unwrap()
    }

    /// Straightforward scalar box average, independent of the parallel/branched
    /// production code.
    fn reference_downsample(frame: &Frame, factor: usize) -> Frame {
        let (dst_w, dst_h) = (frame.width() / factor, frame.height() / factor);
        let mut out = Frame::zeros(dst_w, dst_h, frame.channels()).unwrap();
        for dy in 0..dst_h {
            for dx in 0..dst_w {
                for c in 0..frame.channels() {
                    let mut sum = 0.0f32;
                    for sy in 0..factor {
                        for sx in 0..factor {
                            sum += frame.get_pixel(dx * factor + sx, dy * factor + sy, c);
                        }
                    }
                    out.set_pixel(dx, dy, c, sum / (factor * factor) as f32);
                }
            }
        }
        out
    }

    #[test]
    fn test_downsample_rejects_zero_factor() {
        let frame = Frame::zeros(8, 8, 3).unwrap();
        assert!(frame.downsample(0).is_err());
    }

    #[test]
    fn test_downsample_rejects_factor_larger_than_image() {
        let frame = Frame::zeros(4, 4, 3).unwrap();
        assert!(frame.downsample(8).is_err());
    }

    #[test]
    fn test_downsample_by_one_is_identity() {
        let frame = gradient_frame(6, 5, 3);
        let out = frame.downsample(1).unwrap();
        assert_eq!(out.data(), frame.data());
    }

    /// Covers all three branches (1, 3 and the general fallback) against an
    /// independent reference.
    #[test]
    fn test_downsample_matches_reference_for_every_channel_branch() {
        for channels in [1, 3, 2, 4] {
            for factor in [2, 3, 4] {
                let frame = gradient_frame(24, 18, channels);
                let got = frame.downsample(factor).unwrap();
                let want = reference_downsample(&frame, factor);

                assert_eq!(
                    got.width(),
                    want.width(),
                    "channels={channels} factor={factor}"
                );
                assert_eq!(
                    got.height(),
                    want.height(),
                    "channels={channels} factor={factor}"
                );
                assert_eq!(got.channels(), channels);

                for (i, (&g, &w)) in got.data().iter().zip(want.data()).enumerate() {
                    assert!(
                        (g - w).abs() < 1e-6,
                        "channels={channels} factor={factor} sample {i}: {g} != {w}"
                    );
                }
            }
        }
    }

    /// A uniform frame averages to itself no matter how wrong the weights are,
    /// so pin the area divisor with a frame whose mean is known but not constant.
    #[test]
    fn test_downsample_averages_rather_than_subsamples() {
        // 2x2 block of 0.0, 0.2, 0.4, 0.6 -> mean 0.3
        let data = vec![0.0, 0.2, 0.4, 0.6];
        let frame = Frame::from_f32_vec(data, 2, 2, 1).unwrap();
        let out = frame.downsample(2).unwrap();
        assert_eq!(out.width(), 1);
        assert_eq!(out.height(), 1);
        assert!((out.get_pixel(0, 0, 0) - 0.3).abs() < 1e-6);
    }

    /// Non-multiple dimensions drop the trailing partial block rather than
    /// reading past the row.
    #[test]
    fn test_downsample_truncates_trailing_partial_blocks() {
        let frame = gradient_frame(7, 5, 3);
        let out = frame.downsample(2).unwrap();
        assert_eq!((out.width(), out.height()), (3, 2));

        let want = reference_downsample(&frame, 2);
        for (&g, &w) in out.data().iter().zip(want.data()) {
            assert!((g - w).abs() < 1e-6);
        }
    }

    /// The row-chunked rayon split must not change results.
    #[test]
    fn test_downsample_is_invariant_to_thread_count() {
        let frame = gradient_frame(64, 48, 3);
        let single = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| frame.downsample(3).unwrap());
        let many = rayon::ThreadPoolBuilder::new()
            .num_threads(8)
            .build()
            .unwrap()
            .install(|| frame.downsample(3).unwrap());
        assert_eq!(single.data(), many.data());
    }
}
