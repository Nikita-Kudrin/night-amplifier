//! SER file writer for saving planetary video captures.

use super::color_id::SerColorId;
use super::header::SerHeader;
use crate::error::{Result, StackError};
use crate::frame::Frame;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// SER file writer for saving planetary video captures.
pub struct SerWriter {
    writer: BufWriter<File>,
    header: SerHeader,
    frames_written: u32,
    timestamps: Vec<u64>,
}

impl SerWriter {
    /// Creates a new SER file for writing.
    pub fn create<P: AsRef<Path>>(path: P, header: SerHeader) -> Result<Self> {
        let file = File::create(path.as_ref()).map_err(|e| {
            StackError::InvalidConfiguration(format!("Failed to create SER file: {}", e))
        })?;
        let mut writer = BufWriter::new(file);

        writer.write_all(&header.to_bytes()).map_err(|e| {
            StackError::InvalidConfiguration(format!("Failed to write SER header: {}", e))
        })?;

        Ok(Self {
            writer,
            header,
            frames_written: 0,
            timestamps: Vec::new(),
        })
    }

    /// Returns the header.
    pub fn header(&self) -> &SerHeader {
        &self.header
    }

    /// Returns the number of frames written so far.
    pub fn frames_written(&self) -> u32 {
        self.frames_written
    }

    /// Writes a frame to the file.
    pub fn write_frame(&mut self, frame: &Frame, timestamp: Option<u64>) -> Result<()> {
        if frame.width() != self.header.width as usize
            || frame.height() != self.header.height as usize
        {
            return Err(StackError::CalibrationDimensionMismatch {
                frame_width: frame.width(),
                frame_height: frame.height(),
                cal_width: self.header.width as usize,
                cal_height: self.header.height as usize,
            });
        }

        tracing::debug!(
            frame_number = self.frames_written + 1,
            width = frame.width(),
            height = frame.height(),
            "Encoding and writing SER frame"
        );
        let bytes = encode_frame(frame, &self.header);

        self.writer.write_all(&bytes).map_err(|e| {
            StackError::InvalidConfiguration(format!("Failed to write frame: {}", e))
        })?;

        self.frames_written += 1;
        self.record_timestamp(timestamp);

        Ok(())
    }

    /// Writes raw bytes directly (for passthrough from capture).
    pub fn write_raw_bytes(&mut self, bytes: &[u8], timestamp: Option<u64>) -> Result<()> {
        if bytes.len() != self.header.frame_size() {
            return Err(StackError::BufferSizeMismatch {
                expected: self.header.frame_size(),
                actual: bytes.len(),
            });
        }

        self.writer.write_all(bytes).map_err(|e| {
            StackError::InvalidConfiguration(format!("Failed to write frame: {}", e))
        })?;

        self.frames_written += 1;
        self.record_timestamp(timestamp);

        Ok(())
    }

    fn record_timestamp(&mut self, timestamp: Option<u64>) {
        if let Some(ts) = timestamp {
            self.timestamps.push(ts);
        } else if !self.timestamps.is_empty() {
            self.timestamps.push(0);
        }
    }

    /// Finalizes the file, writing timestamps and updating header.
    pub fn finalize(mut self) -> Result<()> {
        tracing::info!(frames_written = self.frames_written, "Finalizing SER file");
        self.writer
            .flush()
            .map_err(|e| StackError::InvalidConfiguration(format!("Failed to flush: {}", e)))?;

        if !self.timestamps.is_empty() {
            for ts in &self.timestamps {
                self.writer.write_all(&ts.to_le_bytes()).map_err(|e| {
                    StackError::InvalidConfiguration(format!("Write failed: {}", e))
                })?;
            }
        }

        self.header.frame_count = self.frames_written;

        self.writer
            .seek(SeekFrom::Start(0))
            .map_err(|e| StackError::InvalidConfiguration(format!("Seek failed: {}", e)))?;

        self.writer
            .write_all(&self.header.to_bytes())
            .map_err(|e| StackError::InvalidConfiguration(format!("Write failed: {}", e)))?;

        self.writer
            .flush()
            .map_err(|e| StackError::InvalidConfiguration(format!("Flush failed: {}", e)))?;

        Ok(())
    }
}

fn encode_frame(frame: &Frame, header: &SerHeader) -> Vec<u8> {
    if header.bit_depth <= 8 {
        encode_8bit(frame, header)
    } else {
        encode_16bit(frame, header)
    }
}

/// Borrows a frame's three colour planes, replicating the single plane for mono.
///
/// SER payloads are interleaved (RGB/BGR) or single-channel, while `Frame` is
/// planar — so every arm below gathers across planes rather than walking the buffer
/// linearly. Replicating mono into all three slots also makes the Rec. 709
/// luminance sum collapse to the sample itself (the weights total 1.0), which lets
/// mono and colour sources share one code path.
fn planes_of(frame: &Frame) -> (&[f32], &[f32], &[f32]) {
    let area = frame.width() * frame.height();
    let data = frame.data();
    if frame.channels() >= 3 {
        (&data[..area], &data[area..2 * area], &data[2 * area..3 * area])
    } else {
        (&data[..area], &data[..area], &data[..area])
    }
}

#[inline]
fn luminance(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn encode_8bit(frame: &Frame, header: &SerHeader) -> Vec<u8> {
    let (rp, gp, bp) = planes_of(frame);
    let dst_channels = header.color_id.channels();
    let pixels = header.width as usize * header.height as usize;

    let mut bytes = Vec::with_capacity(pixels * dst_channels);
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;

    match header.color_id {
        SerColorId::Mono => {
            for i in 0..pixels {
                bytes.push(to_u8(luminance(rp[i], gp[i], bp[i])));
            }
        }
        SerColorId::Rgb => {
            for i in 0..pixels {
                bytes.push(to_u8(rp[i]));
                bytes.push(to_u8(gp[i]));
                bytes.push(to_u8(bp[i]));
            }
        }
        SerColorId::Bgr => {
            for i in 0..pixels {
                bytes.push(to_u8(bp[i]));
                bytes.push(to_u8(gp[i]));
                bytes.push(to_u8(rp[i]));
            }
        }
        _ => {
            // Bayer or unknown: emit the green channel as greyscale.
            for i in 0..pixels {
                bytes.push(to_u8(gp[i]));
            }
        }
    }

    bytes
}

fn encode_16bit(frame: &Frame, header: &SerHeader) -> Vec<u8> {
    let (rp, gp, bp) = planes_of(frame);
    let dst_channels = header.color_id.channels();
    let pixels = header.width as usize * header.height as usize;

    let mut bytes = Vec::with_capacity(pixels * dst_channels * 2);
    let to_u16 = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;

    match header.color_id {
        SerColorId::Mono => {
            for i in 0..pixels {
                bytes.extend_from_slice(&to_u16(luminance(rp[i], gp[i], bp[i])).to_le_bytes());
            }
        }
        SerColorId::Rgb => {
            for i in 0..pixels {
                bytes.extend_from_slice(&to_u16(rp[i]).to_le_bytes());
                bytes.extend_from_slice(&to_u16(gp[i]).to_le_bytes());
                bytes.extend_from_slice(&to_u16(bp[i]).to_le_bytes());
            }
        }
        SerColorId::Bgr => {
            for i in 0..pixels {
                bytes.extend_from_slice(&to_u16(bp[i]).to_le_bytes());
                bytes.extend_from_slice(&to_u16(gp[i]).to_le_bytes());
                bytes.extend_from_slice(&to_u16(rp[i]).to_le_bytes());
            }
        }
        _ => {
            for i in 0..pixels {
                bytes.extend_from_slice(&to_u16(gp[i]).to_le_bytes());
            }
        }
    }

    bytes
}

