//! Master Dark frame for thermal noise subtraction

use crate::error::Result;
use crate::frame::{Frame, PixelFormat};

/// Master Dark frame for thermal noise subtraction
///
/// Created by averaging multiple dark frames taken with the lens cap on
/// at the same exposure time and temperature as the light frames.
#[derive(Debug, Clone)]
pub struct MasterDark {
    frame: Frame,
}

impl MasterDark {
    /// Creates a new MasterDark from a Frame
    ///
    /// The frame should already be a properly averaged dark frame.
    pub fn new(frame: Frame) -> Self {
        Self { frame }
    }

    /// Creates a MasterDark from raw image data
    pub fn from_raw(
        raw: &[u8],
        width: usize,
        height: usize,
        channels: usize,
        format: PixelFormat,
    ) -> Result<Self> {
        let frame = Frame::from_raw(raw, width, height, channels, format)?;
        Ok(Self::new(frame))
    }

    /// Returns the underlying frame
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Returns image dimensions (width, height, channels)
    pub fn dimensions(&self) -> (usize, usize, usize) {
        (
            self.frame.width(),
            self.frame.height(),
            self.frame.channels(),
        )
    }
}
