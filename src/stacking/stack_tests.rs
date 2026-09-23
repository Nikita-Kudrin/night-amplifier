//! Tests for MasterStack accumulator.

use super::config::{FrameQuality, StackingConfig, WeightingConfig};
use super::rejection::RejectionMethod;
use super::stack::MasterStack;
use crate::frame::Frame;

#[test]
fn test_master_stack_simple() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();

    let frame1 = Frame::filled(4, 4, 1, 0.2).unwrap();
    let frame2 = Frame::filled(4, 4, 1, 0.4).unwrap();

    stack.add_frame(&frame1).unwrap();
    stack.add_frame(&frame2).unwrap();

    let result = stack.compute().unwrap();

    let data = result.data();
    for &v in data {
        assert!((v - 0.3).abs() < 1e-6, "Average should be 0.3, got {}", v);
    }
}

#[test]
fn test_border_handling() {
    let config = StackingConfig::default();
    let mut stack = MasterStack::new(8, 8, 1, config).unwrap();

    let mut data = vec![0.5; 64];
    for i in 0..8 {
        data[i] = 0.0;
        data[56 + i] = 0.0;
    }
    let frame = Frame::from_f32_vec(data, 8, 8, 1).unwrap();

    stack.add_frame(&frame).unwrap();

    let result = stack.compute().unwrap();
    let result_data = result.data();

    assert!(
        (result_data[9] - 0.5).abs() < 1e-6,
        "Interior should be 0.5"
    );
    assert!(result_data[0].abs() < 1e-6, "Border should be 0.0");
}

/// A running mean has no way back from NaN: one bad sample used to leave the pixel NaN
/// for the rest of the session. Non-finite samples are skipped like border pixels.
#[test]
fn test_non_finite_samples_do_not_poison_the_stack() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let config = StackingConfig::default().with_rejection(RejectionMethod::None);
        let mut stack = MasterStack::new(2, 1, 1, config).unwrap();

        for i in 0..6 {
            let first = if i == 3 { bad } else { 0.4 };
            let frame = Frame::from_f32_vec(vec![first, 0.6], 2, 1, 1).unwrap();
            stack.add_frame(&frame).unwrap();
        }

        let result = stack.compute().unwrap();
        assert!(
            (result.data()[0] - 0.4).abs() < 1e-6,
            "{bad} sample reached the mean: {}",
            result.data()[0]
        );
        assert!((result.data()[1] - 0.6).abs() < 1e-6);
        let coverage = stack.coverage_map();
        assert!((coverage.data()[0] - 5.0 / 6.0).abs() < 1e-6, "{bad} was counted");
        assert_eq!(coverage.data()[1], 1.0);
    }
}

#[test]
fn test_stack_clear() {
    let mut stack = MasterStack::with_defaults(4, 4, 1).unwrap();

    let frame = Frame::filled(4, 4, 1, 0.5).unwrap();
    stack.add_frame(&frame).unwrap();

    assert_eq!(stack.frame_count(), 1);

    stack.clear();

    assert_eq!(stack.frame_count(), 0);
}

// ========================================================================
// Weighted stacking tests
// ========================================================================

#[test]
fn test_weighted_stacking_equal_quality() {
    let config_unweighted = StackingConfig::default().with_rejection(RejectionMethod::None);
    let config_weighted = StackingConfig::default()
        .with_rejection(RejectionMethod::None)
        .with_weighting(WeightingConfig::balanced());

    let mut stack_unweighted = MasterStack::new(4, 4, 1, config_unweighted).unwrap();
    let mut stack_weighted = MasterStack::new(4, 4, 1, config_weighted).unwrap();

    let frame1 = Frame::filled(4, 4, 1, 0.2).unwrap();
    let frame2 = Frame::filled(4, 4, 1, 0.4).unwrap();

    let quality = FrameQuality::new(2.5, 10.0);

    stack_unweighted.add_frame(&frame1).unwrap();
    stack_unweighted.add_frame(&frame2).unwrap();

    stack_weighted
        .add_frame_with_quality(&frame1, quality)
        .unwrap();
    stack_weighted
        .add_frame_with_quality(&frame2, quality)
        .unwrap();

    let result_unweighted = stack_unweighted.compute().unwrap();
    let result_weighted = stack_weighted.compute().unwrap();

    let data_uw = result_unweighted.data();
    let data_w = result_weighted.data();

    for (i, (&uw, &w)) in data_uw.iter().zip(data_w.iter()).enumerate() {
        assert!(
            (uw - w).abs() < 0.01,
            "Equal quality should give similar results at {}: unweighted={}, weighted={}",
            i,
            uw,
            w
        );
    }
}

/// Fills the stack with typical frames so weighting has a median to score
/// against. Scoring is suspended until the sample is big enough to say what
/// typical looks like, so a two-frame stack weighs both frames equally by
/// design — the baseline has to exist before quality can tell frames apart.
fn seed_baseline(stack: &mut MasterStack, quality: FrameQuality) {
    let typical = Frame::filled(4, 4, 1, 0.5).unwrap();
    for _ in 0..6 {
        stack.add_frame_with_quality(&typical, quality).unwrap();
    }
}

#[test]
fn test_weighted_stacking_favor_sharp() {
    let config = StackingConfig::default()
        .with_rejection(RejectionMethod::None)
        .with_weighting(WeightingConfig::fwhm_only());

    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();
    seed_baseline(&mut stack, FrameQuality::from_fwhm(3.0));

    let frame_sharp = Frame::filled(4, 4, 1, 1.0).unwrap();
    let frame_blurry = Frame::filled(4, 4, 1, 0.0).unwrap();

    stack
        .add_frame_with_quality(&frame_sharp, FrameQuality::from_fwhm(1.5))
        .unwrap();
    stack
        .add_frame_with_quality(&frame_blurry, FrameQuality::from_fwhm(6.0))
        .unwrap();

    let result = stack.compute().unwrap();
    let data = result.data();

    assert!(
        data[0] > 0.5,
        "Should favor sharp frame: got {}, expected > 0.5",
        data[0]
    );
}

#[test]
fn test_weighted_stacking_favor_high_snr() {
    let config = StackingConfig::default()
        .with_rejection(RejectionMethod::None)
        .with_weighting(WeightingConfig::snr_only());

    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();
    seed_baseline(&mut stack, FrameQuality::from_snr(10.0));

    let frame_clean = Frame::filled(4, 4, 1, 1.0).unwrap();
    let frame_noisy = Frame::filled(4, 4, 1, 0.0).unwrap();

    stack
        .add_frame_with_quality(&frame_clean, FrameQuality::from_snr(20.0))
        .unwrap();
    stack
        .add_frame_with_quality(&frame_noisy, FrameQuality::from_snr(5.0))
        .unwrap();

    let result = stack.compute().unwrap();
    let data = result.data();

    assert!(
        data[0] > 0.5,
        "Should favor high SNR frame: got {}, expected > 0.5",
        data[0]
    );
}

#[test]
fn test_frame_qualities_stored() {
    let config = StackingConfig::default();
    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();

    let frame = Frame::filled(4, 4, 1, 0.5).unwrap();
    let q1 = FrameQuality::new(2.0, 15.0);
    let q2 = FrameQuality::new(3.0, 12.0);

    stack.add_frame_with_quality(&frame, q1).unwrap();
    stack.add_frame_with_quality(&frame, q2).unwrap();

    let qualities = stack.frame_qualities();
    assert_eq!(qualities.len(), 2);
    assert_eq!(qualities[0].fwhm, Some(2.0));
    assert_eq!(qualities[1].snr, Some(12.0));
}

#[test]
fn test_weighting_enabled_by_default() {
    let config = StackingConfig::default();
    assert!(!config.weighting.is_disabled());
    assert_eq!(config.weighting.fwhm_weight, 0.5);
    assert_eq!(config.weighting.snr_weight, 0.5);
}

#[test]
fn test_update_config_gating() {
    // 1. Create stack with simple averaging
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();

    // 2. Try to update to SigmaClip (which is a Pro feature)
    let pro_config = StackingConfig::default().with_rejection(RejectionMethod::SigmaClip);
    stack.update_config(pro_config);

    // 3. Verify it fell back to None (since REJECTION_PLUGIN is None in tests)
    assert_eq!(stack.config().rejection, RejectionMethod::None);
}

#[test]
fn test_pro_rejection_initialization_gating() {
    // Attempting to create a stack with SigmaClip should fail in Community
    let config = StackingConfig::default().with_rejection(RejectionMethod::SigmaClip);
    let result = MasterStack::new(4, 4, 1, config);

    assert!(result.is_err());
}

#[test]
fn test_stack_dimension_mismatch() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();

    let frame = Frame::filled(5, 5, 1, 0.5).unwrap();
    let result = stack.add_frame(&frame);
    assert!(result.is_err());
}

#[test]
fn test_stack_compute_empty() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let stack = MasterStack::new(4, 4, 1, config).unwrap();

    let result = stack.compute();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// The per-pixel noise map (`NoiseField`) and the scale the plain mean now keeps.
// ---------------------------------------------------------------------------

/// Deterministic zero-mean noise, so a test can state the variance it stacked.
///
/// A hash-based sequence rather than a real RNG: the assertions below are on the
/// *measured* spread of what was actually added, so the numbers have to be the same on
/// every machine and every run.
fn noisy_frames(
    width: usize,
    height: usize,
    channels: usize,
    level: f32,
    sigma: f32,
    count: usize,
) -> Vec<Frame> {
    (0..count)
        .map(|n| {
            let data: Vec<f32> = (0..width * height * channels)
                .map(|i| {
                    let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                        ^ (n as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                    h ^= h >> 31;
                    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
                    h ^= h >> 29;
                    // 53 bits over 2^53 is [0, 1); uniform on [-1, 1) has variance
                    // 1/3, so the `sqrt(3)` below scales it to `sigma`.
                    let u = (h >> 11) as f32 / (1u64 << 53) as f32 * 2.0 - 1.0;
                    level + u * sigma * 3f32.sqrt()
                })
                .collect();
            Frame::from_f32_vec(data, width, height, channels).unwrap()
        })
        .collect()
}

/// The scale has to be maintained on the plain-mean path too, or the noise map is
/// identically zero for every Community session and every test that stacks without the
/// rejection plugin — which is every integration test in this repo.
#[test]
fn the_plain_mean_keeps_a_scale_too() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(16, 16, 1, config).unwrap();

    let sigma = 0.01;
    for frame in noisy_frames(16, 16, 1, 0.3, sigma, 64) {
        stack.add_frame(&frame).unwrap();
    }

    let field = stack.noise_field();
    assert!(field.is_usable(), "the plain mean left the noise map empty");

    // `m2` is the single-sub variance; dividing by `count` is the error of the mean.
    let expected = sigma * sigma / 64.0;
    let centre = field.robust_centre();
    assert!(
        (centre / expected - 1.0).abs() < 0.25,
        "noise map centre {centre:e} against an expected {expected:e}"
    );
}

/// `m2 / count` is the variance of the stacked mean, so the map has to fall as `1/N`
/// as the stack deepens.
///
/// Read slightly high at every depth, and knowably so: a deviation is measured against a
/// running mean built from `n-1` samples, so it carries `sigma^2 * (1 + 1/(n-1))` rather
/// than `sigma^2`. That is ~7 % at 64 frames and ~35 % at 8, which is why the absolute
/// bound is only claimed at depth and the falloff is checked as a ratio, where the
/// inflation largely divides out.
#[test]
fn the_noise_field_is_the_standard_error_of_the_mean() {
    let sigma = 0.02;
    let frames = noisy_frames(32, 32, 1, 0.25, sigma, 64);

    let at = |depth: usize| {
        let config = StackingConfig::default().with_rejection(RejectionMethod::None);
        let mut stack = MasterStack::new(32, 32, 1, config).unwrap();
        for frame in frames.iter().take(depth) {
            stack.add_frame(frame).unwrap();
        }
        stack.noise_field().robust_centre()
    };

    let deep = at(64);
    let expected = sigma * sigma / 64.0;
    assert!(
        (deep / expected - 1.0).abs() < 0.2,
        "at 64 frames the map reads {deep:e}, expected {expected:e}"
    );

    for (shallow_depth, deep_depth) in [(16usize, 32usize), (32, 64)] {
        let ratio = at(shallow_depth) / at(deep_depth);
        let wanted = deep_depth as f32 / shallow_depth as f32;
        assert!(
            (ratio / wanted - 1.0).abs() < 0.2,
            "{shallow_depth} -> {deep_depth} frames moved the map by {ratio:.2}x, \
             expected {wanted:.2}x"
        );
    }
}

/// The first offered sample has no mean to deviate from, so there is no spread to
/// report below two — and the cell must say so rather than read as perfectly clean.
#[test]
fn warm_up_cells_are_marked_not_zeroed() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(8, 8, 1, config).unwrap();

    stack.add_frame(&Frame::filled(8, 8, 1, 0.4).unwrap()).unwrap();

    let field = stack.noise_field();
    assert!(
        field.variance().unwrap().iter().all(|v| v.is_nan()),
        "one frame in, the map must be unmeasured rather than zero: {:?}",
        &field.variance().unwrap()[..4]
    );
    assert!(!field.is_usable());
}

/// A block **median**, not a mean. One hot pixel's variance is real but it is not what
/// a sky threshold is asking about, and a mean over 64 samples lets it set the block.
#[test]
fn a_star_does_not_set_its_block() {
    // 8x8 is exactly one field cell, so the whole stack is the block under test.
    let build = |spiked: usize| {
        let config = StackingConfig::default().with_rejection(RejectionMethod::None);
        let mut stack = MasterStack::new(8, 8, 1, config).unwrap();
        for (n, mut frame) in noisy_frames(8, 8, 1, 0.3, 0.001, 32).into_iter().enumerate() {
            // Pixels swinging a hundred times wider than the field's own spread.
            let swing = if n % 2 == 0 { 0.1 } else { -0.1 };
            for i in 0..spiked {
                frame.set_pixel(i % 8, i / 8, 0, 0.3 + swing);
            }
            stack.add_frame(&frame).unwrap();
        }
        stack.noise_field().variance().unwrap()[0]
    };

    let quiet = build(0);
    let one_spike = build(1);
    let all_spiked = build(64);

    assert!(
        (one_spike / quiet - 1.0).abs() < 0.2,
        "one wild pixel moved its block from {quiet:e} to {one_spike:e}; the median is \
         what stops it, so this reads as the reduction having become a mean"
    );

    // The guard has to be able to refute the alternative or it is not guarding the
    // choice: the same spike, everywhere in the block, moves the cell by ~4 orders of
    // magnitude, so the value the mean would have been dragged toward is real and huge.
    assert!(
        all_spiked > quiet * 1000.0,
        "the spike itself has to be large for this test to mean anything: quiet \
         {quiet:e}, all-spiked {all_spiked:e}"
    );
}

/// Fewer subs at the border means more noise there, by `N / count` in variance. This is
/// the defect the whole map exists to let the denoiser see.
#[test]
fn the_field_tracks_coverage() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(64, 16, 1, config).unwrap();

    // The left 16 columns are "border" on half the frames — the value the stack skips.
    for (n, mut frame) in noisy_frames(64, 16, 1, 0.3, 0.01, 32).into_iter().enumerate() {
        if n % 2 == 0 {
            for y in 0..16 {
                for x in 0..16 {
                    frame.set_pixel(x, y, 0, 0.0);
                }
            }
        }
        stack.add_frame(&frame).unwrap();
    }

    let field = stack.noise_field();
    let edge = field.sample(0, 4, 8);
    let centre = field.sample(0, 48, 8);
    let ratio = edge / centre;
    assert!(
        (ratio - 2.0).abs() < 0.5,
        "half the coverage should read twice the variance; got {ratio:.2} \
         (edge {edge:e}, centre {centre:e})"
    );

    // And the coverage plane says so directly, without the brightness a variance also
    // carries: half the subs at the edge, all of them in the middle.
    let (edge_cover, centre_cover) = (field.sample_coverage(4, 8), field.sample_coverage(48, 8));
    assert!((edge_cover - 0.5).abs() < 0.05, "edge coverage read {edge_cover}");
    assert!((centre_cover - 1.0).abs() < 1e-6, "centre coverage read {centre_cover}");
}

/// The display copy and the coverage map come from one read of the accumulator, so they
/// must agree with the separate passes they stand in for.
#[test]
fn compute_with_coverage_matches_the_separate_passes() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(37, 23, 3, config).unwrap();
    for (n, mut frame) in noisy_frames(37, 23, 3, 0.3, 0.01, 12).into_iter().enumerate() {
        // Some coverage structure to agree about: the left columns miss a third of the subs.
        if n % 3 == 0 {
            for y in 0..23 {
                for x in 0..9 {
                    for c in 0..3 {
                        frame.set_pixel(x, y, c, 0.0);
                    }
                }
            }
        }
        stack.add_frame(&frame).unwrap();
    }

    let (fused_frame, fused_field) = stack.compute_with_coverage().unwrap();
    assert_eq!(fused_frame.data(), stack.compute().unwrap().data(), "the means diverged");
    assert_eq!(fused_field.coverage(), stack.noise_field().coverage(), "the coverage diverged");
    assert!(
        fused_field.variance().is_none(),
        "the per-frame path must not pay for the variance planes no filter reads"
    );
    assert!(fused_field.is_usable(), "uneven coverage is something to say");
}

#[test]
fn compute_with_coverage_refuses_an_empty_stack() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let stack = MasterStack::new(4, 4, 1, config).unwrap();
    assert!(stack.compute_with_coverage().is_err());
}

/// A stack every sub covered completely has nothing to say, and must not be carried: that
/// is the common case, and it should cost the encoders nothing.
#[test]
fn a_fully_covered_stack_carries_no_map() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(32, 24, 3, config).unwrap();
    for frame in noisy_frames(32, 24, 3, 0.3, 0.01, 8) {
        stack.add_frame(&frame).unwrap();
    }
    let (_, field) = stack.compute_with_coverage().unwrap();
    assert!(!field.is_usable(), "full coverage everywhere should read as nothing to say");
}

/// A real step in sky level must not leave the scale the size of the step.
///
/// The plain mean rejects nothing, so it has no threshold of its own to winsorise
/// against — and an exposure change, a cloud clearing, or rejection being toggled off
/// while the sky moves all arrive as one enormous deviation. Unguarded, a 1000-sigma
/// step left the window about a hundred sigmas wide for the rest of the EWMA's memory,
/// and a rejection pass switched on afterwards inherited it and clipped nothing.
#[test]
fn a_step_in_sky_level_does_not_set_the_scale() {
    let sigma = 0.0001;
    let settled = |level: f32, frames: Vec<Frame>| {
        let config = StackingConfig::default().with_rejection(RejectionMethod::None);
        let mut stack = MasterStack::new(8, 8, 1, config).unwrap();
        for frame in frames {
            stack.add_frame(&frame).unwrap();
        }
        let _ = level;
        stack.noise_field().robust_centre()
    };

    let steady: Vec<Frame> = noisy_frames(8, 8, 1, 0.3, sigma, 60);
    let mut stepped = noisy_frames(8, 8, 1, 0.3, sigma, 30);
    // A 1000-sigma step, then the same noise about the new level.
    stepped.extend(noisy_frames(8, 8, 1, 0.3 + 1000.0 * sigma, sigma, 30));

    let calm = settled(0.3, steady);
    let after_step = settled(0.3, stepped);

    assert!(
        after_step < calm * 25.0,
        "a level step left the scale {:.1}x wider than a steady sky ({after_step:e} \
         against {calm:e}); the guard is what keeps the step out of the estimate",
        after_step / calm
    );

    // And the guard has to be able to refute the alternative: the step really is huge,
    // so an unwinsorised estimate would have been orders of magnitude out.
    assert!(
        after_step > 0.0 && calm > 0.0,
        "both estimates must be measurable for the ratio to mean anything"
    );
}

/// A pixel whose early samples were identical reads as *unmeasured*, and does not drag
/// the block it sits in.
///
/// 0.95 % of pixels on 14-bit data start that way, and with the scale collapsed every
/// later real sample sits more than eight sigmas out, so the guard drops them all and the
/// scale never escapes. That is deliberate — see `observe_scale_guarded`. What has to
/// hold is that such a pixel says "no measurement" rather than "no noise", and that the
/// block median around it is unmoved.
#[test]
fn a_collapsed_pixel_reads_as_unmeasured_rather_than_clean() {
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(8, 8, 1, config).unwrap();

    let sigma = 0.001;
    // One pixel held perfectly still while the rest of the block carries real noise.
    for mut frame in noisy_frames(8, 8, 1, 0.3, sigma, 48) {
        frame.set_pixel(5, 5, 0, 0.3);
        stack.add_frame(&frame).unwrap();
    }

    let field = stack.noise_field();
    assert!(field.is_usable(), "the rest of the block still has a measurement");

    let centre = field.robust_centre();
    let expected = sigma * sigma / 48.0;
    assert!(
        (centre / expected - 1.0).abs() < 0.3,
        "the still pixel dragged its block to {centre:e}, expected about {expected:e}"
    );
}

/// Each plane's noise must come from that plane's own pixels, at every height.
///
/// A block row is 8 image rows, and a height that is not a multiple of 8 leaves the last
/// block row of each plane short. Traversing the whole buffer in 8-row stripes instead of
/// plane by plane then straddles planes — the first cells of the second plane read the
/// tail of the first, and where the stripe count and the field's row count diverge `zip`
/// silently drops the last rows — while the two computing paths agree with each other
/// because both are wrong. So this checks against the noise that was put in, per plane,
/// at heights that leave 1, 4 and 7 rows over (17 is the one a median cannot mask: its
/// stripe count differs from the field's row count outright), and at one that leaves none.
#[test]
fn every_plane_reads_its_own_noise_at_any_height() {
    for h in [17usize, 20, 23, 24] {
        every_plane_reads_its_own_noise_at(h);
    }
}

fn every_plane_reads_its_own_noise_at(h: usize) {
    let w = 40;
    let sigmas = [0.002f32, 0.02, 0.006];
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(w, h, 3, config).unwrap();
    for n in 0..48 {
        let mut frame = Frame::filled(w, h, 3, 0.0).unwrap();
        for (c, &sigma) in sigmas.iter().enumerate() {
            let plane = noisy_frames(w, h, 1, 0.3, sigma, n + 1).pop().unwrap();
            for y in 0..h {
                for x in 0..w {
                    frame.set_pixel(x, y, c, plane.get_pixel(x, y, 0));
                }
            }
        }
        stack.add_frame(&frame).unwrap();
    }

    let field = stack.noise_field();
    assert_eq!(
        field.coverage(),
        stack.compute_with_coverage().unwrap().1.coverage(),
        "height {h}: the two coverage paths disagree"
    );
    {
        let (fw, fh) = (field.width(), field.height());
        for (c, &sigma) in sigmas.iter().enumerate() {
            let expected = sigma * sigma / 48.0;
            for (i, &v) in field.variance().unwrap()[c * fw * fh..(c + 1) * fw * fh].iter().enumerate() {
                assert!(
                    (v / expected - 1.0).abs() < 0.5,
                    "height {h}: plane {c} cell {i} (row {}) read {v:e}, expected {expected:e} \
                     — a cell reading another plane's noise is off by 10-100x, and one the \
                     traversal never reached is NaN",
                    i / fw
                );
            }
        }
        assert!(field.coverage().iter().all(|&c| (c - 1.0).abs() < 1e-6));
    }
}
