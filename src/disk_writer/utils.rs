use std::path::Path;

/// Write an already-rendered interleaved 8-bit RGB buffer to a PNG file.
///
/// Takes finished bytes rather than a `Frame` on purpose: the stretched
/// stacked-frame export hands this the exact pixels
/// [`crate::server::encoding::frame_to_rgb8_downsampled`] produced for it, which
/// is the same conversion the live view streams through — background, stretch,
/// saturation, contrast, spatial denoise and display quantization all included.
/// Re-deriving those bytes from a `Frame` here would mean reimplementing that
/// pipeline a second time, which is exactly how the PNG export used to drift
/// from what the pixel viewer actually showed (it skipped denoising entirely).
/// See `server::capture::storage::render_stacked_png`.
pub(crate) fn write_rgb8_png(
    rgb8: &[u8],
    width: u32,
    height: u32,
    path: &Path,
) -> Result<(), std::io::Error> {
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(path)?;
    let w = &mut BufWriter::new(file);

    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);

    let mut writer = encoder
        .write_header()
        .map_err(std::io::Error::other)?;

    writer
        .write_image_data(rgb8)
        .map_err(std::io::Error::other)?;

    Ok(())
}
