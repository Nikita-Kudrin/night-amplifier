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
#[ignore = "Requires Pro rejection plugin"]
fn test_master_stack_sigma_clip() {
    // Use ACTUAL SigmaClip, and set min_frames_for_rejection to 3
    // so it has enough history to build a standard deviation before frame 5
    let config = StackingConfig::default()
        .with_rejection(RejectionMethod::SigmaClip)
        .with_sigma(2.0); // 2.0 standard deviations

    let mut config = config;
    config.min_frames_for_rejection = 3;

    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();

    // 1. Establish the "baseline" running average and variance
    for _ in 0..4 {
        let frame = Frame::filled(4, 4, 1, 0.5).unwrap();
        stack.add_frame(&frame).unwrap();
    }

    // 2. Introduce the outlier
    let frame_outlier = Frame::filled(4, 4, 1, 0.9).unwrap();
    stack.add_frame(&frame_outlier).unwrap();

    let result = stack.compute().unwrap();
    let data = result.data();

    // The running Sigma Clip should detect 0.9 as an outlier and reject it instantly.
    // The running mean remains exactly 0.5.
    assert!(
        (data[0] - 0.5).abs() < 0.01,
        "SigmaClip should reject the outlier and give ~0.5, got {}",
        data[0]
    );
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

#[test]
fn test_weighted_stacking_favor_sharp() {
    let config = StackingConfig::default()
        .with_rejection(RejectionMethod::None)
        .with_weighting(WeightingConfig::fwhm_only());

    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();

    let frame_sharp = Frame::filled(4, 4, 1, 0.8).unwrap();
    let frame_blurry = Frame::filled(4, 4, 1, 0.2).unwrap();

    stack
        .add_frame_with_quality(&frame_sharp, FrameQuality::from_fwhm(2.0))
        .unwrap();
    stack
        .add_frame_with_quality(&frame_blurry, FrameQuality::from_fwhm(5.0))
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

    let frame_clean = Frame::filled(4, 4, 1, 0.8).unwrap();
    let frame_noisy = Frame::filled(4, 4, 1, 0.2).unwrap();

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
#[ignore = "Requires Pro rejection plugin"]
fn test_weighted_stacking_with_sigma_clip() {
    let config = StackingConfig::default()
        .with_rejection(RejectionMethod::SigmaClip)
        .with_weighting(WeightingConfig::balanced());

    let mut stack = MasterStack::new(4, 4, 1, config).unwrap();

    for i in 0..4 {
        let frame = Frame::filled(4, 4, 1, 0.5).unwrap();
        let fwhm = 2.0 + i as f32 * 0.1;
        stack
            .add_frame_with_quality(&frame, FrameQuality::from_fwhm(fwhm))
            .unwrap();
    }

    let outlier = Frame::filled(4, 4, 1, 0.9).unwrap();
    stack
        .add_frame_with_quality(&outlier, FrameQuality::from_fwhm(5.0))
        .unwrap();

    let result = stack.compute().unwrap();
    let data = result.data();

    assert!(
        (data[0] - 0.5).abs() < 0.1,
        "Should reject outlier: got {}, expected ~0.5",
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
