//! Tests for the display output path: the black floor and ordered dither that
//! the fused encoders apply where a frame becomes 8-bit, and the resolution the
//! lossless stream encodes into.
//!
//! These measure the quantity that actually predicts what an observer sees at
//! the eyepiece — sky sigma expressed in **output 8-bit levels**, taken from the
//! bytes that reach the browser rather than from the linear frame. Every other
//! measurement in this repo is taken before the stretch, where it says nothing
//! about visible grain.

use std::path::Path;

use serial_test::serial;

use crate::integration::common::{find_image_files_in_dir, FIXTURES_DIR};
use crate::integration::image_loading::load_image;

/// The IMX533 fixture: 3008x3008, 2 s, gain 300 on a 250 mm Dobsonian. Square,
/// so the 1440 tier takes it to exactly 1440x1440 — the eyepiece screen's
/// native resolution.
const FIXTURE: &str = "250mm-dob-imx533-dumbbell-fits";

/// Robust sigma of one channel of an interleaved RGB8 buffer, in 8-bit levels.
///
/// MAD rather than a standard deviation: stars and the target occupy a small
/// fraction of the frame but would dominate a variance, and it is the *sky* this
/// is measuring.
fn sky_sigma_levels(rgb8: &[u8], channel: usize) -> f64 {
    let mut samples: Vec<f64> = rgb8
        .iter()
        .skip(channel)
        .step_by(3)
        .map(|&v| v as f64)
        .collect();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples[samples.len() / 2];

    let mut deviations: Vec<f64> = samples.iter().map(|v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
    deviations[deviations.len() / 2] * 1.4826
}

/// Run the real preview pipeline over a fixture frame and hand back the
/// `RenderReadyFrame` the encoders take, so these tests exercise the same tone
/// curve the eyepiece does rather than a synthetic one.
fn prepare_fixture(intensity: f32) -> Option<night_amplifier::server::state::RenderReadyFrame> {
    let dir = Path::new(FIXTURES_DIR).join(FIXTURE);
    let files = find_image_files_in_dir(&dir);
    let first = files.first()?;

    let loaded = load_image(first).ok()?;
    let mut frame = loaded.frame;
    if frame.channels() == 1 {
        frame = night_amplifier::debayer_auto(&frame).ok()?.0;
    }

    let mut settings = night_amplifier::server::state::CaptureSettings::default();
    settings.auto_stretch = true;
    settings.eyepiece.intensity = intensity;

    let (pipeline_config, stretch_result) =
        night_amplifier::server::capture::pipeline::process_preview_frame(&mut frame, &settings)
            .ok()?;

    Some(night_amplifier::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config,
        stretch_result,
    })
}

/// The headline number for Tier 0, reported rather than only bounded so a
/// regression is legible instead of just red.
///
/// Encoding into the viewport the eyepiece actually displays is an area average;
/// leaving the browser to minify a near-native frame is a four-tap bilinear
/// filter that discards most of that averaging as aliasing.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn lossless_stream_at_display_resolution_is_measurably_quieter() {
    let Some(ready) = prepare_fixture(0.0) else {
        println!("Fixture {FIXTURE} not present. Skipping.");
        return;
    };

    // What the stream used to send: capped at 4K, leaving the browser to shrink
    // 2160 down to the 1440 screen.
    let (native, nw, nh) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, 3840, 2160).unwrap();
    // What a 1440p eyepiece now asks for and receives.
    let (tiered, tw, th) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, 2560, 1440).unwrap();

    let native_sigma = sky_sigma_levels(&native, 1);
    let tiered_sigma = sky_sigma_levels(&tiered, 1);

    println!("\n=== Lossless stream resolution ===");
    println!("  4K cap (was):      {nw}x{nh}, sky sigma {native_sigma:.2} output levels");
    println!("  1440 tier (now):   {tw}x{th}, sky sigma {tiered_sigma:.2} output levels");
    println!(
        "  grain reduction:   {:.2}x, payload {:.2}x smaller",
        native_sigma / tiered_sigma,
        native.len() as f64 / tiered.len() as f64
    );

    assert_eq!((tw, th), (1440, 1440), "square sensor should fit the tier exactly");
    assert!(
        tiered_sigma < native_sigma,
        "encoding at display resolution should reduce sky sigma: {tiered_sigma:.2} vs {native_sigma:.2}"
    );
}

/// The dark blocks: with a black floor set, no pixel of a real stretched frame
/// may reach an OLED as a fully-off pixel.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn black_floor_removes_every_off_pixel_from_a_real_frame() {
    let Some(mut ready) = prepare_fixture(1.0) else {
        println!("Fixture {FIXTURE} not present. Skipping.");
        return;
    };

    // Without a floor, a meaningful share of the sky clamps to zero.
    ready.pipeline_config.display = night_amplifier::render::DisplayOutput::PLAIN;
    let (plain, _, _) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, 2560, 1440).unwrap();
    let zeros = plain.iter().filter(|&&b| b == 0).count();
    let zero_fraction = zeros as f64 / plain.len() as f64;

    ready.pipeline_config.display = night_amplifier::render::DisplayOutput::default()
        .with_pedestal(0.04)
        .with_dither(true);
    let (lifted, _, _) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, 2560, 1440).unwrap();

    println!("\n=== Black floor ===");
    println!("  without floor: {:.2}% of samples at exactly 0", zero_fraction * 100.0);
    println!("  with floor:    {} samples at 0", lifted.iter().filter(|&&b| b == 0).count());

    assert!(
        zero_fraction > 0.0,
        "fixture does not clip to black, so this test cannot detect the fix"
    );
    assert!(
        lifted.iter().all(|&b| b > 0),
        "black floor left samples on zero; an OLED switches those pixels off"
    );
}

/// Raising the eyepiece intensity must smooth the sky, which is what the slider
/// claims to do. Measured end to end on real data, in the units the eye sees.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn eyepiece_intensity_reduces_visible_sky_grain() {
    let (Some(low), Some(high)) = (prepare_fixture(0.0), prepare_fixture(1.0)) else {
        println!("Fixture {FIXTURE} not present. Skipping.");
        return;
    };

    let encode = |ready: &night_amplifier::server::state::RenderReadyFrame| {
        let (bytes, _, _) =
            night_amplifier::server::encoding::frame_to_rgb8_downsampled(ready, 2560, 1440)
                .unwrap();
        sky_sigma_levels(&bytes, 1)
    };

    let sigma_low = encode(&low);
    let sigma_high = encode(&high);

    println!("\n=== Eyepiece intensity vs visible grain ===");
    println!("  intensity 0.0: sky sigma {sigma_low:.2} output levels");
    println!("  intensity 1.0: sky sigma {sigma_high:.2} output levels");

    assert!(
        sigma_high < sigma_low,
        "raising eyepiece intensity must reduce visible sky grain, got {sigma_high:.2} from \
         {sigma_low:.2} — the black point factor is moving the wrong way again"
    );
}
