//! What a target filling the frame may do to the sky estimate: nothing.
//!
//! `black_point::estimate_background_mode` picks a histogram bin and then refines it
//! against the samples in it. Sizing that refinement window from a MAD over *every*
//! sample only works while what contaminates it is a minority — and a frame-filling
//! halo, which `background::target_disc` exists because of, is not. The window then
//! sizes itself around the target's spread, swallows it, and the median lands inside
//! it: measured +7.3 ADU on a synthetic halo over 69 % of the frame and +349 on a 75 %
//! ramp, against a raw histogram peak 1.4 ADU off the sky.
//!
//! Real sky, synthetic contaminant. A crop of a real globular tight enough to be
//! frame-filling has no ground truth — its sky *is* brighter in the middle, by 8 ADU
//! on this fixture — so the target is added to a stack whose sky is already measured.
//! Everything that makes the estimate hard (real grain, real stars, the session's own
//! gradient) is the fixture's; only the answer is known.

use serial_test::serial;

use crate::integration::stack_depth_grain_tests::{managed_session, stack_snapshots};

/// 8 subs of a globular at 2048², the eyepiece fixture. Its sky is uncontaminated over
/// most of the frame, which is what makes it usable as a baseline.
const FIXTURE_SET: &str = "globular-cluster-eyepiece";

/// The estimate may move by this much when a target is laid over the sky.
///
/// Half a sigma, against a measured worst of 0.34 at 90 % cover — and that residue is
/// the *bin* moving, not the refinement: the raw histogram peak walks 115.15 -> 113.28
/// ADU as the uncontaminated sky shrinks to a tenth of the frame, and `clipped_centre`
/// pulls it back toward the sky rather than away. The defect this bounds ran the other
/// way and far further: +3.1 sigma at 60 % cover, +55 at 75 %, +73 at 90 %.
const TOLERATED_DRAG_SIGMAS: f32 = 0.5;

/// Adds a smooth dome over the right-hand `share` of the frame, rising to
/// `peak_excess` above the sky at the far edge.
///
/// Smooth and continuous with the sky, not a disc: a target that steps well clear of
/// the sky falls outside any sane window and is harmless. What breaks a window sized
/// by the whole frame is a target whose samples run continuously out of the sky's own
/// distribution, which is what a halo, a Milky Way band and un-modelled vignetting all
/// look like.
fn lay_a_target_over(frame: &night_amplifier::Frame, share: f32, peak_excess: f32) -> night_amplifier::Frame {
    let (w, h, c) = (frame.width(), frame.height(), frame.channels());
    let cut = (w as f32 * (1.0 - share)) as usize;
    let mut out = frame.clone();
    for ch in 0..c {
        for y in 0..h {
            for x in cut..w {
                let t = (x - cut) as f32 / (w - cut).max(1) as f32;
                let v = out.get_pixel(x, y, ch) + peak_excess * t;
                out.set_pixel(x, y, ch, v);
            }
        }
    }
    out
}

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_target_filling_a_real_frame_does_not_drag_its_sky_estimate() {
    let files = managed_session(FIXTURE_SET);
    let snapshots = stack_snapshots(&files, &[]);
    let (depth, stack) = snapshots.last().expect("the session stacks");
    let stats = night_amplifier::statistics::compute_image_stats(stack).unwrap();
    let sigma = stats.mean_sigma();
    let sky = night_amplifier::render::estimate_background_mode(stack).mode;
    println!(
        "\n=== {FIXTURE_SET}, {depth} subs: sky {:.2} ADU, sigma {:.2} ADU ===",
        sky * 65535.0,
        sigma * 65535.0
    );

    // 7x the sky's own level at the far edge, which is what the outskirts of a bright
    // nebula reach over a dark site.
    let peak_excess = 7.0 * sky;
    let mut worst = (0.0f32, 0.0f32);
    for share in [0.0f32, 0.35, 0.5, 0.6, 0.75, 0.9] {
        let contaminated = lay_a_target_over(stack, share, peak_excess);
        let moved = night_amplifier::render::estimate_background_mode(&contaminated).mode - sky;
        println!(
            "  target over {:>3.0}% of the frame: sky estimate {:+6.2} ADU ({:+.2} sigma)",
            share * 100.0,
            moved * 65535.0,
            moved / sigma
        );
        if moved.abs() > worst.0.abs() {
            worst = (moved, share);
        }
    }

    assert!(
        worst.0.abs() < TOLERATED_DRAG_SIGMAS * sigma,
        "a target over {:.0}% of the frame moved the sky estimate {:+.2} ADU ({:+.2} \
         sigma) although the sky under it never changed — the refinement window is \
         being sized by the target",
        worst.1 * 100.0,
        worst.0 * 65535.0,
        worst.0 / sigma
    );
}
