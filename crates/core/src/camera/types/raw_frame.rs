//! Frame buffers: the reusable byte pool a camera captures into, and the raw frame it hands
//! back before any Bayer or format conversion.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

use super::{CameraInfo, ImageFormat, SensorType};
use crate::camera::error::{CameraError, CameraResult};
use crate::CfaPattern;

/// A pool of reusable byte buffers for zero-allocation camera capture
#[derive(Debug, Clone)]
pub struct BufferPool {
    pool: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferPool {
    /// Create a new buffer pool
    pub fn new() -> Self {
        Self {
            pool: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get a buffer of at least `size` bytes. Memory is uninitialized/reused for performance.
    pub fn get(&self, size: usize) -> PooledBuffer {
        let mut buf = if let Ok(mut pool) = self.pool.lock() {
            pool.pop().unwrap_or_else(|| Vec::with_capacity(size))
        } else {
            Vec::with_capacity(size)
        };

        buf.clear();
        buf.resize(size, 0);

        PooledBuffer {
            data: Some(buf),
            pool: self.pool.clone(),
        }
    }
}

/// A buffer that automatically returns to its pool when dropped
pub struct PooledBuffer {
    data: Option<Vec<u8>>,
    pool: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl std::fmt::Debug for PooledBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.data.as_ref().map_or(0, |v| v.len());
        f.debug_struct("PooledBuffer").field("len", &len).finish()
    }
}

impl Clone for PooledBuffer {
    fn clone(&self) -> Self {
        let buf = self.data.as_ref().unwrap().clone();
        PooledBuffer {
            data: Some(buf),
            pool: self.pool.clone(),
        }
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let Some(buf) = self.data.take() {
            if let Ok(mut pool) = self.pool.lock() {
                if pool.len() < 5 {
                    // Max 5 buffers in the pool
                    pool.push(buf);
                }
            }
        }
    }
}

impl Deref for PooledBuffer {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.data.as_ref().unwrap()
    }
}

impl DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data.as_mut().unwrap()
    }
}

impl From<Vec<u8>> for PooledBuffer {
    fn from(vec: Vec<u8>) -> Self {
        let pool = Arc::new(Mutex::new(Vec::new()));
        PooledBuffer {
            data: Some(vec),
            pool,
        }
    }
}

/// Raw image data returned by a camera before Bayer or format conversion.
#[derive(Clone)]
pub struct RawFrame {
    /// The raw byte buffer straight from the camera SDK.
    pub data: PooledBuffer,
    /// The width of the captured image in pixels.
    pub width: u32,
    /// The height of the captured image in pixels.
    pub height: u32,
    /// The image format (Raw8, Raw16, Rgb24).
    pub format: ImageFormat,
}

impl std::fmt::Debug for RawFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl RawFrame {
    /// Returns a slice exactly matching the image dimensions and format,
    /// ignoring any pooled buffer padding.
    #[inline]
    pub fn data_slice(&self) -> &[u8] {
        let len = (self.width as usize) * (self.height as usize) * self.format.bytes_per_pixel();
        &self.data[..len]
    }

    /// Decodes the raw buffer into a frame that still carries its CFA mosaic.
    ///
    /// This is the seam the raw-CFA stage opens: hot-pixel rejection, row/column
    /// FPN removal and dark/flat calibration are only defined on the mosaic, and
    /// debayering here — as every provider used to — left nowhere to put them.
    /// The demosaic now happens in the stacking task, after
    /// [`CfaPipeline`](crate::cfa::CfaPipeline) has run.
    pub fn to_cfa_frame(&self, info: &CameraInfo) -> CameraResult<crate::cfa::CfaFrame> {
        use crate::PixelFormat;

        let channels = if info.sensor_type == SensorType::Color && self.format == ImageFormat::Rgb24
        {
            3
        } else {
            1
        };
        let pixel_format = match self.format {
            ImageFormat::Raw8 => {
                if channels == 1 && info.sensor_type == SensorType::Color {
                    PixelFormat::Bayer8
                } else {
                    PixelFormat::Rgb8
                }
            }
            ImageFormat::Raw16 => {
                if channels == 1 && info.sensor_type == SensorType::Color {
                    PixelFormat::Bayer16
                } else {
                    PixelFormat::Rgb16
                }
            }
            ImageFormat::Rgb24 => PixelFormat::Rgb8,
        };

        let frame = crate::Frame::from_raw(
            self.data_slice(),
            self.width as usize,
            self.height as usize,
            channels,
            pixel_format,
        )
        .map_err(|e| CameraError::ImageReadFailed(e.to_string()))?;

        if info.sensor_type != SensorType::Color || channels != 1 {
            return Ok(crate::cfa::CfaFrame::direct(frame));
        }

        // A colour sensor that reported no pattern still has one; RGGB is what
        // this path has always assumed, and changing that here would silently
        // re-colour every simulator fixture.
        let pattern = info.bayer_pattern.unwrap_or(CfaPattern::Rggb);
        crate::cfa::CfaFrame::mosaic(frame, pattern)
            .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
    }

    /// Converts the raw buffer into a debayered `Frame`.
    ///
    /// Equivalent to [`Self::to_cfa_frame`] followed by a bilinear demosaic and
    /// no pre-debayer stages — the behaviour every caller had before the raw
    /// stage existed. The capture pipeline uses the two halves separately so it
    /// can run corrections in between.
    pub fn to_frame(&self, info: &CameraInfo) -> CameraResult<crate::Frame> {
        self.to_cfa_frame(info)?
            .debayer(crate::debayer::DebayerAlgorithm::Bilinear)
            .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
    }
}
