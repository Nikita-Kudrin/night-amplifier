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
    let plane = 64 * 64;
    for i in 0..plane {
        data[i] = 0.1;
        data[plane + i] = 0.2;
        data[plane * 2 + i] = 0.3;
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
    let plane = 64 * 64;
    for i in 0..plane {
        data[i] = 0.1;
        data[plane + i] = 0.2;
        data[plane * 2 + i] = 0.3;
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
    let plane = 32 * 32;
    for i in 0..plane {
        data[i] = 0.3;
        data[plane + i] = 0.3;
        data[plane * 2 + i] = 0.3;
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
    let plane = 32 * 32;
    for i in 0..plane {
        data[i] = 0.1;
        data[plane + i] = 0.2;
        data[plane * 2 + i] = 0.3;
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
    let plane = 64 * 64;
    for i in 0..plane {
        for c in 0..3 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.02;
            data[c * plane + i] = 0.2 + noise;
        }
    }
    let idx = 32 * 64 + 32;
    data[idx] = 0.9;
    data[plane + idx] = 0.9;
    data[plane * 2 + idx] = 0.9;

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
    let plane = 64 * 64;
    for i in 0..plane {
        for c in 0..3 {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.1;
            data[c * plane + i] = 0.3 + noise;
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

/// Deterministic Gaussian-ish sky, so the assertions below are reproducible.
fn sky_frame(width: usize, height: usize, level: f32, sigma: f32) -> Frame {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        // xorshift64*, summed in twelves for an approximately normal deviate.
        let mut sum = 0.0f32;
        for _ in 0..12 {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            sum += (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 16_777_216.0;
        }
        sum - 6.0
    };
    let mut data = vec![0.0f32; width * height * 3];
    for v in data.iter_mut() {
        *v = (level + next() * sigma).max(0.0);
    }
    Frame::from_f32_vec(data, width, height, 3).unwrap()
}

/// The sky estimate must track a background far narrower than a histogram bin, at every
/// level — not just at the two the first version of this test happened to sample.
///
/// A 71-frame stack has a sky sigma around 3.4e-5 of full scale (2.2 ADU of a 16-bit
/// frame) against 2.4e-4 bins (16 ADU). Two failure modes live in here and only a sweep
/// finds both:
///
/// * reporting the bin centre makes the mode a step function of stack depth, so the black
///   point (`mode - k * sigma`) jumps 16 ADU against a target only 30 ADU above sky;
/// * the five-wide smoothing turns that narrow sky into a plateau, and taking its first
///   strict maximum puts `peak_bin` two bins low — which the refinement cannot recover
///   from, because its window no longer contains the samples.
///
/// The second only shows at the sky levels where the distribution sits wholly inside one
/// bin, which is 2 positions in 21. Sweeping a whole bin in tenths is what catches it.
#[test]
fn background_mode_tracks_a_sky_narrower_than_a_histogram_bin() {
    // 2.0e-5 is 1.3 ADU, the sky sigma measured on a 71-frame stack. The value matters:
    // the plateau failure only exists while the whole distribution fits inside one bin,
    // so a wider sigma (3.4e-5, say) sweeps clean and pins nothing.
    const SIGMA: f32 = 2.0e-5;
    const BIN: f32 = 1.0 / 4095.0;
    /// Two bins of travel, in tenths. One bin is enough to cross a boundary, but two
    /// confirms the behaviour repeats rather than being one lucky alignment.
    const STEPS: usize = 20;

    let mut worst_err = 0.0f32;
    let mut worst_at = 0.0f32;
    let mut backward = 0;
    let mut previous = f32::NEG_INFINITY;

    for step in 0..=STEPS {
        let level = 10.0 * BIN + step as f32 * BIN / 10.0;
        let mode = estimate_background_mode(&sky_frame(192, 192, level, SIGMA)).mode;

        let err = (mode - level).abs();
        if err > worst_err {
            worst_err = err;
            worst_at = level;
        }
        // The sky only ever moves up across this sweep, so the estimate must too.
        if mode < previous - 1e-7 {
            backward += 1;
        }
        previous = mode;
    }

    assert!(
        worst_err < 2.0 * SIGMA,
        "worst error {:.2} ADU (at a sky of {:.2} ADU) is {:.1} sigma — the estimate is \
         not tracking the sky",
        worst_err * 65535.0,
        worst_at * 65535.0,
        worst_err / SIGMA
    );
    assert_eq!(
        backward, 0,
        "the estimate went backwards {backward} times while the sky rose monotonically"
    );
}
