//! SER Video File Format Support
//!
//! SER (Simple Extensible Recording) is the standard format for planetary imaging.
//! It stores uncompressed video frames with per-frame timestamps, making it ideal
//! for high-frame-rate planetary captures.
//!
//! # Format Specification
//!
//! - **Header**: 178 bytes (fixed)
//! - **Frames**: Raw pixel data (no compression)
//! - **Timestamps**: Optional 8-byte UTC timestamps per frame (at end of file)
//!
//! # Supported Color Modes
//!
//! | ID | Format | Description |
//! |----|--------|-------------|
//! | 0  | MONO   | Grayscale |
//! | 8  | BAYER_RGGB | Raw Bayer RGGB |
//! | 9  | BAYER_GRBG | Raw Bayer GRBG |
//! | 10 | BAYER_GBRG | Raw Bayer GBRG |
//! | 11 | BAYER_BGGR | Raw Bayer BGGR |
//! | 100| RGB    | RGB color (3 channels) |
//! | 101| BGR    | BGR color (3 channels) |
//!
//! # Bit Depth
//!
//! Supports 8-bit and 16-bit (little-endian) pixel data.
//!
//! # References
//!
//! - SER Format Specification: <http://www.grischa-hahn.homepage.t-online.de/astro/ser/>

mod color_id;
mod header;
mod reader;
mod writer;

pub use color_id::SerColorId;
pub use header::SerHeader;
pub use reader::{SerFrameIterator, SerReader};
pub use writer::SerWriter;

use crate::error::{Result, StackError};
use crate::frame::Frame;
use std::path::Path;

/// SER file signature (14 bytes).
const SER_SIGNATURE: &[u8; 14] = b"LUCAM-RECORDER";

/// Convenience function to write a sequence of frames to a SER file.
pub fn write_ser<P: AsRef<Path>>(
    path: P,
    frames: &[Frame],
    color_id: SerColorId,
    bit_depth: u32,
) -> Result<()> {
    if frames.is_empty() {
        return Err(StackError::InvalidConfiguration(
            "No frames to write".to_string(),
        ));
    }

    let first = &frames[0];
    let header = SerHeader::new(
        first.width() as u32,
        first.height() as u32,
        color_id,
        bit_depth,
    );

    let mut writer = SerWriter::create(path, header)?;

    for frame in frames {
        writer.write_frame(frame, None)?;
    }

    writer.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_ser_write_and_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.ser");

        let frame1 = Frame::filled(64, 64, 3, 0.3).unwrap();
        let frame2 = Frame::filled(64, 64, 3, 0.6).unwrap();

        write_ser(&path, &[frame1, frame2], SerColorId::Rgb, 8).unwrap();

        let mut reader = SerReader::open(&path).unwrap();
        assert_eq!(reader.frame_count(), 2);
        assert_eq!(reader.dimensions(), (64, 64));

        let read_frame1 = reader.read_frame(0).unwrap();
        let read_frame2 = reader.read_frame(1).unwrap();

        assert_eq!(read_frame1.width(), 64);
        assert_eq!(read_frame1.height(), 64);
        assert_eq!(read_frame2.width(), 64);
        assert_eq!(read_frame2.height(), 64);

        let data1 = read_frame1.data();
        assert!((data1[0] - 0.3).abs() < 0.01);

        let data2 = read_frame2.data();
        assert!((data2[0] - 0.6).abs() < 0.01);
    }

    #[test]
    fn test_ser_16bit_precision() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test16.ser");

        let frame = Frame::filled(32, 32, 3, 0.123456).unwrap();
        write_ser(&path, &[frame], SerColorId::Rgb, 16).unwrap();

        let mut reader = SerReader::open(&path).unwrap();
        let read_frame = reader.read_frame(0).unwrap();

        let data = read_frame.data();
        assert!(
            (data[0] - 0.123456).abs() < 0.001,
            "Expected ~0.123456, got {}",
            data[0]
        );
    }

    #[test]
    fn test_empty_frames_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.ser");

        let result = write_ser(&path, &[], SerColorId::Rgb, 8);
        assert!(result.is_err());
    }
}
