use super::*;

const SKY: f32 = 0.052;

/// Deterministic N(0, 1).
fn gaussians(n: usize, mut seed: u32) -> Vec<f32> {
    let mut uniform = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 + 1e-7
    };
    (0..n)
        .map(|_| {
            let (u1, u2) = (uniform(), uniform());
            (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
        })
        .collect()
}

/// A grey sky at `SKY` with the post-stretch grain the field render measured.
fn noisy_sky(width: usize, height: usize) -> Vec<f32> {
    gaussians(width * height, 0x51ed_270b)
        .into_iter()
        .flat_map(|z| [(SKY * (1.0 + 0.35 * z)).max(0.0); 3])
        .collect()
}

fn levels(rgb: &[f32], pedestal: f32) -> Vec<f32> {
    rgb.chunks_exact(3)
        .map(|px| (255.0 * (pedestal + (1.0 - pedestal) * px[1])).round())
        .collect()
}

fn weber(levels: &[f32]) -> f32 {
    let n = levels.len() as f32;
    let mean = levels.iter().sum::<f32>() / n;
    let var = levels.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n;
    var.sqrt() / mean
}

/// The eye reads contrast as a ratio, and at a pixel-resolving eyepiece (1.7 arcmin
/// per pixel for 1440 px over 70 mm at 100 mm) grain is what it sees. Darkening
/// must not make it relatively stronger: the knee this replaced took it 1.68x, with
/// 20-35 % of sky pixels flattened onto the pedestal.
#[test]
fn a_darker_sky_does_not_carry_relatively_stronger_grain() {
    const PEDESTAL: f32 = 0.006; // stage_config's DARKENED_FLOOR_PEDESTAL
    let (w, h) = (400, 400);
    let plain = noisy_sky(w, h);
    let shadow = SkyShadow::from_sky(0.035 / SKY, SKY).unwrap();
    let mut darkened = plain.clone();
    apply_sky_shadow_interleaved(&mut darkened, w, h, shadow, &mut vec![], &mut vec![]);

    let (before, after) = (levels(&plain, 0.0), levels(&darkened, PEDESTAL));
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    assert!(
        mean(&after) < mean(&before) * 0.8,
        "the sky only went from {:.1} to {:.1} levels",
        mean(&before),
        mean(&after)
    );
    assert!(
        weber(&after) <= weber(&before) * 1.1,
        "relative sky grain rose {:.2}x ({:.3} -> {:.3})",
        weber(&after) / weber(&before),
        weber(&before),
        weber(&after)
    );
    let floor = (255.0 * PEDESTAL).round() + 1.0;
    let pinned = after.iter().filter(|&&v| v <= floor).count() as f32 / after.len() as f32;
    assert!(pinned < 0.05, "{:.1} % of the sky sat on the pedestal", pinned * 100.0);
}

/// Coherent structure above the sky keeps its level while the sky around it
/// darkens — what separates this from dimming the whole image.
#[test]
fn structure_above_the_sky_keeps_its_level() {
    let (w, h) = (64, 64);
    let mut rgb = vec![SKY; w * h * 3];
    for y in 24..40 {
        for x in 24..40 {
            rgb[(y * w + x) * 3..][..3].fill(2.5 * SKY);
        }
    }
    let shadow = SkyShadow::from_sky(1.0, SKY).unwrap();
    apply_sky_shadow_interleaved(&mut rgb, w, h, shadow, &mut vec![], &mut vec![]);

    let at = |x: usize, y: usize| rgb[(y * w + x) * 3 + 1];
    assert!((at(32, 32) - 2.5 * SKY).abs() < 1e-6, "patch centre moved to {}", at(32, 32));
    assert!((at(4, 4) - SKY * shadow.gain).abs() < 1e-6, "sky is {}", at(4, 4));
}

#[test]
fn stars_and_white_are_untouched() {
    // A bright sky puts the shoulder end above a lone core's 3x3 mean, so only
    // the pixel's own excess can protect it.
    let shadow = SkyShadow::from_sky(10.0, 0.15).unwrap();
    let (w, h) = (9, 9);
    let mut rgb = vec![0.15; w * h * 3];
    rgb[(4 * w + 4) * 3..][..3].fill(0.9);
    apply_sky_shadow_interleaved(&mut rgb, w, h, shadow, &mut vec![], &mut vec![]);
    assert_eq!(rgb[(4 * w + 4) * 3], 0.9, "a lone star core was dimmed");
    assert!((shadow.gain - (1.0 - MAX_DARKENING)).abs() < 1e-6);
}

/// The solver's anchor can miss the rendered sky; the kernel must not care.
#[test]
fn a_misplaced_anchor_still_takes_the_full_gain() {
    let (w, h) = (200, 200);
    let plain = noisy_sky(w, h);
    let mut darkened = plain.clone();
    // Moderate darkening: at a 2-3 level sky, 8-bit rounding alone raises grain.
    let wrong = SkyShadow::from_sky(0.6, SKY / 1.3).unwrap();
    apply_sky_shadow_interleaved(&mut darkened, w, h, wrong, &mut vec![], &mut vec![]);
    let (before, after) = (levels(&plain, 0.0), levels(&darkened, 0.0));
    assert!(weber(&after) <= weber(&before) * 1.1);
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    assert!(mean(&after) < mean(&before) * (wrong.gain + 0.1));
}

#[test]
fn the_multiplier_is_monotone_and_bounded_for_any_sky() {
    for sky in [0.01f32, SKY, 0.3, 0.6] {
        let shadow = SkyShadow::from_sky(1.0, sky).unwrap();
        let mut previous = 0.0;
        for i in 0..=1000 {
            let m = shadow.multiplier(i as f32 / 1000.0);
            assert!(m >= previous && m <= 1.0, "sky {sky}: {m} at {i}");
            previous = m;
        }
        assert_eq!(shadow.multiplier(1.0), 1.0, "sky {sky}: white darkened");
    }
}

#[test]
fn nothing_to_darken_resolves_to_none() {
    assert!(SkyShadow::from_sky(0.0, SKY).is_none());
    assert!(SkyShadow::from_sky(-1.0, SKY).is_none());
    assert!(SkyShadow::from_sky(1.0, 0.0).is_none());
    assert!(SkyShadow::from_sky(1.0, f32::NAN).is_none());
}

/// The encoder and `auto_stretch_frame` apply this at different layouts; one
/// slider position has to mean one image on both.
#[test]
fn planar_and_interleaved_agree() {
    let (w, h) = (37, 23);
    let interleaved = noisy_sky(w, h);
    let mut frame = Frame::zeros(w, h, 3).unwrap();
    for (i, px) in interleaved.chunks_exact(3).enumerate() {
        for c in 0..3 {
            frame.set_pixel(i % w, i / w, c, px[c] * (1.0 + c as f32 * 0.1));
        }
    }
    let mut rgb = vec![0.0f32; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                rgb[(y * w + x) * 3 + c] = frame.get_pixel(x, y, c);
            }
        }
    }
    let shadow = SkyShadow::from_sky(1.0, SKY).unwrap();
    apply_sky_shadow_interleaved(&mut rgb, w, h, shadow, &mut vec![], &mut vec![]);
    apply_sky_shadow_frame(&mut frame, shadow).unwrap();
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                assert!((rgb[(y * w + x) * 3 + c] - frame.get_pixel(x, y, c)).abs() < 1e-6);
            }
        }
    }
}

/// Measured as the median of the guide, a faint target covering most of the frame
/// *became* the sky and was darkened like it: a 1.6-sky nebula over 60 % of the frame
/// kept 50 % of its level, where a correct sky keeps the shoulder's ~80 %.
#[test]
fn a_frame_filling_target_is_not_taken_for_the_sky() {
    let (w, h) = (200, 200);
    let noise = gaussians(w * h, 99);
    let mut rgb = vec![0.0f32; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let level = if x < 80 { SKY } else { 1.6 * SKY };
            rgb[(y * w + x) * 3..][..3].fill(level * (1.0 + 0.35 * noise[y * w + x]));
        }
    }
    let shadow = SkyShadow::from_sky(0.035 / SKY, SKY).unwrap();
    let mut out = rgb.clone();
    apply_sky_shadow_interleaved(&mut out, w, h, shadow, &mut vec![], &mut vec![]);
    let mean = |buf: &[f32], x0: usize, x1: usize| {
        let v: Vec<f32> = (20..180)
            .flat_map(|y| (x0..x1).map(move |x| buf[(y * w + x) * 3]))
            .collect();
        v.iter().sum::<f32>() / v.len() as f32
    };
    let kept = mean(&out, 100, 190) / mean(&rgb, 100, 190);
    let expected = shadow.multiplier(1.6 * SKY);
    assert!(
        kept >= expected - 0.1,
        "the nebula kept {kept:.2} of its level; with the sky found it keeps ~{expected:.2}"
    );
}

/// Guide-like samples: `share` of them sky at `SKY`, the rest at `other * SKY`, both
/// with the 3x3 guide's ~0.12 relative spread.
fn two_populations(share: f32, other: f32) -> Vec<f32> {
    let noise = gaussians(65_536, 7);
    noise
        .iter()
        .enumerate()
        .map(|(i, z)| {
            let level = if (i as f32) < share * 65_536.0 { SKY } else { other * SKY };
            level * (1.0 + 0.12 * z)
        })
        .collect()
}

#[test]
fn the_sky_is_found_on_a_sky_dominated_frame() {
    let sky = estimate_sky(&two_populations(0.9, 3.0), 0.0).unwrap();
    assert!((sky / SKY - 1.0).abs() < 0.03, "sky {sky} against {SKY}");
}

#[test]
fn a_target_over_most_of_the_frame_is_not_the_sky() {
    let sky = estimate_sky(&two_populations(0.4, 1.6), 0.0).unwrap();
    assert!((sky / SKY - 1.0).abs() < 0.03, "sky {sky} against {SKY}");
}

/// A drifting stack leaves a registration border at one dark level: a tall spike
/// holding a few per cent of the samples, below the real sky.
#[test]
fn a_dark_border_is_not_the_sky() {
    let mut samples = two_populations(1.0, 1.0);
    samples.iter_mut().take(2_000).for_each(|v| *v = 0.1 * SKY);
    let sky = estimate_sky(&samples, 0.0).unwrap();
    assert!((sky / SKY - 1.0).abs() < 0.03, "sky {sky} against {SKY}");
}

/// A region darker than the sky over a third of the frame (roof, tree, dew shadow) is a
/// qualifying peak too; taken as the sky (0.3x), the real sky sat past the shoulder and
/// was not darkened at all. The anchor here is off by the 1.3x the solver has missed by.
#[test]
fn a_large_dark_foreground_is_not_the_sky() {
    // 35 % at 0.3 sky, 65 % at the sky.
    let samples: Vec<f32> = two_populations(0.35, 1.0 / 0.3).iter().map(|v| v * 0.3).collect();
    for anchor in [SKY / 1.3, SKY, SKY * 1.3] {
        let sky = SkyShadow::from_sky(1.0, anchor).unwrap().with_measured_sky(&samples).sky;
        assert!((sky / SKY - 1.0).abs() < 0.05, "anchor {anchor}: sky {sky} against {SKY}");
    }
}

/// End to end: the open sky beside a dark foreground takes the full gain.
#[test]
fn the_sky_beside_a_dark_foreground_is_darkened() {
    let (w, h) = (200, 200);
    let noise = gaussians(w * h, 3);
    let mut rgb = vec![0.0f32; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let level = if y < 70 { 0.3 * SKY } else { SKY };
            rgb[(y * w + x) * 3..][..3].fill((level * (1.0 + 0.35 * noise[y * w + x])).max(0.0));
        }
    }
    let shadow = SkyShadow::from_sky(1.0, SKY).unwrap();
    let mut out = rgb.clone();
    apply_sky_shadow_interleaved(&mut out, w, h, shadow, &mut vec![], &mut vec![]);
    let mean = |buf: &[f32]| {
        let v: Vec<f32> = (100..190).flat_map(|y| (10..190).map(move |x| buf[(y * w + x) * 3 + 1])).collect();
        v.iter().sum::<f32>() / v.len() as f32
    };
    let kept = mean(&out) / mean(&rgb);
    assert!(kept < shadow.gain + 0.1, "the open sky kept {kept:.2} of its level (gain {:.2})", shadow.gain);
}

/// >= 25 % identical samples (zeros where the black point clamped an obstruction) make the
/// 5th-25th percentile spread zero; the fallback returned the median of *all* samples, 7 %
/// under the sky at 30 % zeros and nothing from 50 %.
#[test]
fn a_zero_peak_falls_through_to_the_sky() {
    for share in [0.3f32, 0.5, 0.7] {
        let mut samples = two_populations(1.0, 1.0);
        let zeros = (samples.len() as f32 * share) as usize;
        samples.iter_mut().take(zeros).for_each(|v| *v = 0.0);
        let sky = estimate_sky(&samples, 0.0);
        assert!(
            sky.is_some_and(|s| (s / SKY - 1.0).abs() < 0.03),
            "{:.0} % zeros: sky {sky:?} against {SKY}",
            share * 100.0
        );
    }
}

/// A flat frame has no spread to bin by: its one level is the sky, unless under the floor.
#[test]
fn a_constant_frame_is_its_own_sky() {
    assert_eq!(estimate_sky(&[SKY; 64], 0.0), Some(SKY));
    assert_eq!(estimate_sky(&[SKY; 64], 2.0 * SKY), None);
    assert_eq!(estimate_sky(&[0.0; 64], 0.0), None);
}
