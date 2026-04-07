use super::header::{write_fits_headers, write_fits_keyword, write_fits_keyword_string};
use super::metadata::FitsMetadata;
use crate::error::{Result, StackError};
use crate::ffi_safety::catch_ffi_panic;
use crate::frame::Frame;
use fitsio::images::{ImageDescription, ImageType};
use fitsio::FitsFile;
use std::io::Write;
use std::path::Path;

/// Write a monochrome frame to FITS
pub(crate) fn write_mono_fits(
    fptr: &mut FitsFile,
    frame: &Frame,
    metadata: Option<&FitsMetadata>,
) -> Result<()> {
    let width = frame.width();
    let height = frame.height();

    let description = ImageDescription {
        data_type: ImageType::Float,
        dimensions: &[height, width],
    };

    let hdu = catch_ffi_panic("cfitsio::create_image", || {
        fptr.create_image("".to_string(), &description)
    })
    .map_err(StackError::from)?
    .map_err(|e| StackError::ArithmeticError {
        message: format!("Failed to create FITS image HDU: {}", e),
    })?;

    let data = frame.data();
    catch_ffi_panic("cfitsio::write_image", || hdu.write_image(fptr, data))
        .map_err(StackError::from)?
        .map_err(|e| StackError::ArithmeticError {
            message: format!("Failed to write FITS image data: {}", e),
        })?;

    write_fits_headers(fptr, &hdu, metadata)?;

    Ok(())
}

/// Write an RGB frame to FITS
pub(crate) fn write_rgb_fits(
    fptr: &mut FitsFile,
    frame: &Frame,
    metadata: Option<&FitsMetadata>,
) -> Result<()> {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();

    let description = ImageDescription {
        data_type: ImageType::Float,
        dimensions: &[channels, height, width],
    };

    let hdu = catch_ffi_panic("cfitsio::create_image", || {
        fptr.create_image("".to_string(), &description)
    })
    .map_err(StackError::from)?
    .map_err(|e| StackError::ArithmeticError {
        message: format!("Failed to create FITS image HDU: {}", e),
    })?;

    let pixels_per_channel = width * height;
    let mut planar_data = vec![0.0f32; pixels_per_channel * channels];
    let data = frame.data();

    for y in 0..height {
        for x in 0..width {
            let pixel_idx = y * width + x;
            for c in 0..channels {
                let src_idx = pixel_idx * channels + c;
                let dst_idx = c * pixels_per_channel + pixel_idx;
                planar_data[dst_idx] = data[src_idx];
            }
        }
    }

    catch_ffi_panic("cfitsio::write_image", || {
        hdu.write_image(fptr, &planar_data)
    })
    .map_err(StackError::from)?
    .map_err(|e| StackError::ArithmeticError {
        message: format!("Failed to write FITS image data: {}", e),
    })?;

    write_fits_headers(fptr, &hdu, metadata)?;

    catch_ffi_panic("cfitsio::write_key", || {
        hdu.write_key(fptr, "CTYPE3", "RGB")
    })
    .map_err(StackError::from)?
    .map_err(|e| StackError::ArithmeticError {
        message: format!("Failed to write FITS header: {}", e),
    })?;

    Ok(())
}

/// Write 16-bit data directly to PRIMARY HDU
pub(crate) fn write_fits_u16_primary(
    data: &[u16],
    width: usize,
    height: usize,
    channels: usize,
    path: &Path,
    metadata: Option<&FitsMetadata>,
) -> Result<()> {
    // Build FITS header
    let mut header = Vec::new();

    write_fits_keyword(&mut header, "SIMPLE", "T", "file conforms to FITS standard");
    write_fits_keyword(&mut header, "BITPIX", "16", "number of bits per data pixel");

    let naxis = if channels == 1 { 2 } else { 3 };
    write_fits_keyword(
        &mut header,
        "NAXIS",
        &naxis.to_string(),
        "number of array dimensions",
    );
    write_fits_keyword(&mut header, "NAXIS1", &width.to_string(), "width");
    write_fits_keyword(&mut header, "NAXIS2", &height.to_string(), "height");
    if channels > 1 {
        write_fits_keyword(
            &mut header,
            "NAXIS3",
            &channels.to_string(),
            "color channels",
        );
    }

    write_fits_keyword(&mut header, "BZERO", "32768", "offset for unsigned 16-bit");
    write_fits_keyword(&mut header, "BSCALE", "1", "scale factor");

    if let Some(meta) = metadata {
        if let Some(exp) = meta.exposure_s {
            write_fits_keyword(
                &mut header,
                "EXPTIME",
                &format!("{:.6}", exp),
                "exposure time in seconds",
            );
        }
        if let Some(gain) = meta.gain {
            write_fits_keyword(&mut header, "GAIN", &gain.to_string(), "gain value");
        }
        if let Some(ref camera) = meta.camera {
            write_fits_keyword_string(&mut header, "INSTRUME", camera, "camera name");
        }
        if let Some(ref sw) = meta.software {
            write_fits_keyword_string(&mut header, "SOFTWARE", sw, "software name");
        }
    }

    header.extend_from_slice(b"END");
    header.extend(std::iter::repeat(b' ').take(80 - 3));

    let header_blocks = (header.len() + 2879) / 2880;
    header.resize(header_blocks * 2880, b' ');

    let mut data_bytes = Vec::with_capacity(data.len() * 2);
    for &val in data {
        let signed = (val as i32 - 32768) as i16;
        data_bytes.extend_from_slice(&signed.to_be_bytes());
    }

    let data_blocks = (data_bytes.len() + 2879) / 2880;
    data_bytes.resize(data_blocks * 2880, 0);

    let mut file = std::fs::File::create(path).map_err(|e| StackError::ArithmeticError {
        message: format!("Failed to create FITS file: {}", e),
    })?;

    file.write_all(&header)
        .map_err(|e| StackError::ArithmeticError {
            message: format!("Failed to write FITS header: {}", e),
        })?;

    file.write_all(&data_bytes)
        .map_err(|e| StackError::ArithmeticError {
            message: format!("Failed to write FITS data: {}", e),
        })?;

    Ok(())
}
