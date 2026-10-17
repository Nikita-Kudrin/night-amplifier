//! PNG encoding for already-rendered 8-bit buffers.
//!
//! Takes finished bytes rather than a `Frame`, for the same reason the disk export
//! does: the pixels handed in are what [`super::frame_to_rgb8_downsampled`]
//! produced — the whole live-view pipeline, background through quantization — and
//! re-deriving them here would reimplement it and drift.

use std::io::Write;

/// Encode interleaved 8-bit pixels into `sink`.
///
/// Generic over the sink so the disk writer can stream straight into a file while
/// the HTTP snapshot collects a `Vec`: buffering a whole PNG in memory is fine for
/// one download, and wrong for the frame-by-frame export path.
pub fn write_png<W: Write>(
    sink: W,
    pixels: &[u8],
    width: u32,
    height: u32,
    color: png::ColorType,
) -> Result<(), std::io::Error> {
    let mut encoder = png::Encoder::new(sink, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);

    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer
        .write_image_data(pixels)
        .map_err(std::io::Error::other)?;
    writer.finish().map_err(std::io::Error::other)
}

/// Encode interleaved RGB8 to an in-memory PNG.
pub fn encode_rgb8_png(rgb8: &[u8], width: u32, height: u32) -> Result<Vec<u8>, std::io::Error> {
    let mut buffer = Vec::new();
    write_png(&mut buffer, rgb8, width, height, png::ColorType::Rgb)?;
    Ok(buffer)
}

/// Encode interleaved RGBA8 to an in-memory PNG.
pub fn encode_rgba8_png(rgba8: &[u8], width: u32, height: u32) -> Result<Vec<u8>, std::io::Error> {
    let mut buffer = Vec::new();
    write_png(&mut buffer, rgba8, width, height, png::ColorType::Rgba)?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(png_bytes: &[u8]) -> (Vec<u8>, u32, u32, png::ColorType) {
        let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = decoder.read_info().expect("not a readable PNG");
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).expect("no frame in PNG");
        buf.truncate(info.buffer_size());
        (buf, info.width, info.height, info.color_type)
    }

    #[test]
    fn rgb8_round_trips_through_the_encoder() {
        let rgb8 = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];

        let encoded = encode_rgb8_png(&rgb8, 2, 2).unwrap();

        let (pixels, width, height, color) = decode(&encoded);
        assert_eq!((width, height), (2, 2));
        assert_eq!(color, png::ColorType::Rgb);
        assert_eq!(pixels, rgb8);
    }

    #[test]
    fn rgba8_keeps_the_alpha_channel() {
        let rgba8 = vec![1, 2, 3, 0, 4, 5, 6, 255];

        let encoded = encode_rgba8_png(&rgba8, 2, 1).unwrap();

        let (pixels, width, height, color) = decode(&encoded);
        assert_eq!((width, height), (2, 1));
        assert_eq!(color, png::ColorType::Rgba);
        assert_eq!(pixels, rgba8);
    }
}
