use super::*;
use crate::render::simd::scale_lut_lookup;

/// Deterministic noise in [-0.5, 0.5).
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    }
}

/// A limb-darkened disc of `peak` (half of it at the limb) on a sky of `sky`, both with
/// `noise` of uniform noise, `tint` times each channel; `set_pixel` so the layout is the
/// frame's own.
fn disc(size: (usize, usize), centre: (f32, f32), radius: f32, peak: f32, sky: f32, noise: f32, tint: [f32; 3]) -> Frame {
    let (w, h) = size;
    let mut frame = Frame::zeros(w, h, 3).unwrap();
    let mut rng = Lcg(7);
    for y in 0..h {
        for x in 0..w {
            let r = ((x as f32 - centre.0).hypot(y as f32 - centre.1)) / radius;
            let level = if r < 1.0 { peak * (0.5 + 0.5 * (1.0 - r * r).sqrt()) } else { 0.0 };
            for (c, t) in tint.iter().enumerate() {
                frame.set_pixel(x, y, c, ((level + sky) * t + noise * rng.next()).clamp(0.0, 1.0));
            }
        }
    }
    frame
}

const GREY: [f32; 3] = [1.0, 1.0, 1.0];

#[test]
fn a_small_disc_on_a_black_sky_spans_sky_to_its_top() {
    let levels = PlanetaryLevels::solve(&disc((400, 300), (200.0, 150.0), 40.0, 0.8, 0.002, 0.001, GREY));
    let sky = levels.sky.expect("a sky");
    assert!((sky - 0.002).abs() < 3e-4, "{levels:?}");
    assert!((levels.black - sky).abs() < 1e-6, "a frame mostly sky is taken off at its sky: {levels:?}");
    assert!(levels.white > 0.78 && levels.white < 0.81, "{levels:?}");
}

/// The full-Moon panel with no sky: its darkest pixels are maria, which must not be black.
#[test]
fn a_frame_filling_disc_keeps_the_sensors_black() {
    let levels = PlanetaryLevels::solve(&disc((300, 200), (150.0, 100.0), 400.0, 0.7, 0.0, 0.002, GREY));
    assert_eq!((levels.sky, levels.black), (None, 0.0), "{levels:?}");
    assert!(levels.white > 0.65, "{levels:?}");
}

/// Uranus on 2025-12-13: a sky at 0.029 under a 0.75 disc.
#[test]
fn a_twilight_sky_is_taken_off_whole() {
    let levels = PlanetaryLevels::solve(&disc((400, 300), (200.0, 150.0), 30.0, 0.75, 0.03, 0.001, GREY));
    assert!((levels.black - 0.03).abs() < 5e-4, "{levels:?}");
}

/// Venus or the Moon by day, Mercury in twilight: a sky at a fifth of the disc's top is still
/// sky. Above `TARGET_SHARE` of the peak it counts as target, and renders mid-grey. Open: the
/// histogram cannot tell it from a frame-filling Moon's maria at 0.3 of its highlands.
#[test]
#[ignore = "open: a sky past TARGET_SHARE of the peak reads as target; needs a daylight fixture"]
fn a_daylight_sky_under_a_bright_disc_is_taken_off() {
    let levels = PlanetaryLevels::solve(&disc((400, 300), (200.0, 150.0), 40.0, 0.5, 0.12, 0.002, GREY));
    assert!((levels.black - 0.12).abs() < 0.01, "{levels:?}");
}

/// Haze brightening the sky round a disc, a frame at a time: the sky crosses `TARGET_SHARE`
/// of the peak in one step, and the band (a share of the *frame*) must not let the black
/// point fall from the sky to 0 there.
#[test]
fn the_black_point_moves_continuously_as_the_sky_brightens_under_a_disc() {
    let mut previous: Option<PlanetaryLevels> = None;
    for step in 0..80 {
        let sky = 0.04 + step as f32 * 0.001;
        let levels = PlanetaryLevels::solve(&disc((400, 300), (200.0, 150.0), 40.0, 0.5, sky, 0.002, GREY));
        if let Some(previous) = previous {
            assert!(
                (levels.black - previous.black).abs() < 5e-3,
                "sky {sky:.3}: black {} -> {}",
                previous.black,
                levels.black
            );
        }
        previous = Some(levels);
    }
}

#[test]
fn an_underexposed_disc_is_held_to_the_least_span() {
    let levels = PlanetaryLevels::solve(&disc((400, 300), (200.0, 150.0), 40.0, 0.006, 0.001, 0.0002, GREY));
    assert!((levels.span() - MIN_SPAN).abs() < 1e-6, "{levels:?}");
}

#[test]
fn a_frame_with_nothing_in_it_still_solves() {
    for value in [0.0, f32::NAN] {
        let levels = PlanetaryLevels::solve(&Frame::filled(64, 64, 3, value).unwrap());
        assert!(levels.black.is_finite() && levels.white.is_finite(), "{value}: {levels:?}");
        assert!(levels.span() >= MIN_SPAN - 1e-6, "{value}: {levels:?}");
    }
}

/// Isolated hot pixels have dark neighbours, so however many there are, neither the peak nor
/// the white point is theirs.
#[test]
fn hot_pixels_cannot_be_the_targets_top() {
    let mut frame = disc((400, 300), (200.0, 150.0), 40.0, 0.5, 0.002, 0.001, GREY);
    for k in 0..200 {
        for c in 0..3 {
            frame.set_pixel(5 + (k % 50) * 7, 10 + (k / 50) * 9, c, 1.0);
        }
    }
    let levels = PlanetaryLevels::solve(&frame);
    assert!(levels.white < 0.51, "{levels:?}");
    assert!((levels.target_from - TARGET_SHARE * 0.5).abs() < 0.01, "{levels:?}");
}

/// Fewer pixels than one in 10 000 of the frame: a rank that scaled with the frame put the
/// threshold in the sky, and the twilight sky was never taken off.
#[test]
fn a_disc_smaller_than_a_ten_thousandth_of_its_frame_is_still_the_target() {
    let levels = PlanetaryLevels::solve(&disc((6000, 400), (3000.0, 200.0), 6.2, 0.8, 0.03, 0.001, GREY));
    assert!((levels.black - 0.03).abs() < 1e-3, "{levels:?}");
    assert!(levels.white > 0.6, "{levels:?}");
}

/// Uranus's 675 px of a 2712x1538 frame: a sampled histogram saw eight of them.
#[test]
fn a_disc_of_a_few_hundred_pixels_is_found() {
    let levels = PlanetaryLevels::solve(&disc((2000, 1000), (900.0, 400.0), 13.8, 0.8, 0.0015, 0.001, GREY));
    assert!(levels.white > 0.7, "{levels:?}");
    assert!(levels.sky.is_some(), "{levels:?}");
}

/// A Moon drifting to fill the frame: its sky shrinks through the band and the black point
/// walks from the sky to 0 with it, never in one step.
#[test]
fn the_black_point_moves_continuously_as_a_disc_fills_the_frame() {
    let mut previous: Option<PlanetaryLevels> = None;
    for step in 0..60 {
        let radius = 112.0 + step as f32 * 0.5;
        let levels = PlanetaryLevels::solve(&disc((200, 200), (100.0, 100.0), radius, 0.7, 0.003, 0.0005, GREY));
        if let Some(previous) = previous {
            assert!(
                (levels.black - previous.black).abs() < 5e-4,
                "radius {radius}: black {} -> {}",
                previous.black,
                levels.black
            );
        }
        previous = Some(levels);
    }
    let last = previous.unwrap();
    assert_eq!(last.black, 0.0, "the frame filled: {last:?}");
}

#[test]
fn a_mono_frame_solves_on_its_one_channel() {
    let colour = disc((400, 300), (200.0, 150.0), 40.0, 0.8, 0.002, 0.001, GREY);
    let mono = Frame::from_f32_vec(colour.channel_data(1).to_vec(), 400, 300, 1).unwrap();
    let levels = PlanetaryLevels::solve(&mono);
    assert!(levels.white > 0.78 && levels.white < 0.81, "{levels:?}");
}

/// The row walk against every pixel and every horizontal pair counted one by one, on rows
/// that split unevenly into tasks: no pair spans two rows, mono reads its one channel.
#[test]
fn the_histogram_counts_every_pixel_and_every_horizontal_pair_once() {
    let (w, h) = (37, 2311);
    for channels in [1, 3] {
        let mut rng = Lcg(11);
        let frame = Frame::from_f32_vec((0..w * h * channels).map(|_| rng.next() + 0.5).collect(), w, h, channels).unwrap();
        let v = |x, y| (0..channels).map(|c| frame.get_pixel(x, y, c)).fold(0.0f32, f32::max);
        let (mut counts, mut pairs) = (vec![0u32; BINS], vec![0u32; BINS]);
        for y in 0..h {
            for x in 0..w {
                counts[Counts::bin(v(x, y))] += 1;
                if x + 1 < w {
                    pairs[Counts::bin(v(x, y).min(v(x + 1, y)))] += 1;
                }
            }
        }
        let histogram = Histogram::of(&frame);
        assert_eq!((histogram.brightness.total, histogram.pairs.total), (w * h, (w - 1) * h));
        assert!(histogram.brightness.counts == counts, "{channels} channels: pixels");
        assert!(histogram.pairs.counts == pairs, "{channels} channels: pairs");
    }
}

#[test]
fn the_curve_rises_from_black_to_the_white_output_and_stops_at_one() {
    assert_eq!(curve(0.0), 0.0);
    assert!((curve(1.0) - WHITE_OUTPUT).abs() < 1e-6);
    let samples: Vec<f32> = (0..=2000).map(|i| curve(i as f32 / 1000.0)).collect();
    assert!(samples.windows(2).all(|w| w[1] >= w[0]), "monotone");
    assert!(samples.iter().all(|&v| v <= 1.0));
    assert_eq!(curve(2.0), 1.0);
}

/// The table read back through the kernel's own lookup, against the curve, in output levels.
#[test]
fn the_table_holds_the_curve_within_a_fraction_of_a_level() {
    for span in [0.02, 0.15, 0.62, 0.98] {
        let lut = scale_lut(span);
        assert_eq!(lut.basis(), ScaleBasis::MaxChannel);
        let worst = (0..4000)
            .map(|i| 1e-6f32 * (1e6f32).powf(i as f32 / 3999.0))
            .map(|v| (v * scale_lut_lookup(&lut, v) - curve(v / span)).abs() * 255.0)
            .fold(0.0f32, f32::max);
        assert!(worst < 0.15, "span {span}: {worst:.3} levels off");
    }
}

/// The stretch cache's lesson: two curves 3 % apart once shared a table.
#[test]
fn the_table_is_cached_on_its_span_and_never_shared_across_spans() {
    let a = scale_lut(0.5);
    assert!(Arc::ptr_eq(&a, &scale_lut(0.5)));
    let b = scale_lut(0.515);
    assert!(!Arc::ptr_eq(&a, &b));
    assert_ne!(a.entries(), b.entries());
}

#[test]
fn a_balance_is_normalised_to_its_smallest_gain_and_kept_in_range() {
    let balance = DaylightBalance { red: 0.5, green: 0.5, blue: 1.0 }.sanitized();
    assert_eq!(balance, DaylightBalance { red: 1.0, green: 1.0, blue: 2.0 });
    let wild = DaylightBalance { red: f32::NAN, green: -1.0, blue: 100.0 }.sanitized();
    assert_eq!(wild, DaylightBalance { red: 1.0, green: 1.0, blue: 4.0 });
    assert_eq!(DaylightBalance::IDENTITY.sanitized(), DaylightBalance::IDENTITY);
}

#[test]
fn a_highlight_clipped_at_the_sensor_stays_white_through_the_balance() {
    let mut frame = Frame::filled(8, 8, 3, 1.0).unwrap();
    frame.set_pixel(0, 0, 2, 0.25);
    DaylightBalance { red: 1.06, green: 1.0, blue: 2.07 }.apply(&mut frame);
    assert_eq!([frame.get_pixel(1, 1, 0), frame.get_pixel(1, 1, 1), frame.get_pixel(1, 1, 2)], [1.0; 3]);
    assert!((frame.get_pixel(0, 0, 2) - 0.5175).abs() < 1e-6);
}

/// The raw IMX533 Moon of 2026-10-24: blue at 0.48 of green, red at 0.94.
#[test]
fn calibrating_on_a_tinted_grey_disc_brings_it_back_to_grey() {
    const RAW: [f32; 3] = [0.943, 1.0, 0.484];
    let raw = disc((400, 300), (200.0, 150.0), 90.0, 0.7, 0.0, 0.002, RAW);
    let balance = DaylightBalance::IDENTITY.calibrated(&raw).expect("enough target");
    let mut balanced = raw.clone();
    balance.apply(&mut balanced);
    let means: Vec<f64> = (0..3)
        .map(|c| balanced.channel_data(c).iter().filter(|&&v| v > 0.1).map(|&v| f64::from(v)).sum::<f64>())
        .collect();
    for c in [0, 2] {
        assert!((means[c] / means[1] - 1.0).abs() < 0.01, "{balance:?}: {means:?}");
    }
    // Calibrating again on what that balance rendered changes nothing.
    let again = balance.calibrated(&balanced).unwrap();
    for (a, b) in [(again.red, balance.red), (again.green, balance.green), (again.blue, balance.blue)] {
        assert!((a - b).abs() < 0.01, "{again:?} vs {balance:?}");
    }
}

/// A clipped Moon under a sky, as the stream holds it: balanced, its black not yet taken off.
/// Calibration takes the sky off itself (left on, blue read x2.02) and leaves out what the
/// sensor clipped (taken off first, a clipped channel read 1 - black: blue x1.82, not x2.07).
#[test]
fn calibration_takes_the_sky_off_and_leaves_out_what_the_sensor_clipped() {
    const RAW: [f32; 3] = [0.943, 1.0, 0.484];
    const SKY: f32 = 0.02;
    let mut rendered = disc((400, 300), (200.0, 150.0), 90.0, 1.3, 0.0, 0.0005, RAW);
    for c in 0..3 {
        rendered.channel_data_mut(c).iter_mut().for_each(|v| *v = (*v + SKY).min(1.0));
    }
    assert!((PlanetaryLevels::solve(&rendered).black - SKY).abs() < 1e-3);

    let balance = DaylightBalance::IDENTITY.calibrated(&rendered).expect("an unclipped annulus remains");
    assert!((balance.red - 1.0 / 0.943).abs() < 0.01 && (balance.blue - 1.0 / 0.484).abs() < 0.02, "{balance:?}");
}

/// Greying an H-alpha disc needs gains past the range: refused, where clamping saved a balance
/// pinned at x4 that tinted every later target.
#[test]
fn a_coloured_target_is_refused_rather_than_clamped() {
    let h_alpha = disc((400, 300), (200.0, 150.0), 90.0, 0.7, 0.0, 0.002, [1.0, 0.2, 0.15]);
    assert_eq!(DaylightBalance::IDENTITY.calibrated(&h_alpha), Err(CalibrationRefusal::NotNeutral));
    let near_the_end = disc((400, 300), (200.0, 150.0), 90.0, 0.7, 0.0, 0.002, [1.0, 0.95, 0.27]);
    assert!(DaylightBalance::IDENTITY.calibrated(&near_the_end).is_ok(), "x3.7 is inside the range");
}

#[test]
fn calibration_needs_colour_and_enough_unclipped_target() {
    let none = Err(CalibrationRefusal::NoTarget);
    let mono = Frame::filled(64, 64, 1, 0.5).unwrap();
    assert_eq!(DaylightBalance::IDENTITY.calibrated(&mono), none);
    let speck = disc((400, 300), (200.0, 150.0), 10.0, 0.7, 0.002, 0.001, [0.9, 1.0, 0.5]);
    assert_eq!(DaylightBalance::IDENTITY.calibrated(&speck), none, "{} px of target", 314);
    let clipped = disc((400, 300), (200.0, 150.0), 90.0, 3.0, 0.0, 0.0, GREY);
    assert_eq!(DaylightBalance::IDENTITY.calibrated(&clipped), none, "nothing unclipped to average");
}
