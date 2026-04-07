use super::*;
use crate::error::StackError;

#[test]
fn test_black_point_config_defaults() {
    let config = BlackPointConfig::default();
    assert!((config.sigma_factor - 2.0).abs() < 1e-6);

    let conservative = BlackPointConfig::conservative();
    assert!((conservative.sigma_factor - 1.5).abs() < 1e-6);

    let aggressive = BlackPointConfig::aggressive();
    assert!((aggressive.sigma_factor - 2.5).abs() < 1e-6);
}

#[test]
fn test_calculate_black_point_formula() {
    let data = vec![0.2f32; 64 * 64 * 1];
    let frame = Frame::from_f32_vec(data, 64, 64, 1).unwrap();
    let stats = ChannelStats::new(0.2, 0.01, 0.0, 1.0);

    let bp = calculate_black_point(&frame, 0, &stats, 2.0);
    let expected = 0.2 - 2.0 * (0.01 * 1.4826);
    assert!(
        (bp - expected).abs() < 2e-4, // increased tolerance since mode calculation is binned
        "Black point mismatch: got {}, expected {}",
        bp,
        expected
    );
}

#[test]
fn test_calculate_black_point_clamps_to_zero() {
    let data = vec![0.01f32; 64 * 64 * 1];
    let frame = Frame::from_f32_vec(data, 64, 64, 1).unwrap();
    let stats = ChannelStats::new(0.01, 0.02 / 1.4826, 0.0, 1.0);

    let bp = calculate_black_point(&frame, 0, &stats, 2.0);
    assert_eq!(bp, 0.0);
}

#[test]
fn test_calculate_black_points_per_channel() {
    let mut data = vec![0.0f32; 64 * 64 * 3];
    for i in 0..(64 * 64) {
        data[i * 3] = 0.1;
        data[i * 3 + 1] = 0.2;
        data[i * 3 + 2] = 0.3;
    }
    let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
    let stats = compute_image_stats(&frame).unwrap();

    let black_points = calculate_black_points(&frame, &stats, BlackPointConfig::default()).unwrap();

    assert!(black_points[0] < black_points[1]);
    assert!(black_points[1] < black_points[2]);
}

#[test]
fn test_calculate_luminance_black_point() {
    let mut data = vec![0.0f32; 64 * 64 * 3];
    for i in 0..(64 * 64) {
        data[i * 3] = 0.1;
        data[i * 3 + 1] = 0.2;
        data[i * 3 + 2] = 0.3;
    }
    let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
    let stats = compute_image_stats(&frame).unwrap();

    let lum_bp = calculate_luminance_black_point(&frame, &stats, BlackPointConfig::default());

    // For RGB (0.1, 0.2, 0.3), luminance is 0.2126*0.1 + 0.7152*0.2 + 0.0722*0.3 = 0.18596
    assert!((lum_bp - 0.186).abs() < 0.01);
}

#[test]
fn test_subtract_black_point_basic() {
    let mut data = vec![0.0f32; 32 * 32 * 3];
    for i in 0..(32 * 32) {
        data[i * 3] = 0.3;
        data[i * 3 + 1] = 0.3;
        data[i * 3 + 2] = 0.3;
    }
    let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();

    let black_points = [0.2, 0.2, 0.2];
    subtract_black_point(&mut frame, &black_points).unwrap();

    let r = frame.get_pixel(16, 16, 0);
    let g = frame.get_pixel(16, 16, 1);
    let b = frame.get_pixel(16, 16, 2);

    assert!((r - 0.1).abs() < 1e-5);
    assert!((g - 0.1).abs() < 1e-5);
    assert!((b - 0.1).abs() < 1e-5);
}

#[test]
fn test_subtract_black_point_clamps_negative() {
    let mut data = vec![0.0f32; 32 * 32 * 3];
    for i in 0..(32 * 32) {
        data[i * 3] = 0.1;
        data[i * 3 + 1] = 0.2;
        data[i * 3 + 2] = 0.3;
    }
    let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();

    let black_points = [0.2, 0.2, 0.2];
    subtract_black_point(&mut frame, &black_points).unwrap();

    let r = frame.get_pixel(16, 16, 0);
    let g = frame.get_pixel(16, 16, 1);
    let b = frame.get_pixel(16, 16, 2);

    assert_eq!(r, 0.0);
    assert!((g - 0.0).abs() < 1e-5);
    assert!((b - 0.1).abs() < 1e-5);
}

#[test]
fn test_subtract_black_point_uniform() {
    let data = vec![0.5f32; 32 * 32 * 3];
    let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();

    subtract_black_point_uniform(&mut frame, 0.2).unwrap();

    for &v in frame.data() {
        assert!((v - 0.3).abs() < 1e-5);
    }
}

#[test]
fn test_subtract_black_point_auto() {
    let mut data = vec![0.0f32; 64 * 64 * 3];
    let mut seed: u32 = 12345;
    for i in 0..(64 * 64) {
        for c in 0..3 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.02;
            data[i * 3 + c] = 0.2 + noise;
        }
    }
    data[32 * 64 * 3 + 32 * 3] = 0.9;
    data[32 * 64 * 3 + 32 * 3 + 1] = 0.9;
    data[32 * 64 * 3 + 32 * 3 + 2] = 0.9;

    let mut frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();

    let black_points = subtract_black_point_auto(&mut frame, BlackPointConfig::default()).unwrap();

    for bp in black_points {
        assert!(bp < 0.2);
        assert!(bp > 0.1);
    }

    let bg_pixel = frame.get_pixel(0, 0, 0);
    assert!(bg_pixel < 0.05);

    let star_pixel = frame.get_pixel(32, 32, 0);
    assert!(star_pixel > 0.5);
}

#[test]
fn test_subtract_black_point_wrong_channels() {
    let mut frame = Frame::filled(10, 10, 1, 0.5).unwrap();
    let black_points = [0.1, 0.1, 0.1];

    let result = subtract_black_point(&mut frame, &black_points);
    assert!(matches!(result, Err(StackError::ChannelMismatch { .. })));
}

#[test]
fn test_sigma_factor_affects_black_point() {
    let mut data = vec![0.0f32; 64 * 64 * 3];
    let mut seed: u32 = 54321;
    for i in 0..(64 * 64) {
        for c in 0..3 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.1;
            data[i * 3 + c] = 0.3 + noise;
        }
    }
    let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
    let stats = compute_image_stats(&frame).unwrap();

    let conservative =
        calculate_black_points(&frame, &stats, BlackPointConfig::conservative()).unwrap();
    let default = calculate_black_points(&frame, &stats, BlackPointConfig::default()).unwrap();
    let aggressive =
        calculate_black_points(&frame, &stats, BlackPointConfig::aggressive()).unwrap();

    assert!(conservative[0] > aggressive[0]);
    assert!(default[0] > aggressive[0] && default[0] < conservative[0]);
}
