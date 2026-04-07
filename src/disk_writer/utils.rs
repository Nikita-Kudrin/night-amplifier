use crate::frame::Frame;
use std::path::Path;

/// Write a frame to PNG file (8-bit RGB)
pub(crate) fn write_png(frame: &Frame, path: &Path) -> Result<(), std::io::Error> {
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(path)?;
    let ref mut w = BufWriter::new(file);

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
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // Convert f32 [0.0, 1.0] to u8 [0, 255]
    let rgb8: Vec<u8> = frame
        .data()
        .iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();

    writer
        .write_image_data(&rgb8)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    Ok(())
}
