use std::path::Path;

/// Write an already-rendered interleaved 8-bit RGB buffer to a PNG file. Takes
/// finished bytes rather than a `Frame` on purpose: the stacked-frame export hands
/// this the exact pixels [`crate::server::encoding::frame_to_rgb8_downsampled`]
/// produced — the same conversion live view streams through (background, stretch,
/// saturation, contrast, denoise, quantization). Re-deriving from a `Frame` here
/// would reimplement that pipeline — exactly how PNG export used to drift and skip
/// denoising entirely. See `server::capture::storage::render_stacked_png`.
pub(crate) fn write_rgb8_png(
    rgb8: &[u8],
    width: u32,
    height: u32,
    path: &Path,
) -> Result<(), std::io::Error> {
    use std::fs::File;
    use std::io::BufWriter;

    // Streamed into the file rather than encoded to a `Vec` first: this runs once
    // per stacked frame, and the snapshot download is the only caller that wants
    // the whole PNG in memory.
    let file = BufWriter::new(File::create(path)?);
    crate::server::encoding::write_png(file, rgb8, width, height, png::ColorType::Rgb)
}
