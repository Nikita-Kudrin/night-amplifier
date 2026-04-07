//! SER file reader for loading planetary video captures.

use super::color_id::SerColorId;
use super::header::SerHeader;
use crate::error::{Result, StackError};
use crate::frame::{Frame, PixelFormat};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Header size in bytes.
const HEADER_SIZE: u64 = 178;

/// SER file reader for loading planetary video captures.
pub struct SerReader {
    reader: BufReader<File>,
    header: SerHeader,
    timestamps: Option<Vec<u64>>,
}

impl SerReader {
    /// Opens a SER file for reading.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref()).map_err(|e| {
            StackError::InvalidConfiguration(format!("Failed to open SER file: {}", e))
        })?;
        let mut reader = BufReader::new(file);

        let mut header_bytes = [0u8; 178];
        reader.read_exact(&mut header_bytes).map_err(|e| {
            StackError::InvalidConfiguration(format!("Failed to read SER header: {}", e))
        })?;

        let header = SerHeader::from_bytes(&header_bytes)?;
        let timestamps = read_timestamps(&mut reader, &header)?;

        Ok(Self {
            reader,
            header,
            timestamps,
        })
    }

    /// Returns the file header.
    pub fn header(&self) -> &SerHeader {
        &self.header
    }

    /// Returns the number of frames in the file.
    pub fn frame_count(&self) -> u32 {
        self.header.frame_count
    }

    /// Returns the frame dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.header.width, self.header.height)
    }

    /// Returns per-frame timestamps if available.
    pub fn timestamps(&self) -> Option<&[u64]> {
        self.timestamps.as_deref()
    }

    /// Reads a specific frame by index.
    pub fn read_frame(&mut self, index: u32) -> Result<Frame> {
        if index >= self.header.frame_count {
            return Err(StackError::InvalidConfiguration(format!(
                "Frame index {} out of range (max {})",
                index,
                self.header.frame_count - 1
            )));
        }

        let frame_offset = HEADER_SIZE + (index as u64 * self.header.frame_size() as u64);
        self.reader
            .seek(SeekFrom::Start(frame_offset))
            .map_err(|e| StackError::InvalidConfiguration(format!("Seek failed: {}", e)))?;

        self.read_next_frame()
    }

    /// Reads the next frame sequentially.
    pub fn read_next_frame(&mut self) -> Result<Frame> {
        let frame_size = self.header.frame_size();
        let mut buffer = vec![0u8; frame_size];

        self.reader.read_exact(&mut buffer).map_err(|e| {
            StackError::InvalidConfiguration(format!("Failed to read frame data: {}", e))
        })?;

        decode_frame(&buffer, &self.header)
    }

    /// Creates an iterator over all frames.
    pub fn frames(&mut self) -> SerFrameIterator<'_> {
        let _ = self.reader.seek(SeekFrom::Start(HEADER_SIZE));
        SerFrameIterator {
            reader: self,
            current: 0,
        }
    }
}

/// Iterator over frames in a SER file.
pub struct SerFrameIterator<'a> {
    reader: &'a mut SerReader,
    current: u32,
}

impl<'a> Iterator for SerFrameIterator<'a> {
    type Item = Result<Frame>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.reader.header.frame_count {
            return None;
        }
        self.current += 1;
        Some(self.reader.read_next_frame())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.reader.header.frame_count - self.current) as usize;
        (remaining, Some(remaining))
    }
}

fn read_timestamps(reader: &mut BufReader<File>, header: &SerHeader) -> Result<Option<Vec<u64>>> {
    let frame_data_size = HEADER_SIZE as usize + header.frame_size() * header.frame_count as usize;
    let timestamp_size = header.frame_count as usize * 8;

    let file_size = reader
        .seek(SeekFrom::End(0))
        .map_err(|e| StackError::InvalidConfiguration(format!("Seek failed: {}", e)))?
        as usize;

    if file_size >= frame_data_size + timestamp_size {
        reader
            .seek(SeekFrom::Start(frame_data_size as u64))
            .map_err(|e| {
                StackError::InvalidConfiguration(format!("Seek to timestamps failed: {}", e))
            })?;

        let mut timestamps = Vec::with_capacity(header.frame_count as usize);
        for _ in 0..header.frame_count {
            let mut ts_bytes = [0u8; 8];
            reader.read_exact(&mut ts_bytes).map_err(|e| {
                StackError::InvalidConfiguration(format!("Failed to read timestamp: {}", e))
            })?;
            timestamps.push(u64::from_le_bytes(ts_bytes));
        }

        reader
            .seek(SeekFrom::Start(HEADER_SIZE))
            .map_err(|e| StackError::InvalidConfiguration(format!("Seek failed: {}", e)))?;

        Ok(Some(timestamps))
    } else {
        reader
            .seek(SeekFrom::Start(HEADER_SIZE))
            .map_err(|e| StackError::InvalidConfiguration(format!("Seek failed: {}", e)))?;
        Ok(None)
    }
}

fn decode_frame(buffer: &[u8], header: &SerHeader) -> Result<Frame> {
    let width = header.width as usize;
    let height = header.height as usize;

    if header.color_id == SerColorId::Mono {
        return create_mono_frame(buffer, width, height, header.bit_depth);
    }

    if header.color_id == SerColorId::Bgr {
        return create_bgr_frame(buffer, width, height, header.bit_depth);
    }

    if header.color_id.is_bayer() {
        let pixel_format = determine_bayer_format(header);
        let pattern = header.color_id.to_cfa_pattern().unwrap();
        return Frame::from_bayer(buffer, width, height, pixel_format, pattern);
    }

    let pixel_format = determine_rgb_format(header);
    Frame::from_raw(buffer, width, height, 3, pixel_format)
}

fn determine_bayer_format(header: &SerHeader) -> PixelFormat {
    if header.bit_depth <= 8 {
        PixelFormat::Bayer8
    } else if header.little_endian {
        PixelFormat::Bayer16
    } else {
        PixelFormat::Bayer16Be
    }
}

fn determine_rgb_format(header: &SerHeader) -> PixelFormat {
    if header.bit_depth <= 8 {
        PixelFormat::Rgb8
    } else if header.little_endian {
        PixelFormat::Rgb16
    } else {
        PixelFormat::Rgb16Be
    }
}

fn create_mono_frame(buffer: &[u8], width: usize, height: usize, bit_depth: u32) -> Result<Frame> {
    let pixels = width * height;
    let mut data = vec![0.0f32; pixels * 3];

    if bit_depth <= 8 {
        for i in 0..pixels {
            let value = buffer[i] as f32 / 255.0;
            data[i * 3] = value;
            data[i * 3 + 1] = value;
            data[i * 3 + 2] = value;
        }
    } else {
        for i in 0..pixels {
            let value = u16::from_le_bytes([buffer[i * 2], buffer[i * 2 + 1]]) as f32 / 65535.0;
            data[i * 3] = value;
            data[i * 3 + 1] = value;
            data[i * 3 + 2] = value;
        }
    }

    Frame::from_f32_vec(data, width, height, 3)
}

fn create_bgr_frame(buffer: &[u8], width: usize, height: usize, bit_depth: u32) -> Result<Frame> {
    let pixels = width * height;
    let mut data = vec![0.0f32; pixels * 3];

    if bit_depth <= 8 {
        for i in 0..pixels {
            data[i * 3] = buffer[i * 3 + 2] as f32 / 255.0;
            data[i * 3 + 1] = buffer[i * 3 + 1] as f32 / 255.0;
            data[i * 3 + 2] = buffer[i * 3] as f32 / 255.0;
        }
    } else {
        for i in 0..pixels {
            let b = u16::from_le_bytes([buffer[i * 6], buffer[i * 6 + 1]]) as f32 / 65535.0;
            let g = u16::from_le_bytes([buffer[i * 6 + 2], buffer[i * 6 + 3]]) as f32 / 65535.0;
            let r = u16::from_le_bytes([buffer[i * 6 + 4], buffer[i * 6 + 5]]) as f32 / 65535.0;
            data[i * 3] = r;
            data[i * 3 + 1] = g;
            data[i * 3 + 2] = b;
        }
    }

    Frame::from_f32_vec(data, width, height, 3)
}
