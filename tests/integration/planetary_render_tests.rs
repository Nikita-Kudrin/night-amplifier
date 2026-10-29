//! Planetary's levels and max-channel curve on real discs, through the production preview
//! (`planetary_render`). Before them (2026-10-29) the deep-sky stages ran on these frames: the
//! gradient model took a Moon panel's median to 4 of 255 and the S-curve clipped 53-67 % of every
//! Jupiter; Deep Sky's own profile clipped them whole.

use night_amplifier::frame::Frame;
use night_amplifier::render::planetary::curve;

use super::instruments::{load_sub, Replay};
use super::planetary::{moon, planetary_settings as moon_settings};
use super::planetary_render::*;

/// What every planetary render must hold on its target, in output levels.
fn assert_rendered(name: &str, r: &TargetRender) {
    println!("{name:28} target {:6.2} %  clip {:5.2} %  crush {:5.2} %  p1 {:5.1}  p50 {:5.1}  p99 {:5.1}  sky {:>5}  residual {:.2}",
        100.0 * r.target_share, r.clipped, r.crushed, r.p1, r.p50, r.p99,
        r.sky.map_or("-".into(), |s| format!("{s:.1}")), r.residual);
    assert!(r.clipped <= 0.1, "{name}: {:.2} % of the target clipped", r.clipped);
    assert!(r.crushed <= 0.5, "{name}: {:.2} % of the target crushed to black", r.crushed);
    assert!((200.0..=245.0).contains(&r.p99), "{name}: the target's top at {:.1}", r.p99);
    assert!(r.residual < 1.0, "{name}: {:.2} levels of something beside the curve ran", r.residual);
    if let Some(sky) = r.sky {
        assert!(sky <= 3.0, "{name}: the sky at {sky:.1}");
    }
}

#[test]
#[ignore]
fn every_moon_mosaic_panel_renders_unclipped_and_uncrushed() {
    let dir = fixture(MOSAIC);
    for (time, why) in MOSAIC_PANELS {
        let frame = load_sub(&mosaic_panel(&dir, time)).frame;
        let (rgb8, levels) = render(&frame, &planetary_settings(false), 1);
        let rendered = measure(&frame, &rgb8);
        assert_rendered(&format!("{MOSAIC} {time}"), &rendered);
        assert!((100.0..=190.0).contains(&rendered.p50), "{time} ({why}): the surface's median at {:.1}", rendered.p50);
        if time == "19_41_40" {
            assert_eq!(levels.black, 0.0, "{why}: no sky, so the sensor's black: {levels:?}");
        }
    }
}

#[test]
#[ignore]
fn every_planet_renders_unclipped_with_its_colour_kept() {
    let dir = fixture(PLANETS);
    for (file, why) in PLANET_FILES {
        let frame = load_sub(&dir.join(file)).frame;
        let (rgb8, _) = render(&frame, &planetary_settings(false), 1);
        let rendered = measure(&frame, &rgb8);
        assert_rendered(&format!("{PLANETS} {file}"), &rendered);

        // Background neutralisation balanced a black sky and turned Jupiter green: the disc's
        // colour must leave the render as it came in, the curve scaling every channel alike.
        let target = unclipped_target(&frame, &rgb8);
        let input = target_colour(|i, c| f64::from(frame.channel_data(c)[i]), &target);
        let output = target_colour(|i, c| f64::from(rgb8[i * 3 + c]), &target);
        for (k, channel) in ["red", "blue"].into_iter().enumerate() {
            assert!((output[k] / input[k] - 1.0).abs() < 0.03, "{file} ({why}): {channel}/green {:.3} -> {:.3}", input[k], output[k]);
        }
    }
}

#[test]
#[ignore]
fn the_drifting_moon_renders_unclipped_live_and_stacked() {
    let mut live = Replay::with_settings(&moon(), planetary_settings(false));
    let first = live.capture().unwrap();
    let (rgb8, _) = render(&first, &planetary_settings(false), 1);
    let one = measure(&first, &rgb8);
    assert_rendered("moon live frame", &one);

    let mut replay = Replay::with_settings(&moon(), moon_settings());
    replay.run();
    let stack = replay.snapshot();
    let (rgb8, _) = render(&stack, &planetary_settings(true), replay.depth() as u32);
    let stacked = measure(&stack, &rgb8);
    assert_rendered(&format!("moon stack of {}", replay.depth()), &stacked);
    assert!((one.p50 - stacked.p50).abs() < 10.0, "live {:.1} against stacked {:.1}", one.p50, stacked.p50);
}

/// The fixture's frames 4-6 are the capture's 31-33, its largest frame-to-frame jitter: a
/// per-frame solve must not make the surface flicker. Measured: 0.1-0.3 levels of mid-grey.
#[test]
#[ignore]
fn a_jittering_moon_holds_its_levels_frame_to_frame() {
    let mut replay = Replay::with_settings(&moon(), planetary_settings(false));
    let frames: Vec<Frame> = (0..7).map(|_| replay.capture().unwrap()).collect();
    let mid_grey = |frame: &Frame| {
        let (_, levels) = render(frame, &planetary_settings(false), 1);
        255.0 * curve((0.37 - levels.black) / levels.span())
    };
    let greys: Vec<f32> = frames[4..7].iter().map(mid_grey).collect();
    for pair in greys.windows(2) {
        assert!((pair[1] - pair[0]).abs() < 1.5, "mid-grey moved {:.2} -> {:.2}", pair[0], pair[1]);
    }
}

/// Levels take exposure out of the picture: Jupiter at a fifth of its exposure, noisier, renders
/// where the well-exposed one does. The old profile put its median at 21 of 255.
#[test]
#[ignore]
fn an_underexposed_jupiter_renders_like_a_well_exposed_one() {
    let frame = load_sub(&fixture(PLANETS).join("jupiter_io_transit.tif")).frame;
    let mut dim = frame.clone();
    let mut seed = 7u32;
    for c in 0..3 {
        for v in dim.channel_data_mut(c) {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let noise = ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.002;
            *v = (*v * 0.2 + noise).clamp(0.0, 1.0);
        }
    }
    let (bright, _) = render(&frame, &planetary_settings(false), 1);
    let (faint, _) = render(&dim, &planetary_settings(false), 1);
    let (bright, faint) = (measure(&frame, &bright), measure(&frame, &faint));
    println!("well exposed {bright:?}\nat a fifth   {faint:?}");
    assert!((bright.p50 - faint.p50).abs() < 6.0 && (bright.p99 - faint.p99).abs() < 6.0, "{bright:?} against {faint:?}");
}
