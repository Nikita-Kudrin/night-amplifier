use super::*;

/// Deterministic noise in [-0.5, 0.5).
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    }
}

/// A disc of `radius` about `centre` on a `(w, h)` frame over a sky of `sky`, each channel
/// times `tint`, with `noise` of uniform noise. `surface(r)` is its brightness at a share
/// `r` of the radius: a lunar surface is flat and, with `detail`, patched with maria (darker
/// patches scaled to the disc), a planet's darkens to its limb.
fn disc(
    (w, h): (usize, usize),
    centre: (f32, f32),
    radius: f32,
    surface: impl Fn(f32) -> f32,
    detail: bool,
    (sky, noise): (f32, f32),
    tint: [f32; 3],
) -> Frame {
    let mut frame = Frame::zeros(w, h, 3).unwrap();
    let mut rng = Lcg(7);
    for y in 0..h {
        for x in 0..w {
            let r = ((x as f32 - centre.0).hypot(y as f32 - centre.1)) / radius;
            let mut level = if r < 1.0 { surface(r) } else { 0.0 };
            if detail && r < 1.0 {
                let scale = radius.min(w.min(h) as f32 / 2.0);
                let (u, v) = (x as f32 / scale * 6.0, y as f32 / scale * 6.0);
                if (u.sin() * (v * 1.3).cos()) > 0.35 {
                    level *= 0.72;
                }
            }
            for (c, t) in tint.iter().enumerate() {
                frame.set_pixel(x, y, c, ((level + sky) * t + noise * rng.next()).clamp(0.0, 1.0));
            }
        }
    }
    frame
}

const GREY: [f32; 3] = [1.0, 1.0, 1.0];
/// The raw IMX533 Moon of 2026-10-24: blue at 0.49 of green, red at 0.94.
const RAW_MOON: [f32; 3] = [0.944, 1.0, 0.487];

fn flat(_: f32) -> f32 {
    0.6
}

/// Dim enough that no tint or gain below x2.2 clips it.
fn dim(_: f32) -> f32 {
    0.45
}

fn sphere(r: f32) -> f32 {
    0.7 * (1.0 - r * r).max(0.0).sqrt()
}

fn measure(frame: &Frame) -> TargetMeasure {
    TargetMeasure::of(frame, &PlanetaryLevels::solve(frame))
}

/// The verdict on `frame` as the stream shows it: rendered through `balance`.
fn seen_through(frame: &Frame, balance: &DaylightBalance) -> (TargetMeasure, Result<DaylightBalance, NotTheMoon>) {
    let mut shown = frame.clone();
    balance.apply(&mut shown);
    let m = measure(&shown);
    (m, m.moon(balance))
}

fn moon(frame: &Frame) -> Result<DaylightBalance, NotTheMoon> {
    seen_through(frame, &DaylightBalance::PRIOR).1
}

#[test]
fn a_textured_flat_disc_is_the_moon_and_implies_the_balance_that_colours_it_like_one() {
    let frame = disc((600, 500), (300.0, 250.0), 180.0, flat, true, (0.002, 0.004), RAW_MOON);
    let (m, verdict) = seen_through(&frame, &DaylightBalance::PRIOR);
    assert!(m.texture_excess() >= TEXTURE_MIN, "{m:?}");
    assert!(m.limb.is_some_and(|l| l > 0.9), "{m:?}");
    let balance = verdict.expect("the Moon");
    let expected = [MOON_COLOUR[0] / RAW_MOON[0], 1.0, MOON_COLOUR[2] / RAW_MOON[2]];
    for (got, want) in [balance.red, balance.green, balance.blue].into_iter().zip(expected) {
        assert!((got - want).abs() < 0.02, "{balance:?} against {expected:?}");
    }
    // Raw, through no balance at all, its blue is half what the Moon's can be: not the Moon.
    assert_eq!(measure(&frame).moon(&DaylightBalance::IDENTITY), Err(NotTheMoon::Coloured));
}

/// Through the balance it implied, the same Moon implies that balance again.
#[test]
fn the_implied_balance_is_a_fixed_point() {
    let frame = disc((600, 500), (300.0, 250.0), 180.0, flat, true, (0.002, 0.004), RAW_MOON);
    let first = moon(&frame).unwrap();
    let again = seen_through(&frame, &first).1.unwrap();
    for (a, b) in [(again.red, first.red), (again.green, first.green), (again.blue, first.blue)] {
        assert!((a - b).abs() < 0.01, "{again:?} vs {first:?}");
    }
}

#[test]
fn a_limb_darkened_disc_is_a_planet() {
    let jupiter = disc((600, 500), (300.0, 250.0), 180.0, sphere, false, (0.002, 0.004), [0.6, 0.57, 0.5]);
    let m = measure(&jupiter);
    assert!(m.limb.is_some_and(|l| l < LIMB_UNDARKENED), "{m:?}");
    assert!(matches!(moon(&jupiter), Err(NotTheMoon::Smooth | NotTheMoon::LimbDarkened)), "{:?}", moon(&jupiter));
    let banded = disc((600, 500), (300.0, 250.0), 180.0, sphere, true, (0.002, 0.004), [0.6, 0.57, 0.5]);
    let (m, verdict) = seen_through(&banded, &DaylightBalance::PRIOR);
    assert_eq!(verdict, Err(NotTheMoon::LimbDarkened), "textured, yet darkening to its limb: {m:?}");
}

/// Venus, a defocused star, a twilight sky: noise is not texture.
#[test]
fn a_featureless_disc_is_smooth_however_noisy() {
    for noise in [0.0, 0.01, 0.03] {
        let venus = disc((600, 500), (300.0, 250.0), 180.0, flat, false, (0.002, noise), GREY);
        let m = measure(&venus);
        assert_eq!(m.moon(&DaylightBalance::PRIOR), Err(NotTheMoon::Smooth), "noise {noise}: {m:?}");
    }
}

#[test]
fn a_zoomed_in_moon_fills_the_frame_and_is_still_the_moon() {
    let frame = disc((400, 300), (200.0, 150.0), 600.0, flat, true, (0.0, 0.004), RAW_MOON);
    let (m, verdict) = seen_through(&frame, &DaylightBalance::PRIOR);
    assert_eq!((m.sky, m.haze), (None, 0.0), "{m:?}");
    assert!(verdict.is_ok(), "{m:?}: {verdict:?}");
}

#[test]
fn mars_an_eclipse_and_an_ice_giant_are_coloured() {
    // Each in the raw colour this camera gives it: the Moon's raw colour, moved by the
    // target's own colour against the Moon.
    let mars = disc((600, 500), (300.0, 250.0), 180.0, dim, true, (0.002, 0.004), [1.4, 1.0, 0.33]);
    assert_eq!(moon(&mars), Err(NotTheMoon::Coloured));
    let eclipse = disc((600, 500), (300.0, 250.0), 180.0, dim, true, (0.002, 0.004), [1.0, 0.3, 0.1]);
    assert_eq!(moon(&eclipse), Err(NotTheMoon::Coloured));
    let uranus = disc((600, 500), (300.0, 250.0), 180.0, dim, true, (0.002, 0.004), [0.75, 1.0, 0.7]);
    assert_eq!(moon(&uranus), Err(NotTheMoon::Coloured));
    let h_alpha = disc((600, 500), (300.0, 250.0), 180.0, dim, true, (0.002, 0.004), [1.0, 0.2, 0.1]);
    assert_eq!(moon(&h_alpha), Err(NotTheMoon::Coloured), "past x4 even where the ratios pass");
}

#[test]
fn too_little_target_clipping_and_mono_are_refused_first() {
    let speck = disc((600, 500), (300.0, 250.0), 30.0, flat, true, (0.002, 0.004), RAW_MOON);
    assert_eq!(moon(&speck), Err(NotTheMoon::TooSmall));
    let clipped = disc((600, 500), (300.0, 250.0), 180.0, |_| 1.0, true, (0.002, 0.004), [0.6, 0.6, 0.52]);
    assert_eq!(moon(&clipped), Err(NotTheMoon::Clipped), "the highlands clip, the maria do not");
    let all_clipped = disc((600, 500), (300.0, 250.0), 180.0, |_| 1.2, true, (0.002, 0.004), GREY);
    assert_eq!(moon(&all_clipped), Err(NotTheMoon::TooSmall), "nothing unclipped to average");
    let colour = disc((600, 500), (300.0, 250.0), 180.0, flat, true, (0.002, 0.004), GREY);
    let mono = Frame::from_f32_vec(colour.channel_data(1).to_vec(), 600, 500, 1).unwrap();
    assert_eq!(moon(&mono), Err(NotTheMoon::Mono));
    let nothing = Frame::zeros(64, 64, 3).unwrap();
    assert_eq!(moon(&nothing), Err(NotTheMoon::TooSmall));
}

/// The same Moon at a third the focal length scores the same shape.
#[test]
fn the_shape_measures_do_not_depend_on_the_discs_size_in_pixels() {
    let large = measure(&disc((1200, 1000), (600.0, 500.0), 450.0, flat, true, (0.002, 0.004), GREY));
    let small = measure(&disc((400, 340), (200.0, 170.0), 150.0, flat, true, (0.002, 0.004), GREY));
    assert!((large.texture_excess() - small.texture_excess()).abs() < 0.3 * large.texture_excess(), "{large:?}\n{small:?}");
    assert!((large.limb.unwrap() - small.limb.unwrap()).abs() < 0.1, "{large:?}\n{small:?}");
}

/// `frame` with `sky(x, y)` added to each channel everywhere, the disc included: a pedestal, a
/// twilight sky or a glow lies in front of the target as much as beside it.
fn under(mut frame: Frame, sky: impl Fn(usize, usize) -> [f32; 3]) -> Frame {
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            for (c, s) in sky(x, y).into_iter().enumerate() {
                let v = frame.get_pixel(x, y, c);
                frame.set_pixel(x, y, c, (v + s).min(1.0));
            }
        }
    }
    frame
}

/// The balance a raw Moon implies is the one that renders it in `MOON_COLOUR`.
fn assert_true_balance(balance: &DaylightBalance, what: &str) {
    let expected = [MOON_COLOUR[0] / RAW_MOON[0], 1.0, MOON_COLOUR[2] / RAW_MOON[2]];
    for (got, want) in [balance.red, balance.green, balance.blue].into_iter().zip(expected) {
        assert!((got / want - 1.0).abs() < 0.01, "{what}: {balance:?} against {expected:?}");
    }
}

/// A Moon at a seventh to a half of full scale over a pedestal of 0.3-1.2 % per channel: the
/// pedestal times the prior's blue gain once read as haze, and its black biased blue 1-2.5 %.
#[test]
fn a_dim_moon_over_a_pedestal_is_the_moon_in_its_own_colour() {
    for (peak, pedestal) in [(0.15, 0.003), (0.15, 0.006), (0.2, 0.006), (0.2, 0.012), (0.3, 0.012), (0.5, 0.012)] {
        let moon = disc((600, 500), (300.0, 250.0), 180.0, |_| peak, true, (0.0, 0.01 * peak), RAW_MOON);
        let frame = under(moon, |_, _| [pedestal; 3]);
        let (m, verdict) = seen_through(&frame, &DaylightBalance::PRIOR);
        let what = format!("peak {peak}, pedestal {pedestal}: {m:?}");
        assert_true_balance(&verdict.unwrap_or_else(|why| panic!("{why:?}, {what}")), &what);
    }
}

/// A flat sky is taken off whatever its tint, twilight's blue included, as far as it lies under
/// the target's threshold (`PlanetaryLevels::target_from`, 0.06 here): past it, it is target.
#[test]
fn a_flat_sky_is_taken_off_the_colour_whatever_its_tint() {
    for sky in [[0.02, 0.02, 0.02], [0.004, 0.008, 0.02], [0.01, 0.012, 0.022]] {
        let frame = under(disc((600, 500), (300.0, 250.0), 180.0, dim, true, (0.0, 0.004), RAW_MOON), |_, _| sky);
        let (m, verdict) = seen_through(&frame, &DaylightBalance::PRIOR);
        let what = format!("sky {sky:?}: {m:?}");
        assert_true_balance(&verdict.unwrap_or_else(|why| panic!("{why:?}, {what}")), &what);
    }
}

/// A glow brightening towards the disc lies over it more than its median sky says: refused.
#[test]
fn a_sky_glowing_towards_the_disc_is_haze() {
    let glow = |x: usize, y: usize| {
        let r = (x as f32 - 300.0).hypot(y as f32 - 250.0) / 180.0;
        [0.08 * (-(r - 1.0).max(0.0) / 0.4).exp(); 3]
    };
    let frame = under(disc((600, 500), (300.0, 250.0), 180.0, dim, true, (0.0, 0.004), RAW_MOON), glow);
    let (m, verdict) = seen_through(&frame, &DaylightBalance::PRIOR);
    assert_eq!(verdict, Err(NotTheMoon::HazySky), "{m:?}");
}

/// Read noise is not haze: a sub's noise averages out of the sky's blocks.
#[test]
fn a_noisy_flat_sky_is_not_haze() {
    for noise in [0.004, 0.03] {
        let frame = disc((600, 500), (300.0, 250.0), 180.0, flat, true, (0.006, noise), RAW_MOON);
        let (m, verdict) = seen_through(&frame, &DaylightBalance::PRIOR);
        assert!(m.haze < 0.005 && verdict != Err(NotTheMoon::HazySky), "noise {noise}: {m:?}: {verdict:?}");
    }
}

/// The size gates count sensor pixels: binned by 4, a Moon of the least diameter (38 px as
/// measured) is the Moon and a planet that size still darkens to its limb (0.43-0.47 from 38 to
/// 150 px), so no floor in measured pixels is needed. A speck is still refused.
#[test]
fn the_size_gates_count_sensor_pixels_through_the_bin() {
    let at_bin_4 = |frame: Frame| {
        let mut shown = frame;
        DaylightBalance::PRIOR.apply(&mut shown);
        let m = measure(&shown);
        (m, m.binned(4).moon(&DaylightBalance::PRIOR))
    };
    let (m, verdict) = at_bin_4(disc((300, 250), (150.0, 125.0), 19.0, flat, true, (0.002, 0.004), RAW_MOON));
    assert_eq!(m.moon(&DaylightBalance::PRIOR), Err(NotTheMoon::TooSmall), "unbinned: {m:?}");
    assert_true_balance(&verdict.unwrap_or_else(|why| panic!("{why:?}: {m:?}")), "binned by 4");
    let (m, verdict) = at_bin_4(disc((300, 250), (150.0, 125.0), 19.0, sphere, true, (0.002, 0.004), [0.6, 0.57, 0.5]));
    assert_eq!(verdict, Err(NotTheMoon::LimbDarkened), "{m:?}");
    let speck = disc((300, 250), (150.0, 125.0), 15.0, flat, true, (0.002, 0.004), RAW_MOON);
    assert_eq!(moon(&speck), Err(NotTheMoon::TooSmall));
}
