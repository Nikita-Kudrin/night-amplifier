//! In-Memory FITS Decoder

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use std::collections::HashMap;

use crate::frame::Frame;
use crate::indi::error::{IndiError, Result};

pub struct FitsDecoder;

impl FitsDecoder {
    /// Decodes a base64 INDI BLOB string into a raw binary buffer using a pre-allocated Vec.
    pub fn decode_base64_blob(base64_str: &str, out_buffer: &mut Vec<u8>) -> Result<()> {
        let expected_len = (base64_str.len() / 4) * 3;
        out_buffer.clear();
        out_buffer.reserve(expected_len);
        BASE64_STANDARD
            .decode_vec(base64_str, out_buffer)
            .map_err(|e| IndiError::DeviceNotFound(format!("Base64 decode error: {}", e)))?;
        Ok(())
    }

    /// Parses a raw FITS buffer (in memory) and returns a Frame.
    pub fn parse_fits_buffer(buffer: &[u8]) -> Result<Frame> {
        let mut header_map = HashMap::new();
        let mut data_start = 0;

        // Parse FITS headers (blocks of 2880 bytes, 80-byte records)
        let mut offset = 0;
        let mut end_found = false;

        while offset + 2880 <= buffer.len() {
            let block = &buffer[offset..offset + 2880];
            
            for i in 0..36 {
                let record_start = i * 80;
                let record = &block[record_start..record_start + 80];
                let record_str = String::from_utf8_lossy(record);
                
                if record_str.starts_with("END ") {
                    end_found = true;
                    break;
                }

                if let Some(eq_idx) = record_str.find('=') {
                    let key = record_str[0..eq_idx].trim().to_string();
                    let val_part = &record_str[eq_idx + 1..];
                    
                    let value = if let Some(slash_idx) = val_part.find('/') {
                        val_part[0..slash_idx].trim()
                    } else {
                        val_part.trim()
                    };
                    
                    // Strip quotes for strings
                    let value = value.trim_matches('\'').trim().to_string();
                    header_map.insert(key, value);
                }
            }

            offset += 2880;
            if end_found {
                data_start = offset;
                break;
            }
        }

        if !end_found {
            return Err(IndiError::DeviceNotFound("FITS END keyword not found".to_string()));
        }

        let width: u32 = header_map.get("NAXIS1").and_then(|s| s.parse().ok()).unwrap_or(0);
        let height: u32 = header_map.get("NAXIS2").and_then(|s| s.parse().ok()).unwrap_or(0);
        let bitpix: i32 = header_map.get("BITPIX").and_then(|s| s.parse().ok()).unwrap_or(8);
        let _bzero: f64 = header_map.get("BZERO").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let _bscale: f64 = header_map.get("BSCALE").and_then(|s| s.parse().ok()).unwrap_or(1.0);
        
        let bayer_pat = header_map.get("BAYERPAT").or(header_map.get("CCD_CFA"));
        let is_color = bayer_pat.is_some() || header_map.get("COLOR").map_or(false, |s| s == "T");

        if width == 0 || height == 0 {
            return Err(IndiError::DeviceNotFound("Invalid FITS dimensions".to_string()));
        }

        let pixel_count = (width * height) as usize;
        let raw_data = &buffer[data_start..];
        let mut pixels = vec![0.0; pixel_count];

        match bitpix {
            8 => {
                if raw_data.len() < pixel_count {
                    return Err(IndiError::DeviceNotFound("Truncated 8-bit FITS data".to_string()));
                }
                for i in 0..pixel_count {
                    pixels[i] = raw_data[i] as f32 / 255.0;
                }
            }
            16 => {
                if raw_data.len() < pixel_count * 2 {
                    return Err(IndiError::DeviceNotFound("Truncated 16-bit FITS data".to_string()));
                }
                for i in 0..pixel_count {
                    let idx = i * 2;
                    let val = u16::from_be_bytes([raw_data[idx], raw_data[idx + 1]]);
                    pixels[i] = val as f32 / 65535.0;
                }
            }
            _ => {
                return Err(IndiError::DeviceNotFound(format!("Unsupported BITPIX: {}", bitpix)));
            }
        }

        let frame = Frame::from_f32_vec(pixels, width as usize, height as usize, 1)
            .map_err(|e| IndiError::DeviceNotFound(format!("Frame creation failed: {:?}", e)))?;

        Ok(frame)
    }
}

