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

/// A stack deepening under a fixed sky must not move the estimate.
///
/// The other sweep moves the sky and holds the noise; this holds the sky and shrinks
/// the noise, which is what a session actually does between stack updates. The
/// histogram peak alone fails it: on a real 106-sub session it returned bit-identical
/// values at 32, 64 and 106 frames, 1.2 ADU above the sky, having jumped 2.1 ADU
/// between 16 and 32 subs while the sky moved 0.2 — four output levels of background
/// step in one update, which at the eyepiece reads as the field pumping.
#[test]
fn the_sky_estimate_holds_still_as_a_stack_deepens() {
    const LEVEL: f32 = 0.0023;
    // 10 ADU down to 1.3, i.e. a single sub through to a ~60-frame stack.
    const SIGMAS: [f32; 6] = [1.53e-4, 1.08e-4, 7.6e-5, 5.4e-5, 3.0e-5, 2.0e-5];

    let mut estimates = Vec::new();
    for sigma in SIGMAS {
        estimates.push(estimate_background_mode(&sky_frame(192, 192, LEVEL, sigma)).mode);
    }

    let worst_step = estimates
        .windows(2)
        .map(|p| (p[1] - p[0]).abs())
        .fold(0.0f32, f32::max);
    let worst_err = estimates
        .iter()
        .map(|e| (e - LEVEL).abs())
        .fold(0.0f32, f32::max);
    println!(
        "estimates (ADU): {:?}",
        estimates.iter().map(|e| e * 65535.0).collect::<Vec<_>>()
    );
    assert!(
        worst_step * 65535.0 < 0.7,
        "the estimate moved {:.2} ADU between two depths of the same sky",
        worst_step * 65535.0
    );
    assert!(
        worst_err * 65535.0 < 1.0,
        "the estimate sat {:.2} ADU off the sky it was given",
        worst_err * 65535.0
    );
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

/// A sky with a broad target over `share` of its width, rising to `peak_excess` above
/// sky at the far edge. Smooth, so its samples are continuous with the sky's — which is
/// what makes it dangerous to a window sized by a MAD over everything.
fn target_over_sky(width: usize, height: usize, sky: f32, sigma: f32, share: f32, peak_excess: f32) -> Frame {
    let mut state = 0xA076_1D64_78BD_642Fu64;
    let mut next = || {
        let mut sum = 0.0f32;
        for _ in 0..12 {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            sum += (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 16_777_216.0;
        }
        sum - 6.0
    };
    let cut = (width as f32 * (1.0 - share)) as usize;
    let mut data = vec![0.0f32; width * height * 3];
    let plane = width * height;
    for c in 0..3 {
        for y in 0..height {
            for x in 0..width {
                let level = if x < cut {
                    sky
                } else {
                    sky + peak_excess * (x - cut) as f32 / (width - cut).max(1) as f32
                };
                data[c * plane + y * width + x] = (level + next() * sigma).max(0.0);
            }
        }
    }
    Frame::from_f32_vec(data, width, height, 3).unwrap()
}

/// A target filling most of the frame must not drag the sky estimate up with it.
///
/// The histogram passes already handle this — the sky is the densest bin whatever else
/// is in frame. What does not is sizing the refinement window from a MAD over *every*
/// sample: a MAD is robust while the contaminant is a minority, and a frame-filling
/// halo is not, so the window swallows the target and its median lands inside it.
/// Measured before the spread was taken from the samples below the peak: +7.25 ADU at
/// 60 % and +349 ADU at 75 %, against a raw histogram peak 1.4 ADU off the sky.
///
/// Swept rather than sampled at one share, because the failure only appears once the
/// target passes half the frame and then grows fast.
#[test]
fn a_frame_filling_target_does_not_drag_the_sky_estimate() {
    const SKY: f32 = 0.0023;
    const SIGMA: f32 = 3.0e-5;
    // 7x sky at the far edge: bright enough to be a target, smooth enough that its
    // samples run continuously out of the sky's own distribution.
    const PEAK_EXCESS: f32 = 7.0 * SKY;

    let mut worst = (0.0f32, 0.0f32);
    for share in [0.0f32, 0.2, 0.35, 0.5, 0.6, 0.75] {
        let frame = target_over_sky(256, 256, SKY, SIGMA, share, PEAK_EXCESS);
        let err = estimate_background_mode(&frame).mode - SKY;
        println!("target over {:.0}% of the frame: {:+.2} ADU", share * 100.0, err * 65535.0);
        if err.abs() > worst.0.abs() {
            worst = (err, share);
        }
    }

    assert!(
        worst.0.abs() * 65535.0 < 0.5,
        "the sky estimate moved {:+.2} ADU with a target over {:.0}% of the frame — the \
         refinement window is being sized by the target, not by the sky",
        worst.0 * 65535.0,
        worst.1 * 100.0
    );
}

/// The window is sized from a one-sided spread, so the constant that turns it into a
/// sigma has to be right: a half-normal's median is `0.6745 * sigma`, the same as a MAD.
/// A sky with no target in it is where that is checkable against the sigma it was built
/// with.
#[test]
fn the_window_is_scaled_to_one_sigma_of_the_sky_it_measures() {
    const LEVEL: f32 = 0.0023;
    for sigma in [1.0e-4f32, 3.0e-5, 1.5e-5] {
        let frame = sky_frame(256, 256, LEVEL, sigma);
        let err = (estimate_background_mode(&frame).mode - LEVEL).abs();
        assert!(
            err < 0.25 * sigma,
            "sigma {:.2} ADU: estimate {:.2} ADU off the sky, {:.2} sigma",
            sigma * 65535.0,
            err * 65535.0,
            err / sigma
        );
    }
}
