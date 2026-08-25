use crate::frame::Frame;
use std::path::Path;

/// Write a frame to PNG file (8-bit RGB)
pub(crate) fn write_png(frame: &Frame, path: &Path) -> Result<(), std::io::Error> {
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(path)?;
    let w = &mut BufWriter::new(file);

    let mut encoder = png::Encoder::new(w, frame.width() as u32, frame.height() as u32);

    let color_type = if frame.channels() == 1 {
        png::ColorType::Grayscale
    } else {
        png::ColorType::Rgb
    };
    encoder.set_color(color_type);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);

    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::other(e))?;

    // `to_rgb8_fast` owns the planar -> interleaved gather and the canonical
    // rounding. Duplicating the conversion here is what let the two drift apart.
    let rgb8 = frame.to_rgb8_fast();

    writer
        .write_image_data(&rgb8)
        .map_err(|e| std::io::Error::other(e))?;

    Ok(())
}
