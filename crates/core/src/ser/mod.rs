//! SER Video File Format Support: the standard uncompressed, per-frame-timestamped
//! format for high-frame-rate planetary imaging. 178-byte fixed header, raw pixel
//! frames, optional 8-byte UTC timestamps at end of file, 8-bit or 16-bit
//! (little-endian) samples.
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
//! Spec: <http://www.grischa-hahn.homepage.t-online.de/astro/ser/>

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

    /// 16x8 Bayer16 frames whose every sample names its frame, as a capture writes them.
    const W: u32 = 16;
    const H: u32 = 8;
    const FRAME_BYTES: u64 = (W * H * 2) as u64;

    fn raw(frame: u16) -> Vec<u8> {
        (0..W * H).flat_map(|i| (frame * 1000 + i as u16).to_le_bytes()).collect()
    }

    fn recording(path: &std::path::Path, frames: u16) -> SerWriter {
        let mut writer = SerWriter::create(path, SerHeader::new(W, H, SerColorId::BayerRggb, 16)).unwrap();
        for frame in 0..frames {
            writer.write_raw_bytes(&raw(frame), None).unwrap();
        }
        writer
    }

    fn header_count(path: &std::path::Path) -> u32 {
        let bytes = std::fs::read(path).unwrap();
        u32::from_le_bytes(bytes[38..42].try_into().unwrap())
    }

    fn first_sample(reader: &mut SerReader, index: u32) -> u16 {
        let mosaic = reader.read_mosaic(index).unwrap();
        (mosaic.channel_data(0)[0] * 65535.0).round() as u16
    }

    /// 2026-10-24's Moon: 1467 frames behind a header still reading 0, because the capture
    /// never finalized it. The reader goes by the frames the file holds.
    #[test]
    fn a_header_left_at_zero_reads_the_frames_the_file_holds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("unfinalized.ser");
        drop(recording(&path, 3));
        assert_eq!(header_count(&path), 0);

        let mut reader = SerReader::open(&path).unwrap();
        assert_eq!(reader.frame_count(), 3);
        assert_eq!(reader.timestamps(), None);
        assert_eq!(first_sample(&mut reader, 2), 2000);
    }

    #[test]
    fn a_frame_cut_short_by_a_power_loss_is_left_out() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("cut.ser");
        drop(recording(&path, 3));
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut file, &raw(3)[..100]).unwrap();
        assert_eq!(SerReader::open(&path).unwrap().frame_count(), 3);
    }

    /// The writer rewrites the count every 32 frames, so a recording stopped between two
    /// rewrites holds more frames than its header says.
    #[test]
    fn frames_past_the_last_rewritten_count_are_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("refreshed.ser");
        drop(recording(&path, 40));
        assert_eq!(header_count(&path), 32);
        let mut reader = SerReader::open(&path).unwrap();
        assert_eq!(reader.frame_count(), 40);
        assert_eq!(first_sample(&mut reader, 39), 39000);
    }

    #[test]
    fn a_file_shorter_than_its_header_reads_the_frames_that_are_there() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("truncated.ser");
        recording(&path, 5).finalize().unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(178 + 3 * FRAME_BYTES + 10).unwrap();
        assert_eq!(SerReader::open(&path).unwrap().frame_count(), 3);
    }

    #[test]
    fn a_finalized_file_keeps_its_count_and_timestamps() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("whole.ser");
        let mut writer = SerWriter::create(&path, SerHeader::new(W, H, SerColorId::BayerRggb, 16)).unwrap();
        for frame in 0..4u16 {
            writer.write_raw_bytes(&raw(frame), Some(100 + u64::from(frame))).unwrap();
        }
        writer.finalize().unwrap();
        let reader = SerReader::open(&path).unwrap();
        assert_eq!(reader.frame_count(), 4);
        assert_eq!(reader.timestamps(), Some(&[100, 101, 102, 103][..]));
    }

    #[test]
    fn a_file_without_frames_refuses_a_read_instead_of_underflowing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.ser");
        recording(&path, 0).finalize().unwrap();
        let mut reader = SerReader::open(&path).unwrap();
        assert_eq!(reader.frame_count(), 0);
        assert!(reader.read_frame(0).is_err());
        assert!(reader.read_mosaic(0).is_err());
    }

    /// A sync flushes the open container: what it makes durable must be readable by a tool
    /// that trusts the header.
    #[test]
    fn the_header_counts_the_frames_a_flush_pushed_out() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("flushed.ser");
        let mut writer = recording(&path, 33);
        assert_eq!(header_count(&path), 32);
        writer.flush().unwrap();
        assert_eq!(header_count(&path), 33);
        writer.write_raw_bytes(&raw(33), None).unwrap();
        writer.finalize().unwrap();
        assert_eq!(header_count(&path), 34);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 178 + 34 * FRAME_BYTES);
    }

    /// The simulated camera demosaics like a real one, so it reads the mosaic, bit for bit.
    #[test]
    fn a_mosaic_reads_back_as_the_samples_written() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("mosaic.ser");
        recording(&path, 2).finalize().unwrap();
        let mut reader = SerReader::open(&path).unwrap();
        let mosaic = reader.read_mosaic(1).unwrap();
        assert_eq!((mosaic.width(), mosaic.height(), mosaic.channels()), (16, 8, 1));
        // Rounded: `sample_to_u16` truncates, as the writers always have.
        let samples: Vec<u8> = mosaic.channel_data(0).iter().flat_map(|&v| ((v * 65535.0).round() as u16).to_le_bytes()).collect();
        assert_eq!(samples, raw(1));
        assert_eq!(reader.read_frame(1).unwrap().channels(), 3, "read_frame still demosaics");
    }

    #[test]
    fn test_empty_frames_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.ser");

        let result = write_ser(&path, &[], SerColorId::Rgb, 8);
        assert!(result.is_err());
    }
}
