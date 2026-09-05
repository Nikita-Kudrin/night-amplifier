//! Eyepiece snapshot download.
//!
//! Serves the frame the render task last published, as a PNG. Two shapes: the
//! round eyepiece image the view shows, and the uncropped frame behind it — the
//! same picture the stacked PNG export stores.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Local;
use rayon::prelude::*;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::server::dto::ApiResponse;
use crate::server::encoding::{encode_rgb8_png, frame_to_rgb8_downsampled};
use crate::server::state::{AppState, RenderReadyFrame};

/// One snapshot render at a time, process-wide.
///
/// A native-resolution render is the most expensive thing this server does on
/// demand: on a 26 MP sensor with the denoisers on it transiently holds ~933 MB of
/// denoise scratch plus two RGB8 buffers of up to 77 MB each, and takes ~0.5 s on
/// a desktop (several times that on a Pi 5). The eyepiece view is a *second-device*
/// view by design, so two clients clicking Download is ordinary use, not abuse —
/// unbounded, that is gigabytes at once on a board with 8 GB shared with the
/// pipeline. Callers that find it taken are told to come back rather than queued:
/// waiting holds a connection open for however long the frame in front takes.
static SNAPSHOT_SLOT: Semaphore = Semaphore::const_new(1);

/// What a busy server tells the client to wait, in seconds. The frontend retries
/// on this cadence; see `fetchEyepieceSnapshot`.
const RETRY_AFTER_SECS: u32 = 2;

#[derive(Debug, Deserialize, Default)]
pub struct SnapshotQuery {
    /// Round eyepiece image when set; the uncropped frame otherwise.
    #[serde(default)]
    circular: Option<bool>,
}

/// GET /api/eyepiece/snapshot
pub async fn get_snapshot(
    Query(query): Query<SnapshotQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    // Held for the whole render; released when this handler returns either way.
    let Ok(_permit) = SNAPSHOT_SLOT.try_acquire() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, RETRY_AFTER_SECS.to_string())],
            ApiResponse::err::<()>("Another snapshot is still rendering"),
        )
            .into_response();
    };

    // Not 503: nothing has been rendered yet, and no amount of retrying changes
    // that until a capture produces a frame. The client says so and stops.
    let Some(frame) = state.main_stream.get_latest_raw_frame().await else {
        return (
            StatusCode::NOT_FOUND,
            ApiResponse::err::<()>("No rendered frame available yet"),
        )
            .into_response();
    };

    let circular = query.circular.unwrap_or(false);
    // Off the async runtime: a full-sensor conversion plus a PNG encode is tens of
    // milliseconds of pure CPU, and it must not sit on a worker serving the streams.
    let encoded = tokio::task::spawn_blocking(move || render_snapshot_png(&frame, circular)).await;

    let png = match encoded {
        Ok(Ok(png)) => png,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiResponse::err::<()>(&format!("Snapshot encoding failed: {}", e)),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiResponse::err::<()>(&format!("Snapshot task failed: {}", e)),
            )
                .into_response()
        }
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", snapshot_filename(circular)),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        png,
    )
        .into_response()
}

/// Name the download carries.
///
/// Stamped here rather than in the browser because a `Content-Disposition`
/// filename overrides an `<a download>` attribute — a name chosen client-side
/// never reaches the disk, and every save lands on top of the last one. Local
/// time and `DD-MM-YYYY_HH-MM-SS`, matching the capture session folders.
fn snapshot_filename(circular: bool) -> String {
    let stamp = Local::now().format("%d-%m-%Y_%H-%M-%S");
    let suffix = if circular { "" } else { "-original" };
    format!("eyepiece_{stamp}{suffix}.png")
}

/// Convert one rendered frame to PNG bytes.
///
/// At the frame's own resolution, not a client's tier: a download is the copy the
/// observer keeps, so it is not held to whatever box the screen asked the stream
/// for. Passing the frame's own dimensions as the bounding box is what makes
/// [`frame_to_rgb8_downsampled`] a straight conversion with no resampling.
fn render_snapshot_png(frame: &RenderReadyFrame, circular: bool) -> Result<Vec<u8>, String> {
    let native_w = frame.linear_frame.width() as u32;
    let native_h = frame.linear_frame.height() as u32;
    let (rgb8, width, height) = frame_to_rgb8_downsampled(frame, native_w, native_h)?;

    if !circular {
        return encode_rgb8_png(&rgb8, width, height).map_err(|e| e.to_string());
    }

    let (masked, side) = circular_rgb(&rgb8, width, height);
    encode_rgb8_png(&masked, side, side).map_err(|e| e.to_string())
}

/// The round eyepiece image: the centre square of the frame, black outside the
/// circle inscribed in it. Returns the pixels and the side they form.
///
/// Square because that is what the view shows. Its canvas is `100cqmin` on both
/// axes with `object-fit: cover`, so a wide frame is centre-cropped to a square
/// *before* `clip-path: circle(closest-side)` cuts the circle out of it. Masking
/// the full rectangle instead put the same circle in the middle of a wide canvas —
/// the right pixels, but over half the file wasted on padding for an IMX464 frame
/// (2712x1538: 55.5 %) and a shape the observer never saw.
///
/// Black rather than transparent: an alpha channel is composited onto whatever the
/// viewer shows behind it, which is white in every default light theme — the field
/// stop came out glaring instead of dark. Opaque means RGB, so the fourth channel
/// carrying nothing but 0 or 255 goes with it.
fn circular_rgb(rgb8: &[u8], width: u32, height: u32) -> (Vec<u8>, u32) {
    let (w, side) = (width as usize, width.min(height) as usize);
    // Truncating halves, matching `object-fit: cover`'s centring on an odd margin.
    let (x0, y0) = ((w - side) / 2, (height as usize - side) / 2);
    let radius = side as f32 / 2.0;
    let radius_sq = radius * radius;
    let centre = side as f32 / 2.0;

    let mut masked = vec![0u8; side * side * 3];
    masked
        .par_chunks_mut(side * 3)
        .enumerate()
        .for_each(|(y, row)| {
            // Pixel centres, so the mask stays symmetric about the image centre.
            let dy = y as f32 + 0.5 - centre;
            let dy_sq = dy * dy;
            let src_row = (y0 + y) * w;
            for x in 0..side {
                let dx = x as f32 + 0.5 - centre;
                if dx * dx + dy_sq > radius_sq {
                    continue; // Left as the all-zero (black) initial value.
                }
                let src = (src_row + x0 + x) * 3;
                let dst = x * 3;
                row[dst..dst + 3].copy_from_slice(&rgb8[src..src + 3]);
            }
        });
    (masked, side as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_frame(width: usize, height: usize) -> RenderReadyFrame {
        let mut frame = crate::frame::Frame::filled(width, height, 3, 0.5).unwrap();
        // A gradient, so a resampled snapshot would not merely be a smaller flat
        // field that still passes a "colour survived" check.
        for y in 0..height {
            for x in 0..width {
                frame.set_pixel(x, y, 0, (x as f32) / (width as f32));
            }
        }
        RenderReadyFrame {
            linear_frame: std::sync::Arc::new(frame),
            pipeline_config: crate::render::RenderPipelineConfig::default(),
            stretch_result: None,
        }
    }

    fn png_info(bytes: &[u8]) -> (u32, u32, ::png::ColorType) {
        let decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
        let reader = decoder.read_info().expect("not a readable PNG");
        let info = reader.info();
        (info.width, info.height, info.color_type)
    }

    fn png_pixels(bytes: &[u8]) -> Vec<u8> {
        let decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().expect("not a readable PNG");
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).expect("no frame in PNG");
        buf.truncate(info.buffer_size());
        buf
    }

    /// The download is the observer's copy, so it must come out at the frame's own
    /// size — not the tier whatever screen is streaming happened to ask for.
    #[test]
    fn snapshot_keeps_the_frames_native_resolution() {
        let frame = ready_frame(37, 19);

        let png = render_snapshot_png(&frame, false).unwrap();

        assert_eq!(png_info(&png), (37, 19, ::png::ColorType::Rgb));
    }

    /// What the observer opens has to be black outside the field stop, all the way
    /// through the encoder. Transparency is what it must not be: a viewer composites
    /// alpha onto its own background, which is white in every default light theme.
    ///
    /// The fixture's corner is mid-grey on two channels before the mask, so a
    /// snapshot that skipped it would not pass this by accident.
    #[test]
    fn circular_snapshot_is_black_outside_the_field_stop() {
        let frame = ready_frame(24, 24);

        let png = render_snapshot_png(&frame, true).unwrap();

        assert_eq!(png_info(&png), (24, 24, ::png::ColorType::Rgb));
        let pixels = png_pixels(&png);
        assert_eq!(&pixels[0..3], &[0, 0, 0], "corner must be black");
        let centre = (12 * 24 + 12) * 3;
        assert_ne!(
            &pixels[centre..centre + 3],
            &[0, 0, 0],
            "the image itself must survive"
        );
    }

    /// The two shapes are the same picture; only the field stop differs. A
    /// divergence here would mean "Download original" is not the image on screen.
    #[test]
    fn both_shapes_come_from_one_render() {
        let frame = ready_frame(16, 16);

        let (plain, _, _) = frame_to_rgb8_downsampled(&frame, 16, 16).expect("conversion failed");
        let (masked, side) = circular_rgb(&plain, 16, 16);

        assert_eq!(side, 16);
        let centre = (8 * 16 + 8) * 3;
        assert_eq!(&masked[centre..centre + 3], &plain[centre..centre + 3]);
    }

    /// An image whose every RGB byte is non-zero, so a pixel the mask cleared is
    /// distinguishable from one it merely copied.
    fn rgb_fixture(width: usize, height: usize) -> Vec<u8> {
        (0..width * height * 3)
            .map(|i| (i % 200 + 55) as u8)
            .collect()
    }

    #[test]
    fn circular_mask_keeps_the_centre_and_blacks_out_the_corners() {
        let (w, h) = (8, 8);
        let rgb = rgb_fixture(w, h);

        let (masked, side) = circular_rgb(&rgb, w as u32, h as u32);

        let pixel = |x: usize, y: usize| -> [u8; 3] {
            let i = (y * side as usize + x) * 3;
            [masked[i], masked[i + 1], masked[i + 2]]
        };
        assert_ne!(pixel(4, 4), [0, 0, 0], "centre must keep the image");
        for (x, y) in [(0, 0), (7, 0), (0, 7), (7, 7)] {
            assert_eq!(pixel(x, y), [0, 0, 0], "corner ({x},{y}) must be black");
        }
    }

    /// Inside the circle the mask must be a straight copy — it decides which pixels
    /// survive, never what they look like.
    #[test]
    fn circular_mask_copies_colour_verbatim_inside_the_circle() {
        let (w, h) = (8, 8);
        let rgb = rgb_fixture(w, h);

        let (masked, _) = circular_rgb(&rgb, w as u32, h as u32);

        let centre = (4 * w + 4) * 3;
        assert_eq!(&masked[centre..centre + 3], &rgb[centre..centre + 3]);
    }

    /// The view draws a square canvas (`100cqmin`) filled with `object-fit: cover`,
    /// so a wide frame is centre-cropped *before* the circle is cut out of it.
    /// Masking the full rectangle instead left the same circle stranded in a wide,
    /// half-transparent canvas the observer never saw.
    #[test]
    fn circular_snapshot_is_the_square_the_view_shows() {
        let frame = ready_frame(2712, 1538);

        let png = render_snapshot_png(&frame, true).unwrap();

        assert_eq!(png_info(&png), (1538, 1538, ::png::ColorType::Rgb));
    }

    /// The square is the *centre* crop: an off-centre one would show a different
    /// part of the sky from the view it is supposed to reproduce.
    #[test]
    fn circular_snapshot_crops_from_the_centre() {
        let (w, h) = (16usize, 8usize);
        let rgb = rgb_fixture(w, h);

        let (masked, side) = circular_rgb(&rgb, w as u32, h as u32);

        assert_eq!(side, 8);
        // Centre of the crop is the centre of the frame: frame x = (16-8)/2 + 4.
        let frame_centre = (4 * w + 8) * 3;
        let crop_centre = (4 * side as usize + 4) * 3;
        assert_eq!(
            &masked[crop_centre..crop_centre + 3],
            &rgb[frame_centre..frame_centre + 3]
        );
    }

    /// No black margin beyond the circle's own corners: every cleared pixel must be
    /// one the field stop cut, not padding the crop should have removed.
    #[test]
    fn circular_snapshot_wastes_no_more_than_the_field_stop() {
        let (w, h) = (64usize, 32usize);

        let (masked, side) = circular_rgb(&rgb_fixture(w, h), w as u32, h as u32);

        // The fixture has no zero bytes, so an all-zero pixel is one the mask cleared.
        let cleared = masked
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|p| p.iter().all(|&b| b == 0))
            .count();
        let total = (side * side) as f64;
        // A circle inscribed in a square leaves 1 - pi/4 = 21.5 % in the corners.
        let ratio = cleared as f64 / total;
        assert!(
            (0.19..0.24).contains(&ratio),
            "cleared {ratio:.3} of the square; only the circle's corners should be"
        );
    }

    #[test]
    fn filenames_distinguish_the_two_shapes_and_carry_a_stamp() {
        let round = snapshot_filename(true);
        let original = snapshot_filename(false);

        assert!(round.starts_with("eyepiece_") && round.ends_with(".png"));
        assert!(original.ends_with("-original.png"));
        assert_ne!(round, original);
        // DD-MM-YYYY_HH-MM-SS between the prefix and the suffix.
        let stamp = round
            .trim_start_matches("eyepiece_")
            .trim_end_matches(".png");
        assert_eq!(stamp.len(), "DD-MM-YYYY_HH-MM-SS".len(), "got {stamp}");
    }

    /// The permit is what keeps two clients from each allocating a gigabyte. A
    /// second caller must be turned away rather than queued behind the first.
    #[tokio::test]
    async fn only_one_snapshot_renders_at_a_time() {
        let held = SNAPSHOT_SLOT.try_acquire().expect("slot should start free");

        assert!(
            SNAPSHOT_SLOT.try_acquire().is_err(),
            "a second render must be refused while one holds the slot"
        );

        drop(held);
        assert!(
            SNAPSHOT_SLOT.try_acquire().is_ok(),
            "the slot must free up once the render finishes"
        );
    }
}
