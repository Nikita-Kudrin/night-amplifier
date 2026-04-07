//! SER file header (178 bytes).

use super::color_id::SerColorId;
use super::SER_SIGNATURE;
use crate::error::{Result, StackError};

/// SER file header (178 bytes).
#[derive(Debug, Clone)]
pub struct SerHeader {
    /// File signature (always "LUCAM-RECORDER")
    pub signature: [u8; 14],
    /// Camera serial number (unused, set to 0)
    pub camera_serial: u32,
    /// Color format ID
    pub color_id: SerColorId,
    /// Little-endian pixel data (0 = big-endian, 1 = little-endian for 16-bit)
    pub little_endian: bool,
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// Bit depth per channel (8 or 16)
    pub bit_depth: u32,
    /// Total number of frames
    pub frame_count: u32,
    /// Observer name (40 bytes, null-padded)
    pub observer: [u8; 40],
    /// Instrument/camera name (40 bytes, null-padded)
    pub instrument: [u8; 40],
    /// Telescope name (40 bytes, null-padded)
    pub telescope: [u8; 40],
    /// Date/time of recording start (Windows FILETIME, 100ns intervals since 1601-01-01)
    pub date_time: u64,
    /// Date/time in UTC (Windows FILETIME)
    pub date_time_utc: u64,
}

impl Default for SerHeader {
    fn default() -> Self {
        Self {
            signature: *SER_SIGNATURE,
            camera_serial: 0,
            color_id: SerColorId::Mono,
            little_endian: true,
            width: 0,
            height: 0,
            bit_depth: 8,
            frame_count: 0,
            observer: [0u8; 40],
            instrument: [0u8; 40],
            telescope: [0u8; 40],
            date_time: 0,
            date_time_utc: 0,
        }
    }
}

impl SerHeader {
    /// Creates a new SER header with the given parameters.
    pub fn new(width: u32, height: u32, color_id: SerColorId, bit_depth: u32) -> Self {
        Self {
            width,
            height,
            color_id,
            bit_depth,
            little_endian: true,
            ..Default::default()
        }
    }

    /// Sets the observer name.
    pub fn with_observer(mut self, name: &str) -> Self {
        copy_name_to_buffer(name, &mut self.observer);
        self
    }

    /// Sets the instrument name.
    pub fn with_instrument(mut self, name: &str) -> Self {
        copy_name_to_buffer(name, &mut self.instrument);
        self
    }

    /// Sets the telescope name.
    pub fn with_telescope(mut self, name: &str) -> Self {
        copy_name_to_buffer(name, &mut self.telescope);
        self
    }

    /// Returns the size of a single frame in bytes.
    pub fn frame_size(&self) -> usize {
        let pixels = self.width as usize * self.height as usize * self.color_id.channels();
        let bytes_per_pixel = if self.bit_depth <= 8 { 1 } else { 2 };
        pixels * bytes_per_pixel
    }

    /// Reads a header from a byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 178 {
            return Err(StackError::InvalidConfiguration(
                "SER header too short".to_string(),
            ));
        }

        if &bytes[0..14] != SER_SIGNATURE {
            return Err(StackError::InvalidConfiguration(
                "Invalid SER signature".to_string(),
            ));
        }

        let camera_serial = read_u32_le(bytes, 14);
        let color_id_raw = read_u32_le(bytes, 18);
        let little_endian = read_u32_le(bytes, 22) != 0;
        let width = read_u32_le(bytes, 26);
        let height = read_u32_le(bytes, 30);
        let bit_depth = read_u32_le(bytes, 34);
        let frame_count = read_u32_le(bytes, 38);

        let color_id = SerColorId::from_u32(color_id_raw).ok_or_else(|| {
            StackError::InvalidConfiguration(format!("Unknown SER color ID: {}", color_id_raw))
        })?;

        let mut observer = [0u8; 40];
        observer.copy_from_slice(&bytes[42..82]);

        let mut instrument = [0u8; 40];
        instrument.copy_from_slice(&bytes[82..122]);

        let mut telescope = [0u8; 40];
        telescope.copy_from_slice(&bytes[122..162]);

        let date_time = read_u64_le(bytes, 162);
        let date_time_utc = read_u64_le(bytes, 170);

        Ok(Self {
            signature: *SER_SIGNATURE,
            camera_serial,
            color_id,
            little_endian,
            width,
            height,
            bit_depth,
            frame_count,
            observer,
            instrument,
            telescope,
            date_time,
            date_time_utc,
        })
    }

    /// Writes the header to a byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(178);

        bytes.extend_from_slice(&self.signature);
        bytes.extend_from_slice(&self.camera_serial.to_le_bytes());
        bytes.extend_from_slice(&(self.color_id as u32).to_le_bytes());
        bytes.extend_from_slice(&(u32::from(self.little_endian)).to_le_bytes());
        bytes.extend_from_slice(&self.width.to_le_bytes());
        bytes.extend_from_slice(&self.height.to_le_bytes());
        bytes.extend_from_slice(&self.bit_depth.to_le_bytes());
        bytes.extend_from_slice(&self.frame_count.to_le_bytes());
        bytes.extend_from_slice(&self.observer);
        bytes.extend_from_slice(&self.instrument);
        bytes.extend_from_slice(&self.telescope);
        bytes.extend_from_slice(&self.date_time.to_le_bytes());
        bytes.extend_from_slice(&self.date_time_utc.to_le_bytes());

        bytes
    }
}

fn copy_name_to_buffer(name: &str, buffer: &mut [u8; 40]) {
    let bytes = name.as_bytes();
    let len = bytes.len().min(40);
    buffer[..len].copy_from_slice(&bytes[..len]);
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ser_header_roundtrip() {
        let header = SerHeader::new(640, 480, SerColorId::Mono, 8)
            .with_observer("Test Observer")
            .with_telescope("Test Scope");

        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 178);

        let parsed = SerHeader::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.width, 640);
        assert_eq!(parsed.height, 480);
        assert_eq!(parsed.color_id, SerColorId::Mono);
        assert_eq!(parsed.bit_depth, 8);
    }

    #[test]
    fn test_frame_size_calculation() {
        let header = SerHeader::new(640, 480, SerColorId::Rgb, 16);
        assert_eq!(header.frame_size(), 640 * 480 * 3 * 2);

        let mono_8bit = SerHeader::new(1920, 1080, SerColorId::Mono, 8);
        assert_eq!(mono_8bit.frame_size(), 1920 * 1080);
    }

    #[test]
    fn test_header_too_short() {
        let bytes = [0u8; 100];
        assert!(SerHeader::from_bytes(&bytes).is_err());
    }

    #[test]
    fn test_invalid_signature() {
        let mut bytes = [0u8; 178];
        bytes[0..14].copy_from_slice(b"INVALID-SIGNAT");
        assert!(SerHeader::from_bytes(&bytes).is_err());
    }
}
