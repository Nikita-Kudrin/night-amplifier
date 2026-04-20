use super::metadata::FitsMetadata;
use crate::error::Result;
use fitsio::FitsFile;

/// Write standard FITS headers from metadata
pub(crate) fn write_fits_headers(
    fptr: &mut FitsFile,
    hdu: &fitsio::hdu::FitsHdu,
    metadata: Option<&FitsMetadata>,
) -> Result<()> {
    // Always write BSCALE and BZERO for proper interpretation
    hdu.write_key(fptr, "BSCALE", 1.0f64).ok();
    hdu.write_key(fptr, "BZERO", 0.0f64).ok();

    if let Some(meta) = metadata {
        // Exposure time
        if let Some(exp) = meta.exposure_s {
            hdu.write_key(fptr, "EXPTIME", exp).ok();
            hdu.write_key(fptr, "EXPOSURE", exp).ok();
        }

        // Gain
        if let Some(gain) = meta.gain {
            hdu.write_key(fptr, "GAIN", gain).ok();
        }

        // Offset
        if let Some(offset) = meta.offset {
            hdu.write_key(fptr, "OFFSET", offset).ok();
        }

        // Camera/instrument
        if let Some(ref camera) = meta.camera {
            hdu.write_key(fptr, "INSTRUME", camera.as_str()).ok();
        }

        // Object name
        if let Some(ref object) = meta.object {
            hdu.write_key(fptr, "OBJECT", object.as_str()).ok();
        }

        // Observation date
        if let Some(ref date) = meta.date_obs {
            hdu.write_key(fptr, "DATE-OBS", date.as_str()).ok();
        }

        // Frame number
        if let Some(frame_num) = meta.frame_number {
            hdu.write_key(fptr, "FRAMENUM", frame_num as i64).ok();
        }

        // Stacked frames count
        if let Some(stacked) = meta.stacked_frames {
            hdu.write_key(fptr, "NCOMBINE", stacked as i64).ok();
        }

        // Software
        if let Some(ref sw) = meta.software {
            hdu.write_key(fptr, "SOFTWARE", sw.as_str()).ok();
        }

        // CFA pattern
        if let Some(ref cfa) = meta.cfa_pattern {
            hdu.write_key(fptr, "BAYERPAT", cfa.as_str()).ok();
        }

        // Binning
        if let Some(bin) = meta.binning {
            hdu.write_key(fptr, "XBINNING", bin as i32).ok();
            hdu.write_key(fptr, "YBINNING", bin as i32).ok();
        }

        // Temperature
        if let Some(temp) = meta.temperature {
            hdu.write_key(fptr, "CCD-TEMP", temp).ok();
        }

        // Target (set point) temperature
        if let Some(set_temp) = meta.set_temp_c {
            hdu.write_key(fptr, "SET-TEMP", set_temp).ok();
        }
    }

    Ok(())
}

/// Write a FITS keyword with numeric or boolean value
pub(crate) fn write_fits_keyword(header: &mut Vec<u8>, keyword: &str, value: &str, comment: &str) {
    let mut card = [b' '; 80];

    // Keyword (8 chars, left-justified)
    let keyword_bytes = keyword.as_bytes();
    card[..keyword_bytes.len().min(8)]
        .copy_from_slice(&keyword_bytes[..keyword_bytes.len().min(8)]);

    // "= " at positions 8-9
    card[8] = b'=';
    card[9] = b' ';

    // Value (right-justified in columns 11-30)
    let value_bytes = value.as_bytes();
    let value_start = 30 - value_bytes.len().min(20);
    card[value_start..value_start + value_bytes.len().min(20)]
        .copy_from_slice(&value_bytes[..value_bytes.len().min(20)]);

    // Comment after " / "
    if !comment.is_empty() {
        card[31] = b'/';
        card[32] = b' ';
        let comment_bytes = comment.as_bytes();
        let comment_len = comment_bytes.len().min(80 - 33);
        card[33..33 + comment_len].copy_from_slice(&comment_bytes[..comment_len]);
    }

    header.extend_from_slice(&card);
}

/// Write a FITS keyword with string value (in single quotes)
pub(crate) fn write_fits_keyword_string(
    header: &mut Vec<u8>,
    keyword: &str,
    value: &str,
    comment: &str,
) {
    let mut card = [b' '; 80];

    // Keyword (8 chars, left-justified)
    let keyword_bytes = keyword.as_bytes();
    card[..keyword_bytes.len().min(8)]
        .copy_from_slice(&keyword_bytes[..keyword_bytes.len().min(8)]);

    // "= " at positions 8-9
    card[8] = b'=';
    card[9] = b' ';

    // String value in quotes (starting at position 10)
    card[10] = b'\'';
    let value_bytes = value.as_bytes();
    let value_len = value_bytes.len().min(68);
    card[11..11 + value_len].copy_from_slice(&value_bytes[..value_len]);
    card[11 + value_len] = b'\'';

    // Comment after " / " if there's room
    let comment_start = 11 + value_len + 2;
    if comment_start < 77 && !comment.is_empty() {
        card[comment_start] = b'/';
        card[comment_start + 1] = b' ';
        let comment_bytes = comment.as_bytes();
        let comment_len = comment_bytes.len().min(80 - comment_start - 2);
        card[comment_start + 2..comment_start + 2 + comment_len]
            .copy_from_slice(&comment_bytes[..comment_len]);
    }

    header.extend_from_slice(&card);
}
