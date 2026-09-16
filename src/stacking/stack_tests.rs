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
