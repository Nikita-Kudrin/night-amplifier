//! The render task applying `StackedFrame::crop`: what it publishes, at which size, and
//! what it announces.

use std::sync::Arc;

use super::QueueDepth;
use crate::events::{ServerEvent, StreamCrop};
use crate::state::{AppState, CaptureSettings, Resolution};
use night_amplifier_core::frame::Frame;
use night_amplifier_core::stacking::CropRect;

fn passthrough_settings() -> CaptureSettings {
    CaptureSettings {
        auto_stretch: false,
        background_subtraction: false,
        saturation_boost: false,
        preview_resolution: Resolution::Native,
        auto_crop: true,
        ..CaptureSettings::default()
    }
}

/// Every sample names its own position, so a crop from the wrong origin shows.
fn labelled(width: usize, height: usize) -> Frame {
    let mut frame = Frame::zeros(width, height, 3).unwrap();
    for c in 0..3 {
        for y in 0..height {
            for x in 0..width {
                frame.set_pixel(x, y, c, (c * 100_000 + y * 300 + x) as f32 / 1e6);
            }
        }
    }
    frame
}

fn stack_of(frame: Frame, crop: Option<CropRect>, settings: CaptureSettings) -> super::StackedFrame {
    super::StackedFrame {
        display_frame: Arc::new(frame),
        showing_stack: true,
        was_stacked: true,
        frame_number: 1,
        settings: settings.into(),
        stack_depth: 10,
        stack_generation: 0,
        noise: None,
        crop,
        star_fwhm: None,
    }
}

async fn render(state: &Arc<AppState>, frames: Vec<super::StackedFrame>) {
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || {
        // One run per frame, so draining never merges them.
        for frame in frames {
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(frame).unwrap();
            drop(tx);
            super::run_render_task(Arc::clone(&state), rx, QueueDepth::default());
        }
    })
    .await
    .unwrap();
}

fn published(state: &AppState) -> Arc<night_amplifier_core::render::display::RenderReadyFrame> {
    state.main_stream.get_latest_raw_frame().expect("the render task published a frame")
}

const CROP: CropRect = CropRect { x: 16, y: 8, width: 200, height: 136 };

#[tokio::test]
async fn a_cropped_stack_is_published_at_its_crop() {
    let state = Arc::new(AppState::new_for_testing().0);
    let whole = labelled(256, 192);

    render(&state, vec![stack_of(whole.clone(), Some(CROP), passthrough_settings())]).await;

    let frame = &published(&state).linear_frame;
    assert_eq!((frame.width(), frame.height()), (200, 136));
    for c in 0..3 {
        assert_eq!(frame.get_pixel(0, 0, c), whole.get_pixel(16, 8, c), "channel {c} lost its origin");
        assert_eq!(frame.get_pixel(199, 135, c), whole.get_pixel(215, 143, c), "channel {c} lost its far corner");
    }
    assert_eq!(
        state.stream_crop.get(),
        Some(StreamCrop { x: 16, y: 8, width: 200, height: 136, source_width: 256, source_height: 192 })
    );
}

/// With no crop the frame is published whole and nothing is announced: what the
/// stacking task sends for every frame while `auto_crop` is off.
#[tokio::test]
async fn without_a_crop_the_stack_is_published_whole() {
    let state = Arc::new(AppState::new_for_testing().0);
    let mut events = state.subscribe_events();
    let whole = labelled(256, 192);

    render(&state, vec![stack_of(whole.clone(), None, passthrough_settings())]).await;

    let frame = &published(&state).linear_frame;
    assert_eq!((frame.width(), frame.height()), (256, 192));
    assert!(frame.data() == whole.data(), "an uncropped stack changed");
    assert_eq!(state.stream_crop.get(), None);
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, ServerEvent::StackCropChanged { .. }), "announced {event:?}");
    }
}

/// Defence in depth: a frame whose own settings have `auto_crop` off is shown whole, even
/// if a crop came with it.
#[tokio::test]
async fn a_frame_with_the_setting_off_is_never_cut() {
    let state = Arc::new(AppState::new_for_testing().0);
    let off = CaptureSettings { auto_crop: false, ..passthrough_settings() };

    render(&state, vec![stack_of(labelled(256, 192), Some(CROP), off)]).await;

    let frame = &published(&state).linear_frame;
    assert_eq!((frame.width(), frame.height()), (256, 192));
    assert_eq!(state.stream_crop.get(), None);
}

/// The factor is the uncropped stack's: on the crop, 2400² → 2000² at 1080p flips 2 → 1.
#[tokio::test]
async fn the_bin_is_resolved_on_the_uncropped_stack() {
    let state = Arc::new(AppState::new_for_testing().0);
    let settings = CaptureSettings { preview_resolution: Resolution::Hd1080, ..passthrough_settings() };
    let crop = CropRect { x: 200, y: 200, width: 2000, height: 2000 };

    let frame = Frame::filled(2400, 2400, 3, 0.25).unwrap();
    render(&state, vec![stack_of(frame, Some(crop), settings)]).await;

    let frame = &published(&state).linear_frame;
    assert_eq!((frame.width(), frame.height()), (1000, 1000), "binned by the crop's factor");
}

/// An in-flight solve holding the stack keeps it whole, while the live view gets the cut,
/// binned as an unshared frame would be.
#[tokio::test]
async fn a_stack_shared_with_a_solve_is_cut_for_the_view_alone() {
    let state = Arc::new(AppState::new_for_testing().0);
    let settings = CaptureSettings { preview_resolution: Resolution::Hd1080, ..passthrough_settings() };
    let crop = CropRect { x: 200, y: 200, width: 2000, height: 2000 };
    let message = stack_of(labelled(2400, 2400), Some(crop), settings);
    let solving = Arc::clone(&message.display_frame);

    render(&state, vec![message]).await;

    let frame = &published(&state).linear_frame;
    assert_eq!((frame.width(), frame.height()), (1000, 1000));
    let mut expected = labelled(2400, 2400);
    expected.crop_in_place(&crop).unwrap();
    let expected = expected.downsample(2).unwrap();
    for c in 0..3 {
        assert_eq!(frame.get_pixel(0, 0, c), expected.get_pixel(0, 0, c), "channel {c} lost its origin");
        assert_eq!(frame.get_pixel(999, 999, c), expected.get_pixel(999, 999, c), "channel {c} lost its corner");
    }
    assert_eq!((solving.width(), solving.height()), (2400, 2400), "the solve's frame was cut");
    assert!(state.stream_crop.get().is_some());
}

/// Each change once, in order, as the frames that carry it are published.
#[tokio::test]
async fn crop_changes_are_announced_once_each() {
    let state = Arc::new(AppState::new_for_testing().0);
    let mut events = state.subscribe_events();
    let other = CropRect { x: 24, ..CROP };

    render(
        &state,
        vec![
            stack_of(labelled(256, 192), Some(CROP), passthrough_settings()),
            stack_of(labelled(256, 192), Some(CROP), passthrough_settings()),
            stack_of(labelled(256, 192), Some(other), passthrough_settings()),
            stack_of(labelled(256, 192), None, passthrough_settings()),
        ],
    )
    .await;

    let mut announced = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let ServerEvent::StackCropChanged { crop, .. } = event {
            announced.push(crop.map(|c| c.x));
        }
    }
    assert_eq!(announced, vec![Some(16), Some(24), None]);
}

/// A rect the frame cannot hold is a bug upstream, never a lost frame.
#[tokio::test]
async fn a_crop_that_does_not_fit_shows_the_stack_whole() {
    let state = Arc::new(AppState::new_for_testing().0);
    let outside = CropRect { x: 128, y: 0, width: 200, height: 64 };

    render(&state, vec![stack_of(labelled(256, 192), Some(outside), passthrough_settings())]).await;

    let frame = &published(&state).linear_frame;
    assert_eq!((frame.width(), frame.height()), (256, 192));
    assert_eq!(state.stream_crop.get(), None);
}

/// A 256² sky with a left-to-right gradient and noise, so every render stage has
/// something to measure.
fn sky() -> Frame {
    let (w, h) = (256, 256);
    let mut frame = Frame::zeros(w, h, 3).unwrap();
    let mut seed: u32 = 24680;
    for c in 0..3 {
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.004;
                frame.set_pixel(x, y, c, 0.05 + 0.15 * x as f32 / (w - 1) as f32 + noise);
            }
        }
    }
    frame
}

/// The saved PNG of a cropped stack is byte-for-byte the frame the render task streams for
/// it: one crop, cut by the same function, before the same pipeline, with every display stage
/// on. A crop applied after the render on either side (or to one side only) fails here.
#[tokio::test]
async fn the_saved_png_of_a_cropped_stack_is_what_the_live_view_streams() {
    use night_amplifier_core::frame::{NoiseField, NOISE_REDUCTION};
    use night_amplifier_core::render::display::frame_to_rgb8_downsampled;

    // Two thin columns at the left, cut away, and a 90 % patch inside the crop, kept.
    let cells = 256 / NOISE_REDUCTION;
    let coverage: Vec<f32> = (0..cells * cells)
        .map(|i| match (i % cells, i / cells) {
            (0 | 1, _) => 0.5,
            (10..=12, 10..=12) => 0.9,
            _ => 1.0,
        })
        .collect();
    let coverage = NoiseField::coverage_only(coverage, cells, cells, 3, 256, 256).unwrap();
    let crop = CropRect { x: 16, y: 0, width: 240, height: 256 };

    let mut settings = CaptureSettings {
        preview_resolution: Resolution::Native,
        auto_crop: true,
        background_subtraction: true,
        auto_stretch: true,
        saturation_boost: true,
        ..Default::default()
    };
    settings.eyepiece.intensity = 0.7;
    settings.eyepiece.black_floor = -0.03;
    settings.eyepiece.dither = true;
    settings.denoise.chroma = true;
    settings.denoise.luma_strength = 1.0;

    let state = Arc::new(AppState::new_for_testing().0);
    let mut message = stack_of(sky(), Some(crop), settings.clone());
    message.stack_depth = 40;
    message.noise = Some(coverage.clone());
    render(&state, vec![message]).await;
    let (streamed, w, h) = frame_to_rgb8_downsampled(&published(&state), u32::MAX, u32::MAX).unwrap();

    let mut saved = sky();
    let saved_coverage = crate::capture::stack_crop::crop_stack(&mut saved, Some(&coverage), &crop).unwrap();
    assert!(saved_coverage.is_some(), "the kept 90 % patch should leave the crop a map");
    let (exported, ew, eh) =
        crate::capture::storage::render_stacked_png(saved, &settings, &state.plugins, 40, saved_coverage, state.stack_noise.of(0)).unwrap();

    assert_eq!((ew, eh), (w, h));
    assert_eq!((w, h), (240, 256));
    assert!(exported == streamed, "the saved PNG differs from the cropped live render");
}
