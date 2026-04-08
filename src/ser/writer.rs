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
        tracing::info!(
            frames_written = self.frames_written,
            "Finalizing SER file"
        );
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

fn encode_8bit(frame: &Frame, header: &SerHeader) -> Vec<u8> {
    let data = frame.data();
    let src_channels = frame.channels();
    let dst_channels = header.color_id.channels();
    let pixels = header.width as usize * header.height as usize;

    let mut bytes = Vec::with_capacity(pixels * dst_channels);

    match header.color_id {
        SerColorId::Mono => {
            if src_channels >= 3 {
                for i in 0..pixels {
                    let r = data[i * 3];
                    let g = data[i * 3 + 1];
                    let b = data[i * 3 + 2];
                    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    bytes.push((lum.clamp(0.0, 1.0) * 255.0) as u8);
                }
            } else {
                for i in 0..pixels {
                    bytes.push((data[i].clamp(0.0, 1.0) * 255.0) as u8);
                }
            }
        }
        SerColorId::Rgb => {
            for &v in data {
                bytes.push((v.clamp(0.0, 1.0) * 255.0) as u8);
            }
        }
        SerColorId::Bgr => {
            if src_channels >= 3 {
                for i in 0..pixels {
                    bytes.push((data[i * 3 + 2].clamp(0.0, 1.0) * 255.0) as u8);
                    bytes.push((data[i * 3 + 1].clamp(0.0, 1.0) * 255.0) as u8);
                    bytes.push((data[i * 3].clamp(0.0, 1.0) * 255.0) as u8);
                }
            } else {
                for i in 0..pixels {
                    let v = (data[i].clamp(0.0, 1.0) * 255.0) as u8;
                    bytes.push(v);
                    bytes.push(v);
                    bytes.push(v);
                }
            }
        }
        _ => {
            // For Bayer or unknown, try to output as grayscale
            if src_channels >= 3 {
                for i in 0..pixels {
                    bytes.push((data[i * 3 + 1].clamp(0.0, 1.0) * 255.0) as u8);
                }
            } else {
                for i in 0..pixels {
                    bytes.push((data[i].clamp(0.0, 1.0) * 255.0) as u8);
                }
            }
        }
    }

    bytes
}

fn encode_16bit(frame: &Frame, header: &SerHeader) -> Vec<u8> {
    let data = frame.data();
    let src_channels = frame.channels();
    let dst_channels = header.color_id.channels();
    let pixels = header.width as usize * header.height as usize;

    let mut bytes = Vec::with_capacity(pixels * dst_channels * 2);

    match header.color_id {
        SerColorId::Mono => {
            if src_channels >= 3 {
                for i in 0..pixels {
                    let r = data[i * 3];
                    let g = data[i * 3 + 1];
                    let b = data[i * 3 + 2];
                    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    let value = (lum.clamp(0.0, 1.0) * 65535.0) as u16;
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            } else {
                for i in 0..pixels {
                    let value = (data[i].clamp(0.0, 1.0) * 65535.0) as u16;
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        SerColorId::Rgb => {
            for &v in data {
                let value = (v.clamp(0.0, 1.0) * 65535.0) as u16;
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        SerColorId::Bgr => {
            if src_channels >= 3 {
                for i in 0..pixels {
                    let b = (data[i * 3 + 2].clamp(0.0, 1.0) * 65535.0) as u16;
                    let g = (data[i * 3 + 1].clamp(0.0, 1.0) * 65535.0) as u16;
                    let r = (data[i * 3].clamp(0.0, 1.0) * 65535.0) as u16;
                    bytes.extend_from_slice(&b.to_le_bytes());
                    bytes.extend_from_slice(&g.to_le_bytes());
                    bytes.extend_from_slice(&r.to_le_bytes());
                }
            } else {
                for i in 0..pixels {
                    let v = (data[i].clamp(0.0, 1.0) * 65535.0) as u16;
                    bytes.extend_from_slice(&v.to_le_bytes());
                    bytes.extend_from_slice(&v.to_le_bytes());
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        _ => {
            if src_channels >= 3 {
                for i in 0..pixels {
                    let value = (data[i * 3 + 1].clamp(0.0, 1.0) * 65535.0) as u16;
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            } else {
                for i in 0..pixels {
                    let value = (data[i].clamp(0.0, 1.0) * 65535.0) as u16;
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
    }

    bytes
}
