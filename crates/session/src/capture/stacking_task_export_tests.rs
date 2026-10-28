//! The saved stack on disk, through the stacking task's own export and a real disk writer:
//! cropped FITS and PNG with `auto_crop` on, the whole stack with it off.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{save_stacked_result, StackCrop};
use crate::capture::context::{LiveStacker, StackSettings, StackingContext};
use crate::state::{AppState, CameraRole, CaptureSettings, ConnectedCameraInfo};
use night_amplifier_core::disk_writer::WritingSessionType;
use night_amplifier_core::frame::Frame;
use night_amplifier_core::registration::AffineTransform;

/// A 128² colour stack, deep enough to be cropped: a reference and subs whose leftmost 8 px
/// fell outside the warp, over a gradient sky so the PNG's render has something to measure.
fn drifted_stack() -> StackingContext {
    let size = 128;
    let settings = StackSettings::of(&CaptureSettings::default(), &night_amplifier_core::plugins::Plugins::none());
    let mut sky = Frame::zeros(size, size, 3).unwrap();
    for c in 0..3 {
        for y in 0..size {
            for x in 0..size {
                sky.set_pixel(x, y, c, 0.05 + 0.1 * x as f32 / size as f32 + 0.002 * ((x * 7 + y * 13) % 5) as f32);
            }
        }
    }
    let mut drifted = sky.clone();
    for c in 0..3 {
        for y in 0..size {
            for x in 0..8 {
                drifted.set_pixel(x, y, c, 0.0); // the warp border's value
            }
        }
    }
    let mut ctx = StackingContext::new(size, size, 3, &settings).expect("context");
    ctx.stacker.add_reference(&sky).unwrap();
    for _ in 1..night_amplifier_core::stacking::MIN_CROP_DEPTH {
        ctx.stacker.add_frame(&drifted, &AffineTransform::identity()).unwrap();
    }
    ctx
}

struct Saved {
    fits: Frame,
    /// `XORGSUBF`/`YORGSUBF`, when the header carries them.
    origin: Option<(i64, i64)>,
    png: image::RgbImage,
}

/// The value of an integer card in a FITS file's headers, read from the raw 80-byte cards.
fn card(bytes: &[u8], keyword: &str) -> Option<i64> {
    bytes.chunks(80).find_map(|card| {
        let card = std::str::from_utf8(card).ok()?;
        let (key, value) = card.split_once('=')?;
        (key.trim() == keyword).then(|| value.split('/').next().unwrap().trim().parse().ok())?
    })
}

fn file_ending(dir: &Path, suffix: &str) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("no *{suffix} in {dir:?}"))
}

/// Ends a stacking session the way the stacking task does, with `auto_crop` set to `on`.
fn save(ctx: &StackingContext, on: bool) -> (Saved, CaptureSettings) {
    save_through(ctx, on, None)
}

/// [`save`], the render task having shown the stack through noise `window`.
fn save_through(ctx: &StackingContext, on: bool, window: Option<f32>) -> (Saved, CaptureSettings) {
    let (state, writer) = AppState::new_for_testing();
    let state = Arc::new(state);
    let writer = std::thread::spawn(move || writer.run());
    let captures = state.disk_writer.raw_dir().parent().unwrap().to_path_buf();
    state.settings.update(|s| {
        s.stacking = true;
        s.wanderer_mode = false;
        s.save_stacked_image = true;
        s.auto_crop = on;
    });
    state.roster.install(
        ConnectedCameraInfo {
            id: "mock_0".into(),
            provider: "Mock".into(),
            index: 0,
            role: CameraRole::Main,
            info: night_amplifier_core::camera::CameraInfo { name: "Mock".into(), ..Default::default() },
        },
        false,
    );
    state.frame_captured(true, true, None);
    state.disk_writer.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();

    if let Some(window) = window {
        state.stack_noise.publish(0, window);
    }
    save_stacked_result(&state, Some(ctx as &dyn LiveStacker), &mut StackCrop::new(), 0);

    let settings = (*state.settings.snapshot()).clone();
    state.disk_writer.end_session();
    drop(state);
    writer.join().unwrap();

    let stacked = captures.join("stacked");
    let fits_path = file_ending(&stacked, ".fits");
    let bytes = std::fs::read(&fits_path).unwrap();
    let saved = Saved {
        fits: night_amplifier_core::fits::read_frame(&fits_path).unwrap(),
        origin: card(&bytes, "XORGSUBF").zip(card(&bytes, "YORGSUBF")),
        png: image::open(file_ending(&stacked, "_stretched.png")).unwrap().to_rgb8(),
    };
    std::fs::remove_dir_all(&captures).ok();
    (saved, settings)
}

#[test]
fn the_saved_stack_is_cropped_with_the_setting_on() {
    let ctx = drifted_stack();
    let (saved, _) = save(&ctx, true);

    assert_eq!((saved.fits.width(), saved.fits.height()), (120, 128));
    assert_eq!(saved.origin, Some((8, 0)));
    assert_eq!(saved.png.dimensions(), (120, 128));
}

/// With the setting off the files are what they were before `auto_crop` existed: the whole
/// stack, no origin card, and the PNG the export renders from the uncropped accumulator.
#[test]
fn the_saved_stack_is_whole_with_the_setting_off() {
    let ctx = drifted_stack();
    let (saved, settings) = save(&ctx, false);

    assert_eq!((saved.fits.width(), saved.fits.height()), (128, 128));
    assert_eq!(saved.origin, None);

    let (whole, coverage) = ctx.snapshot().unwrap();
    let (expected, w, h) = crate::capture::storage::render_stacked_png(
        whole,
        &settings,
        &night_amplifier_core::plugins::Plugins::installed(),
        ctx.depth() as u32,
        coverage,
        1.0,
    )
    .unwrap();
    assert_eq!(saved.png.dimensions(), (w, h));
    assert!(saved.png.as_raw() == &expected, "the saved PNG is not the uncropped stack's render");
}

/// The saved PNG is the live view's picture: rendered through the noise window the render
/// task carried the stack's figure through as its crop moved, not the final crop's own MAD.
#[test]
fn the_saved_png_renders_through_the_live_view_s_noise_window() {
    let ctx = drifted_stack();
    let (plain, settings) = save_through(&ctx, false, None);
    let (through, _) = save_through(&ctx, false, Some(0.8));
    assert!(plain.png.as_raw() != through.png.as_raw(), "the window did not reach the saved PNG");

    let (whole, coverage) = ctx.snapshot().unwrap();
    let plugins = night_amplifier_core::plugins::Plugins::installed();
    let (expected, _, _) =
        crate::capture::storage::render_stacked_png(whole, &settings, &plugins, ctx.depth() as u32, coverage, 0.8).unwrap();
    assert!(through.png.as_raw() == &expected, "the saved PNG is not the stack's render through its window");
}

