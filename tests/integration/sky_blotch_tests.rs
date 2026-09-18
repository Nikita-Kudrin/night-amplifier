//! Colour blotches: the defect the chroma denoiser used to add to the sky it was
//! cleaning.
//!
//! The guided filter's regularisation was a constant `1e-4` in linear light, while a
//! real sky's guide variance is ~1e-9 and a faint star's ~1e-8. Every window read as
//! flat, so the filter became a ~40 px box blur of chroma and spread each star's colour
//! into a halo that size. Measured in output levels on real stacks, 32-64 px chroma
//! noise came out 2-4.6x *higher* with the filter on than with it off — at the scale a
//! dark-adapted eye is most sensitive to, and made more visible by the fine grain the
//! filter had just removed. `ChromaDenoiseConfig::noise_k` scales it to the guide's own
//! noise instead.
//!
//! So the filter has two things to prove at every depth, and this asserts both: fine
//! chroma noise falls (it is doing its job) and coarse chroma noise does not rise (it is
//! not making blotches).

use serial_test::serial;

use crate::integration::stack_depth_grain_tests::{managed_session, render, stack_snapshots};

const FIXTURE_SET: &str = "250mm-dob-imx533-dumbbell-fits";
/// The two sets the halos measured worst on: the 106-sub M27 (4.6x coarse chroma) and
/// the globular the defect was reported at the eyepiece on.
const DEEP_SET: &str = "deep-stack-dumbbell-106";
const GLOBULAR_SET: &str = "globular-cluster-eyepiece";

/// B3 spline, the à trous kernel — the same basis the luma denoiser works in, so
/// "level 5" means the same size structure in both.
const B3: [f32; 5] = [0.0625, 0.25, 0.375, 0.25, 0.0625];

/// One à trous smoothing step with `hole - 1` zeros between taps, mirrored at the edges.
fn smooth(src: &[f32], width: usize, height: usize, hole: usize) -> Vec<f32> {
    let reflect = |v: isize, n: usize| -> usize {
        let n = n as isize;
        let v = if v < 0 { -v } else { v };
        let v = if v >= n { 2 * n - v - 2 } else { v };
        v.clamp(0, n - 1) as usize
    };
    let mut rows = vec![0.0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut acc = 0.0;
            for (t, w) in B3.iter().enumerate() {
                let sx = reflect(x as isize + (t as isize - 2) * hole as isize, width);
                acc += w * src[y * width + sx];
            }
            rows[y * width + x] = acc;
        }
    }
    let mut out = vec![0.0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut acc = 0.0;
            for (t, w) in B3.iter().enumerate() {
                let sy = reflect(y as isize + (t as isize - 2) * hole as isize, height);
                acc += w * rows[sy * width + x];
            }
            out[y * width + x] = acc;
        }
    }
    out
}

/// Standard deviation of each à trous detail level over `mask`, in the plane's own units.
fn band_sigmas(plane: &[f32], mask: &[bool], width: usize, height: usize, levels: usize) -> Vec<f64> {
    let mut coarse = plane.to_vec();
    let mut out = Vec::with_capacity(levels);
    for level in 0..levels {
        let next = smooth(&coarse, width, height, 1 << level);
        let detail: Vec<f32> = coarse.iter().zip(next.iter()).map(|(c, n)| c - n).collect();
        let kept: Vec<f64> = detail
            .iter()
            .zip(mask.iter())
            .filter(|(_, &m)| m)
            .map(|(&v, _)| v as f64)
            .collect();
        let mean = kept.iter().sum::<f64>() / kept.len() as f64;
        out.push((kept.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / kept.len() as f64).sqrt());
        coarse = next;
    }
    out
}

/// Luma and the two chroma planes of an interleaved RGB8 buffer, in output levels.
fn ycbcr(rgb8: &[u8]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let n = rgb8.len() / 3;
    let (mut y, mut cb, mut cr) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for (i, px) in rgb8.chunks_exact(3).enumerate() {
        let luma = 0.2126 * px[0] as f32 + 0.7152 * px[1] as f32 + 0.0722 * px[2] as f32;
        y[i] = luma;
        cb[i] = px[2] as f32 - luma;
        cr[i] = px[0] as f32 - luma;
    }
    (y, cb, cr)
}

/// Sky: pixels within a few levels of the median, with anything brighter grown by 4 px
/// so a star's own wings are outside the measurement.
fn sky_mask(luma: &[f32], width: usize, height: usize) -> Vec<bool> {
    // Thresholded on a blurred copy, not on the pixels: at one sub the sky's own grain
    // is several output levels, so a per-pixel threshold calls most of the sky a star
    // and leaves nothing to measure.
    let blurred = smooth(&smooth(luma, width, height, 1), width, height, 2);
    let mut sorted: Vec<f32> = blurred.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];

    let bright: Vec<bool> = blurred.iter().map(|&v| v > median + 2.0).collect();
    let mut mask = vec![true; width * height];
    for y in 0..height {
        for x in 0..width {
            if !bright[y * width + x] {
                continue;
            }
            for dy in y.saturating_sub(4)..(y + 5).min(height) {
                for dx in x.saturating_sub(4)..(x + 5).min(width) {
                    mask[dy * width + dx] = false;
                }
            }
        }
    }
    // The registered border is only partly covered, so keep well inside it.
    for y in 0..height {
        for x in 0..width {
            if x < 64 || y < 64 || x + 64 >= width || y + 64 >= height {
                mask[y * width + x] = false;
            }
        }
    }
    mask
}

struct Chroma {
    fine: f64,
    coarse: f64,
}

/// Fine (2-4 px) and coarse (32-64 px) chroma noise of a render, in output levels.
fn chroma_bands(rgb8: &[u8], width: usize, height: usize, mask: &[bool]) -> Chroma {
    let (_, cb, cr) = ycbcr(rgb8);
    let b_cb = band_sigmas(&cb, mask, width, height, 6);
    let b_cr = band_sigmas(&cr, mask, width, height, 6);
    let rms = |a: f64, b: f64| (a * a + b * b).sqrt();
    Chroma {
        fine: rms(rms(b_cb[0], b_cb[1]), rms(b_cr[0], b_cr[1])),
        coarse: rms(rms(b_cb[4], b_cb[5]), rms(b_cr[4], b_cr[5])),
    }
}

/// The two claims, at every depth of a real session.
fn assert_chroma_denoise_helps_without_blotching(label: &str, files: &[std::path::PathBuf], depths: &[usize]) {
    let settings = night_amplifier::server::state::CaptureSettings::default();
    let snapshots = stack_snapshots(files, depths);

    println!("\n=== {label}: chroma noise in output levels (fine 2-4 px / coarse 32-64 px) ===");
    println!("   N   denoise off        default            fine ratio  coarse ratio");
    for (n, stack) in &snapshots {
        let (plain, w, h) = render(stack.clone(), &settings, false, (2560, 1440), *n as u32);
        let (filtered, _, _) = render(stack.clone(), &settings, true, (2560, 1440), *n as u32);

        let (luma, _, _) = ycbcr(&plain);
        let mask = sky_mask(&luma, w, h);
        let sky_fraction = mask.iter().filter(|&&m| m).count() as f64 / (w * h) as f64;
        assert!(
            sky_fraction > 0.02,
            "{label} at {n} subs: only {:.1}% of the frame is sky, nothing to measure",
            sky_fraction * 100.0
        );

        let off = chroma_bands(&plain, w, h, &mask);
        let on = chroma_bands(&filtered, w, h, &mask);
        let fine_ratio = on.fine / off.fine;
        let coarse_ratio = on.coarse / off.coarse;
        println!(
            "{n:>4}   {:>5.2} / {:<5.2}      {:>5.2} / {:<5.2}      {fine_ratio:>10.2}  {coarse_ratio:>12.2}",
            off.fine, off.coarse, on.fine, on.coarse
        );

        assert!(
            fine_ratio < 0.5,
            "{label} at {n} subs: colour denoising only took fine chroma noise to \
             {fine_ratio:.2}x — the filter is not doing its job"
        );
        assert!(
            coarse_ratio < 1.25,
            "{label} at {n} subs: colour denoising *raised* 32-64 px chroma noise to \
             {coarse_ratio:.2}x of the unfiltered sky. That is star colour bleeding into \
             the background — the blotches `noise_k` exists to prevent."
        );
    }
}

/// The 35-sub set, the cheapest of the three to run.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn colour_denoising_does_not_blotch_the_sky() {
    let files = managed_session(FIXTURE_SET);
    assert_chroma_denoise_helps_without_blotching(FIXTURE_SET, &files, &[1, 8, 30]);
}

/// The two sets the halos measured worst on. Managed like every other fixture rather
/// than read from one machine's home directory, so the claim is checkable anywhere.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn colour_denoising_does_not_blotch_deep_sessions() {
    for (set, depths) in [(DEEP_SET, [1, 8, 64].as_slice()), (GLOBULAR_SET, [1, 8].as_slice())] {
        let files = managed_session(set);
        assert_chroma_denoise_helps_without_blotching(set, &files, depths);
    }
}
