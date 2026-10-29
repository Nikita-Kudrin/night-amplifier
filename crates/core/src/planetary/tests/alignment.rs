use super::common::{create_planetary_frame, create_test_frame, small_planet_frame, textured_frame, with_noise};
use crate::planetary::alignment::{align_counted, climb_from};
use std::sync::atomic::{AtomicU64, Ordering};
use super::*;

/// `(dx, dy, ncc)` of `frame` against `reference`, through fresh pyramids.
fn compute(reference: &Frame, frame: &Frame, roi: &AlignmentRoi, radius: usize, subpixel_factor: usize) -> (f32, f32, f32) {
    let (rp, fp) = pyramids(reference, frame);
    let found = align_on(&rp, &fp, roi, Search::within(radius), subpixel_factor);
    (found.dx, found.dy, found.ncc)
}

fn align_on(reference: &LuminancePyramid, frame: &LuminancePyramid, roi: &AlignmentRoi, search: Search, subpixel_factor: usize) -> Alignment {
    align(&AlignmentReference::new(reference, roi), frame, search, subpixel_factor)
}

#[test]
fn test_alignment() {
    let reference = create_test_frame(64, 64, 0.0, 0.0);
    let shifted = create_test_frame(64, 64, 5.0, 3.0);

    let roi = AlignmentRoi::centered(64, 64, 32);
    let (dx, dy, _) = compute(&reference, &shifted, &roi, 10, 1);

    assert!(
        (dx - (-5.0)).abs() < 2.0,
        "X offset should be ~-5, got {}",
        dx
    );
    assert!(
        (dy - (-3.0)).abs() < 2.0,
        "Y offset should be ~-3, got {}",
        dy
    );
}

#[test]
fn test_alignment_with_planetary_disk() {
    let reference = create_planetary_frame(64, 64, 0.0, 0.0, 0.0);
    let shifted = create_planetary_frame(64, 64, 3.0, -2.0, 0.0);

    let roi = AlignmentRoi::centered(64, 64, 40);
    let (dx, dy, _) = compute(&reference, &shifted, &roi, 10, 2);

    assert!(
        (dx - (-3.0)).abs() < 1.5,
        "X offset should be ~-3, got {}",
        dx
    );
    assert!((dy - 2.0).abs() < 1.5, "Y offset should be ~2, got {}", dy);
}

#[test]
fn test_alignment_roi_centered() {
    let roi = AlignmentRoi::centered(100, 80, 30);
    assert_eq!(roi.x, 35);
    assert_eq!(roi.y, 25);
    assert_eq!(roi.width, 30);
    assert_eq!(roi.height, 30);
}

#[test]
fn test_alignment_roi_clamped() {
    let roi = AlignmentRoi::centered(50, 50, 100);
    assert_eq!(roi.width, 50);
    assert_eq!(roi.height, 50);
}

#[test]
fn test_subpixel_alignment_accuracy() {
    let reference = create_planetary_frame(64, 64, 0.0, 0.0, 0.0);
    let shifted = create_planetary_frame(64, 64, 2.5, 1.3, 0.0);
    let roi = AlignmentRoi::centered(64, 64, 40);
    let (dx_sub, dy_sub, _) = compute(&reference, &shifted, &roi, 10, 2);
    let (dx_int, dy_int, _) = compute(&reference, &shifted, &roi, 10, 1);
    assert!((dx_sub - (-2.5)).abs() < 1.0 || (dx_int - (-2.5)).abs() < 1.0);
    assert!((dy_sub - (-1.3)).abs() < 1.0 || (dy_int - (-1.3)).abs() < 1.0);
}

#[test]
fn test_alignment_with_custom_roi() {
    let reference = create_planetary_frame(64, 64, 0.0, 0.0, 0.0);
    let shifted = create_planetary_frame(64, 64, 4.0, 2.0, 0.0);
    let roi = AlignmentRoi::new(10, 10, 30, 30);
    let (dx, dy, _) = compute(&reference, &shifted, &roi, 15, 1);
    assert!(
        (dx - (-4.0)).abs() < 3.0,
        "X offset should be ~ -4, got {}",
        dx
    );
    assert!(
        (dy - (-2.0)).abs() < 3.0,
        "Y offset should be ~ -2, got {}",
        dy
    );
}

/// A window lying inside a flat-topped blob has nothing to correlate: the frames'
/// centroids answer, and say so.
#[test]
fn test_alignment_centroid_fallback() {
    let blob = |x0: usize, y0: usize| {
        let mut frame = Frame::zeros(64, 64, 1).unwrap();
        for y in y0..y0 + 24 {
            for x in x0..x0 + 24 {
                frame.set_pixel(x, y, 0, 0.2);
            }
        }
        frame
    };
    let (rp, fp) = pyramids(&blob(20, 20), &blob(30, 25));
    let found = align_on(&rp, &fp, &AlignmentRoi::new(24, 24, 16, 16), Search::within(15), 1);
    assert_eq!(found.fit, Fit::Featureless);
    assert!((found.dx + 10.0).abs() < 1.0 && (found.dy + 5.0).abs() < 1.0, "{found:?}");
}

/// A window at the frame's left edge, content moved 10 px further left: the old search
/// clamped every window left of the frame onto column 0 and reported the first of them, -50.
#[test]
fn a_window_at_the_frame_edge_reports_its_own_shift() {
    let reference = create_planetary_frame(256, 256, -60.0, 0.0, 0.0);
    let shifted = create_planetary_frame(256, 256, -70.0, 0.0, 0.0);
    let roi = AlignmentRoi::new(10, 64, 96, 128);
    let (dx, dy, ncc) = compute(&reference, &shifted, &roi, 50, 1);
    assert!((dx - 10.0).abs() < 1.0 && dy.abs() < 1.0, "dx {dx} dy {dy} ncc {ncc}");
}

fn pyramids(reference: &Frame, frame: &Frame) -> (LuminancePyramid, LuminancePyramid) {
    (LuminancePyramid::of(reference), LuminancePyramid::of(frame))
}

/// Every offset in `±radius` at full resolution, the search the pyramid replaced.
fn exhaustive(reference: &Frame, frame: &Frame, roi: &AlignmentRoi, radius: i32) -> (i32, i32) {
    let (r, f) = (crate::planetary::frame_to_luminance(reference), crate::planetary::frame_to_luminance(frame));
    let w = reference.width();
    let n = (roi.width * roi.height) as f64;
    let stats = |lum: &[f32], x0: usize, y0: usize| {
        let values: Vec<f64> = (0..roi.height)
            .flat_map(|y| (0..roi.width).map(move |x| (x, y)))
            .map(|(x, y)| f64::from(lum[(y0 + y) * w + x0 + x]))
            .collect();
        let mean = values.iter().sum::<f64>() / n;
        let std = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
        (values, mean, std)
    };
    let (rv, rm, rs) = stats(&r, roi.x, roi.y);
    let offsets: Vec<(i32, i32)> = (-radius..=radius).flat_map(|dy| (-radius..=radius).map(move |dx| (dx, dy))).collect();
    use rayon::prelude::*;
    let best = offsets
        .par_iter()
        .map(|&(dx, dy)| {
            let (fv, fm, fs) = stats(&f, (roi.x as i32 + dx) as usize, (roi.y as i32 + dy) as usize);
            let cross: f64 = rv.iter().zip(&fv).map(|(a, b)| (a - rm) * (b - fm)).sum();
            (dx, dy, cross / (n * rs * fs))
        })
        .reduce_with(|a, b| if b.2 > a.2 { b } else { a })
        .unwrap();
    (-best.0, -best.1)
}

/// The pyramid follows the coarse winner down; on textured frames it must land where a
/// search of every offset does, through every level it can use.
#[test]
fn the_pyramid_finds_what_an_exhaustive_search_does() {
    let (w, h) = (800, 800);
    let reference = textured_frame(w, h, (0.0, 0.0), 7);
    let roi = AlignmentRoi::centered(w, h, 400);
    for (i, &(sx, sy)) in [(0.0, 0.0), (3.0, -2.0), (-17.0, 11.0), (29.0, 28.0), (-30.0, -9.0), (12.5, -21.25)]
        .iter()
        .enumerate()
    {
        let frame = with_noise(&textured_frame(w, h, (sx, sy), 7), 0.01, i as u64);
        let (rp, fp) = pyramids(&reference, &frame);
        let found = align_on(&rp, &fp, &roi, Search::within(30), 1);
        let oracle = exhaustive(&reference, &frame, &roi, 30);
        assert_eq!((found.dx as i32, found.dy as i32), oracle, "shift ({sx}, {sy}): {found:?}");
    }
}

#[test]
fn shifts_at_the_edge_of_the_radius_are_found() {
    let (w, h) = (1024, 1024);
    let reference = textured_frame(w, h, (0.0, 0.0), 3);
    let roi = AlignmentRoi::centered(w, h, 512);
    for (sx, sy) in [(49.0, -47.0), (-49.0, 47.0), (50.0, 0.0)] {
        let frame = textured_frame(w, h, (sx, sy), 3);
        let (rp, fp) = pyramids(&reference, &frame);
        let found = align_on(&rp, &fp, &roi, Search::within(50), 2);
        assert!((found.dx + sx).abs() < 0.3 && (found.dy + sy).abs() < 0.3, "shift ({sx}, {sy}): {found:?}");
    }
}

/// A 40 px disc in the default 1504 px window of a 3008 px frame is 5 px on the level the
/// search starts on: still the only structure there.
#[test]
fn a_small_planet_in_the_default_window_is_found() {
    let (w, h) = (3008, 3008);
    let reference = small_planet_frame(w, h, (0.0, 0.0));
    let frame = small_planet_frame(w, h, (23.0, -17.5));
    let (rp, fp) = pyramids(&reference, &frame);
    let found = align_on(&rp, &fp, &AlignmentRoi::centered(w, h, w / 2), Search::within(50), 2);
    assert!((found.dx + 23.0).abs() < 0.3 && (found.dy - 17.5).abs() < 0.3, "{found:?}");
}

#[test]
fn sensor_noise_does_not_move_the_answer() {
    let (w, h) = (800, 800);
    let reference = with_noise(&textured_frame(w, h, (0.0, 0.0), 11), 0.05, 100);
    let frame = with_noise(&textured_frame(w, h, (6.4, -3.7), 11), 0.05, 101);
    let (rp, fp) = pyramids(&reference, &frame);
    let found = align_on(&rp, &fp, &AlignmentRoi::centered(w, h, 400), Search::within(50), 2);
    assert!((found.dx + 6.4).abs() < 0.25 && (found.dy - 3.7).abs() < 0.25, "{found:?}");
}

#[test]
fn the_answer_does_not_depend_on_the_thread_count() {
    let (w, h) = (800, 800);
    let reference = textured_frame(w, h, (0.0, 0.0), 5);
    let frame = with_noise(&textured_frame(w, h, (8.3, 4.1), 5), 0.02, 9);
    let (rp, fp) = pyramids(&reference, &frame);
    let roi = AlignmentRoi::centered(w, h, 400);
    let many = align_on(&rp, &fp, &roi, Search::within(50), 2);
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| align_on(&rp, &fp, &roi, Search::within(50), 2));
    assert_eq!(many, one);
}

/// The 2026-10-24 Moon's search at its size: the exhaustive one compared 101^2 windows
/// of 1504^2 pixels, 134 s a frame on the Pi. A regression to it fails here, not on a clock,
/// and so does one to the fixed ±2 refinement boxes, which compared 45 windows to the
/// climb's 19.
#[test]
fn the_default_search_compares_under_one_percent_of_an_exhaustive_one() {
    let (w, h) = (3008, 3008);
    let reference = create_planetary_frame(w, h, 0.0, 0.0, 0.0);
    let frame = create_planetary_frame(w, h, 0.4, 1.2, 0.0);
    let (rp, fp) = pyramids(&reference, &frame);
    let compared = AtomicU64::new(0);
    let roi = AlignmentRoi::centered(w, h, w / 2);
    let found = align_counted(&AlignmentReference::new(&rp, &roi), &fp, Search::within(50), 2, &compared);
    let window = 1504u64 * 1504;
    let compared = compared.load(Ordering::Relaxed);
    assert!(compared * 100 < 101 * 101 * window, "compared {compared}");
    assert!(compared < 24 * window, "compared {:.1} windows", compared as f64 / window as f64);
    assert!((found.dx + 0.4).abs() < 0.3 && (found.dy + 1.2).abs() < 0.3, "{found:?}");
}

/// Pro's alignment points search a few pixels around the frame's global shift.
#[test]
fn a_search_around_an_expected_shift_finds_it_beyond_its_own_radius() {
    let (w, h) = (512, 512);
    let reference = textured_frame(w, h, (0.0, 0.0), 21);
    let frame = textured_frame(w, h, (31.0, -24.0), 21);
    let (rp, fp) = pyramids(&reference, &frame);
    let roi = AlignmentRoi::centered(w, h, 64);
    let around = align_on(&rp, &fp, &roi, Search { expected: (-29.0, 22.0), radius: 5 }, 1);
    assert_eq!((around.dx, around.dy), (-31.0, 24.0), "{around:?}");
    let near_zero = align_on(&rp, &fp, &roi, Search::within(5), 1);
    assert_ne!((near_zero.dx, near_zero.dy), (-31.0, 24.0));
}

/// A window asked for at (or past) the frame's edge keeps clear of it, so a frame that has
/// not moved is a match its neighbours can be compared with, not one on the frame's edge.
#[test]
fn the_window_keeps_clear_of_the_frame_edges() {
    let (w, h) = (600, 400);
    let reference = textured_frame(w, h, (0.0, 0.0), 17);
    let (rp, fp) = pyramids(&reference, &with_noise(&reference, 0.01, 3));
    let edge = AlignmentReference::new(&rp, &AlignmentRoi::new(0, 0, 300, 200));
    assert_eq!(edge.window(), AlignmentRoi::new(16, 16, 284, 184));
    let whole = AlignmentReference::new(&rp, &AlignmentRoi::new(0, 0, w, h));
    assert_eq!(whole.window(), AlignmentRoi::new(16, 16, w - 32, h - 32));
    let small = AlignmentReference::new(&LuminancePyramid::of(&Frame::zeros(64, 40, 1).unwrap()), &AlignmentRoi::new(0, 0, 64, 40));
    assert_eq!(small.window(), AlignmentRoi::new(8, 5, 48, 30), "an eighth of a short side");
    for reference in [&edge, &whole] {
        let found = align(reference, &fp, Search::within(20), 2);
        assert_eq!(found.fit, Fit::Correlated, "{:?}: {found:?}", reference.window());
        assert!(found.dx.abs() < 0.3 && found.dy.abs() < 0.3, "{found:?}");
    }
}

/// The last step of a search climbs from the coarser level's doubled winner; a start a few
/// pixels off still reaches the peak, and one farther than the climb's steps says so.
#[test]
fn a_refinement_climbs_to_the_peak_and_says_when_it_ran_out() {
    let (w, h) = (512, 512);
    let reference = textured_frame(w, h, (0.0, 0.0), 23);
    let (rp, fp) = pyramids(&reference, &textured_frame(w, h, (-6.0, 4.0), 23));
    let window = AlignmentReference::new(&rp, &AlignmentRoi::centered(w, h, 256));
    assert_eq!(climb_from(&window, &fp, (-4, 2), 20), Some((-6, 4, true)), "two steps off");
    let (x, y, summit) = climb_from(&window, &fp, (1, 4), 20).expect("a peak");
    assert_eq!((x, y, summit), (-4, 4, false), "seven steps off");
}

#[test]
fn a_pyramid_halves_each_level_down_to_the_coarsest_useful_one() {
    let pyramid = LuminancePyramid::from_luminance(vec![0.5; 3008 * 3007], 3008, 3007);
    assert_eq!(pyramid.level_sizes(), [(3008, 3007), (1504, 1503), (752, 751), (376, 375), (188, 187)]);
    let small = LuminancePyramid::from_luminance(vec![0.5; 150 * 150], 150, 150);
    assert_eq!(small.level_sizes(), [(150, 150)]);

    let lum: Vec<f32> = (0..(193 * 193)).map(|i| (i % 193 + 3 * (i / 193)) as f32).collect();
    let binned = LuminancePyramid::from_luminance(lum, 193, 193);
    assert_eq!(binned.level_sizes()[1], (96, 96));
    // (0,0),(1,0),(0,1),(1,1) of `x + 3y`: 0, 1, 3, 4.
    assert_eq!(binned.level(1)[0], 2.0);
}

/// Every fit the session acts on: it stacks `Correlated` and `Featureless`, and refuses a
/// match on the search's or the frame's edge, or none at all.
#[test]
fn each_fit_says_what_the_shift_rests_on() {
    let (w, h) = (600, 600);
    let reference = textured_frame(w, h, (0.0, 0.0), 13);
    let roi = AlignmentRoi::centered(w, h, 300);
    let fit_for = |shift: (f32, f32), radius: usize| {
        let (rp, fp) = pyramids(&reference, &textured_frame(w, h, shift, 13));
        align_on(&rp, &fp, &roi, Search::within(radius), 2)
    };
    assert_eq!(fit_for((7.0, -4.0), 20).fit, Fit::Correlated);
    assert_eq!(fit_for((0.0, 30.0), 20).fit, Fit::SearchEdge, "past the search");
    assert_eq!(fit_for((0.0, 155.0), 200).fit, Fit::SearchEdge, "past what the frame holds");
    let unrelated = {
        let (rp, fp) = pyramids(&reference, &textured_frame(w, h, (0.0, 0.0), 14));
        align_on(&rp, &fp, &roi, Search::within(20), 2)
    };
    assert_eq!(unrelated.fit, Fit::Uncorrelated, "{unrelated:?}");

    let flat = Frame::filled(w, h, 1, 0.3).unwrap();
    let (rp, fp) = pyramids(&flat, &flat);
    assert_eq!(align_on(&rp, &fp, &roi, Search::within(20), 2).fit, Fit::Featureless);
}

/// The NCC of the reference's `roi` against the frame's window moved by `offset`, two-pass
/// in f64: what `align`'s one-pass f32 sums must agree with.
fn oracle_ncc(reference: &Frame, frame: &Frame, roi: &AlignmentRoi, offset: (i32, i32)) -> f64 {
    let (r, f) = (crate::planetary::frame_to_luminance(reference), crate::planetary::frame_to_luminance(frame));
    let w = reference.width();
    let window = |lum: &[f32], x0: usize, y0: usize| -> Vec<f64> {
        (0..roi.height).flat_map(|y| (0..roi.width).map(move |x| (x, y))).map(|(x, y)| f64::from(lum[(y0 + y) * w + x0 + x])).collect()
    };
    let (rv, fv) = (window(&r, roi.x, roi.y), window(&f, (roi.x as i32 + offset.0) as usize, (roi.y as i32 + offset.1) as usize));
    let n = rv.len() as f64;
    let (rm, fm) = (rv.iter().sum::<f64>() / n, fv.iter().sum::<f64>() / n);
    let (rs, fs) = (
        (rv.iter().map(|v| (v - rm).powi(2)).sum::<f64>() / n).sqrt(),
        (fv.iter().map(|v| (v - fm).powi(2)).sum::<f64>() / n).sqrt(),
    );
    rv.iter().zip(&fv).map(|(a, b)| (a - rm) * (b - fm)).sum::<f64>() / (n * rs * fs)
}

/// A smooth, low-contrast surface (0.07 % RMS on a 0.9 mean: a bright disc's flat
/// middle): the window's variance is a sliver of its mean squared, more than one-pass f32
/// sums of `x` and `x²` resolve. The shift and its NCC must be a two-pass f64 search's.
#[test]
fn a_low_contrast_surface_correlates_as_a_two_pass_f64_ncc_does() {
    let (w, h) = (1024, 1024);
    let roi = AlignmentRoi::centered(w, h, 512);
    let surface = |shift| {
        let mut frame = textured_frame(w, h, shift, 7);
        frame.data_mut().iter_mut().for_each(|v| *v = 0.9 + (*v - 0.4) * 0.01);
        frame
    };
    let (reference, frame) = (surface((0.0, 0.0)), surface((3.0, -2.0)));
    let (rp, fp) = pyramids(&reference, &frame);
    let found = align_on(&rp, &fp, &roi, Search::within(20), 1);
    assert_eq!(exhaustive(&reference, &frame, &roi, 20), (-3, 2), "the oracle");
    assert_eq!((found.dx, found.dy), (-3.0, 2.0), "{found:?}");
    let exact = oracle_ncc(&reference, &frame, &roi, (3, -2));
    assert!((f64::from(found.ncc) - exact).abs() < 1e-3, "ncc {} against {exact}", found.ncc);
}
