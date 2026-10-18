//! `auto_crop` on real sessions, replayed through production: the simulated camera, the
//! raw-CFA stage and the live stack (`instruments::Replay`), then the stacking task's own
//! `StackCrop` on every snapshot, and the render task's `crop_stack` before the render.
//!
//! What the crop keeps, how often the live view's crop moves, that the cut precedes the
//! statistics the stretch is solved from, and that with the setting off nothing is cut.
//! Run with `--ignored --test-threads=1`.

use serial_test::serial;

use crate::integration::instruments::{render, Replay};
use crate::integration::stack_depth_grain_tests::managed_session;
use night_amplifier::session::capture::stack_crop::{crop_stack, StackCrop};
use night_amplifier::session::state::CaptureSettings;
use night_amplifier::stacking::{min_coverage, CropRect, KEEP_COVERAGE};
use night_amplifier::statistics::{select_median, MAD_TO_SIGMA};
use night_amplifier::{Frame, NoiseField};

/// 39 IMX533 subs of M101 that drift across the frame: a fifth of it thins out.
const DRIFTING: &str = "m101-pinwheel-galaxy-imx533";
/// 35 IMX533 subs of M33, drifting less.
const M33: &str = "m33-triangulum-galaxy-imx533";
/// The 35-sub M27 session, which barely moves.
const STEADY: &str = "250mm-dob-imx533-dumbbell-fits";
/// 64 subs of M33 cut to 1024², across which the field drifts by almost half.
const HEAVY: &str = "m33-drifting-imx533-1024";

/// One snapshot of a replayed session and the crop decided for it.
struct Snapshot {
    depth: usize,
    crop: Option<CropRect>,
    /// The lowest coverage cell inside the crop.
    min_coverage: Option<f32>,
}

/// A session played through, every snapshot's crop decided as the stacking task decides it.
struct Session {
    snapshots: Vec<Snapshot>,
    /// The last snapshot, uncropped.
    frame: Frame,
    coverage: Option<NoiseField>,
    depth: usize,
    /// The session's crop tracker after the last snapshot: what the export continues with.
    crop: StackCrop,
}

fn play(name: &str, auto_crop: bool) -> Session {
    play_shown(name, auto_crop, |_| true)
}

/// [`play`], deciding only for the subs `shown(index)` picks: the stacking task decides on
/// display frames alone, and makes none while the render task is busy.
fn play_shown(name: &str, auto_crop: bool, shown: impl Fn(usize) -> bool) -> Session {
    let dir = managed_session(name)[0].parent().unwrap().to_path_buf();
    let mut replay = Replay::open(&dir);
    let mut crop = StackCrop::new();
    let mut snapshots = Vec::new();
    let mut index = 0;
    while replay.step().is_some() {
        index += 1;
        if !shown(index - 1) {
            continue;
        }
        let (_, coverage) = replay.snapshot_with_coverage();
        let depth = replay.depth();
        let rect = crop.decide(auto_crop, depth, coverage.as_ref());
        snapshots.push(Snapshot {
            depth,
            crop: rect,
            min_coverage: rect.zip(coverage.as_ref()).and_then(|(r, c)| min_coverage(c, &r)),
        });
    }
    let (frame, coverage) = replay.snapshot_with_coverage();
    Session { snapshots, frame, coverage, depth: replay.depth(), crop }
}

fn share(rect: Option<CropRect>, frame: &Frame) -> f64 {
    rect.map_or(1.0, |r| r.area() as f64 / (frame.width() * frame.height()) as f64)
}

/// Every shown crop holds only cells at `KEEP_COVERAGE` or more.
fn assert_crops_are_covered(session: &Session, name: &str) {
    for s in &session.snapshots {
        if let Some(lowest) = s.min_coverage {
            assert!(lowest >= KEEP_COVERAGE - 1e-6, "{name} depth {}: the crop holds a {lowest:.2} cell", s.depth);
        }
    }
}

/// `(changes, largest edge step in px)` of the shown crop over the session.
fn crop_changes(session: &Session) -> (usize, usize) {
    let mut changes = 0;
    let mut largest = 0;
    for pair in session.snapshots.windows(2) {
        let (a, b) = (pair[0].crop, pair[1].crop);
        if a == b {
            continue;
        }
        changes += 1;
        if let (Some(a), Some(b)) = (a, b) {
            let step = [
                a.x.abs_diff(b.x),
                a.y.abs_diff(b.y),
                (a.x + a.width).abs_diff(b.x + b.width),
                (a.y + a.height).abs_diff(b.y + b.height),
            ];
            largest = largest.max(step.into_iter().max().unwrap());
        }
    }
    (changes, largest)
}

fn report(name: &str, session: &Session) {
    let (changes, largest) = crop_changes(session);
    let last = session.snapshots.last().unwrap();
    println!(
        "{name}: depth {}, final crop {:?} = {:.1} % of the frame, {changes} changes over {} snapshots, largest step {largest} px",
        session.depth,
        last.crop,
        100.0 * share(last.crop, &session.frame),
        session.snapshots.len()
    );
}

/// Measured when the constants were chosen; a change of rule or constant that moves them
/// should be a decision, not a side effect.
#[test]
#[ignore]
#[serial]
fn drifting_sessions_crop_to_their_covered_area() {
    let mut failures = Vec::new();
    for (name, expected) in [(DRIFTING, 0.841), (M33, 0.934), (HEAVY, 0.621)] {
        let session = play(name, true);
        report(name, &session);
        assert_crops_are_covered(&session, name);
        let kept = share(session.snapshots.last().unwrap().crop, &session.frame);
        if (kept - expected).abs() >= 0.02 {
            failures.push(format!("{name}: kept {:.1} %, expected {:.1} %", 100.0 * kept, 100.0 * expected));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
#[ignore]
#[serial]
fn a_steady_session_is_barely_cropped() {
    let session = play(STEADY, true);
    report(STEADY, &session);
    assert_crops_are_covered(&session, STEADY);
    let kept = share(session.snapshots.last().unwrap().crop, &session.frame);
    assert!(kept >= 0.97, "kept {:.1} %", 100.0 * kept);
}

/// The hysteresis: a crop picked at 0.95 and kept to 0.85 moves on at most one sub in two,
/// in steps a viewer reads as a change of framing rather than a creep. The raw best crop
/// moved on 51 of the heavy session's 64 subs.
#[test]
#[ignore]
#[serial]
fn the_live_crop_changes_rarely() {
    for name in [DRIFTING, M33, HEAVY] {
        let session = play(name, true);
        report(name, &session);
        let (changes, largest) = crop_changes(&session);
        assert!(2 * changes <= session.snapshots.len(), "{name}: {changes} changes over {} subs", session.snapshots.len());
        assert!(largest <= 96, "{name}: a {largest} px step");
    }
}

/// Robust sky level and MAD-sigma of the green plane.
fn sky_and_sigma(frame: &Frame) -> (f32, f32) {
    let mut v = frame.channel_data(1).to_vec();
    let median = night_amplifier::statistics::select_median(&mut v);
    let mut deviations: Vec<f32> = frame.channel_data(1).iter().map(|x| (x - median).abs()).collect();
    (median, select_median(&mut deviations) * MAD_TO_SIGMA)
}

fn green_percentile(rgb8: &[u8], percentile: f64) -> u8 {
    let mut green: Vec<u8> = rgb8.iter().skip(1).step_by(3).copied().collect();
    green.sort_unstable();
    green[((green.len() - 1) as f64 * percentile) as usize]
}

/// Why the crop is cut before the render rather than after it: the thin border inflates the
/// sky noise the stretch is solved from, and the curve solved on the crop alone is a
/// different, brighter one. The render task's parity with this crop-then-render is pinned
/// by `the_saved_png_of_a_cropped_stack_is_what_the_live_view_streams`.
#[test]
#[ignore]
#[serial]
fn the_border_skews_the_statistics_the_crop_removes() {
    let session = play(HEAVY, true);
    let rect = session.snapshots.last().unwrap().crop.expect("the heavy session is cropped");
    let mut cropped = session.frame.clone();
    crop_stack(&mut cropped, session.coverage.as_ref(), &rect).unwrap();

    let (_, whole_sigma) = sky_and_sigma(&session.frame);
    let (_, crop_sigma) = sky_and_sigma(&cropped);
    let inflation = whole_sigma / crop_sigma - 1.0;
    println!("whole-frame sky MAD {whole_sigma:.6} vs the crop's {crop_sigma:.6}: {:+.1} %", 100.0 * inflation);
    assert!(inflation > 0.08, "the border inflated the sky MAD by only {:+.1} %", 100.0 * inflation);

    // Background subtraction off: the claim is about the stretch, and on this frame-filling
    // galaxy the background models of the crop and the whole frame differ by their own
    // amount (the gap was 72 vs 64 without them, 61 vs 59 with them, 2026-10-18).
    let settings = CaptureSettings { background_subtraction: false, ..CaptureSettings::default() };
    let depth = session.depth as u32;
    let unbounded = (u32::MAX, u32::MAX);
    let (first, w, h) = render(cropped, &settings, false, unbounded, depth);
    assert_eq!((w, h), (rect.width, rect.height));
    let (whole, fw, _) = render(session.frame.clone(), &settings, false, unbounded, depth);
    let mut after = Vec::with_capacity(rect.area() * 3);
    for y in rect.y..rect.y + rect.height {
        after.extend_from_slice(&whole[(y * fw + rect.x) * 3..(y * fw + rect.x + rect.width) * 3]);
    }
    let (target_first, target_after) = (green_percentile(&first, 0.995), green_percentile(&after, 0.995));
    println!("target p99.5: crop then render {target_first}, render then crop {target_after}");
    assert!(
        target_first as i32 >= target_after as i32 + 3,
        "cutting first no longer brightens the target ({target_first} vs {target_after}): is the border still skewing the curve?"
    );
}

/// With the setting off nothing is cut, on the session the setting cuts most: every
/// decision is none, and what renders is the whole stack.
#[test]
#[ignore]
#[serial]
fn toggle_off_never_crops_a_drifting_session() {
    let off = play(DRIFTING, false);
    assert!(off.snapshots.iter().all(|s| s.crop.is_none()), "a crop was decided with the setting off");

    let settings = CaptureSettings::default();
    let (bytes, w, h) = render(off.frame.clone(), &settings, false, (u32::MAX, u32::MAX), off.depth as u32);
    assert_eq!((w, h), (off.frame.width(), off.frame.height()));
    assert_eq!(bytes.len(), w * h * 3);

    // The same session with it on does cut: off is the setting, not a session with nothing to cut.
    let on = play(DRIFTING, true);
    assert!(on.snapshots.last().unwrap().crop.is_some());
}

/// The export continues the live view's tracker: the saved FITS and PNG are cut where the
/// last frame shown was, and the FITS reads back at that size.
#[test]
#[ignore]
#[serial]
fn the_saved_stack_is_cropped_like_the_live_view() {
    let mut session = play(DRIFTING, true);
    let shown = session.snapshots.last().unwrap().crop.expect("the drifting session is cropped");
    let saved = session.crop.decide(true, session.depth, session.coverage.as_ref());
    assert_eq!(saved, Some(shown), "the export cut somewhere the live view did not");

    let mut frame = session.frame.clone();
    let coverage = crop_stack(&mut frame, session.coverage.as_ref(), &shown).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stack.fits");
    let metadata = night_amplifier::fits::FitsMetadata::new().with_subframe_origin(shown.x, shown.y);
    night_amplifier::fits::write_fits(&frame, &path, Some(&metadata)).unwrap();
    let read = night_amplifier::fits::read_frame(&path).unwrap();
    assert_eq!((read.width(), read.height()), (shown.width, shown.height));

    let settings = CaptureSettings { auto_crop: true, ..CaptureSettings::default() };
    let (_, w, h) = night_amplifier::session::capture::storage::render_stacked_png(
        frame,
        &settings,
        &night_amplifier::plugins::Plugins::installed(),
        session.depth as u32,
        coverage,
    )
    .unwrap();
    assert_eq!((w as usize, h as usize), (shown.width, shown.height));
}

/// The export decides on the final stack, which can be subs deeper than the last frame shown
/// when the render task was busy. It continues the live tracker: it keeps the shown rect
/// while that still holds `KEEP_COVERAGE` and no fresh pick is `GROW_SHARE` larger, and
/// whatever it saves holds `KEEP_COVERAGE` on the stack it saves.
#[test]
#[ignore]
#[serial]
fn the_saved_stack_continues_the_live_crop_after_skipped_frames() {
    use night_amplifier::stacking::{largest_covered, GROW_SHARE, PICK_COVERAGE};

    let mut session = play_shown(DRIFTING, true, |index| index % 3 == 0 && index < 36);
    let shown = session.snapshots.last().unwrap();
    assert!(session.depth > shown.depth, "the export saw no deeper stack than the last frame shown");
    let shown = shown.crop.expect("the drifting session is cropped");
    let coverage = session.coverage.as_ref().expect("a drifting stack carries its map");

    let saved = session.crop.decide(true, session.depth, Some(coverage)).expect("the export is cropped");
    assert!(min_coverage(coverage, &saved).unwrap() >= KEEP_COVERAGE - 1e-6, "saved {saved:?} holds a thin cell");
    let holds = min_coverage(coverage, &shown).unwrap() >= KEEP_COVERAGE - 1e-6;
    let outgrown = largest_covered(coverage, PICK_COVERAGE)
        .is_some_and(|fresh| fresh.area() as f64 >= shown.area() as f64 * GROW_SHARE);
    if holds && !outgrown {
        assert_eq!(saved, shown, "the export left a crop the live view could still show");
    }
    println!("last shown {shown:?} at depth {}, saved {saved:?} at {}", session.snapshots.last().unwrap().depth, session.depth);
}

/// The plan's open risk, measured: a crop change re-measures the render's estimates on a new
/// window, so the tone can step at each change. Every snapshot renders through one persistent
/// `PreviewAnalysis`, as the render task does, and the rendered sky is read over one fixed
/// patch of the stack that every crop holds, so a step is tone, not a different region.
#[test]
#[ignore]
#[serial]
fn a_crop_change_does_not_step_the_tone() {
    use night_amplifier::session::capture::pipeline::process_preview_frame_with_analysis;
    use night_amplifier::session::capture::{AnalysisContext, PreviewAnalysis};

    let dir = managed_session(HEAVY)[0].parent().unwrap().to_path_buf();
    let settings = CaptureSettings { auto_crop: true, ..CaptureSettings::default() };
    let steps = |auto_crop: bool| -> (Vec<f64>, Vec<f64>) {
        let mut replay = Replay::open(&dir);
        let mut crop = StackCrop::new();
        let mut analysis = PreviewAnalysis::new();
        let mut frames = Vec::new();
        while replay.step().is_some() {
            let (mut frame, coverage) = replay.snapshot_with_coverage();
            let depth = replay.depth();
            let (width, height) = (frame.width(), frame.height());
            let rect = crop.decide(auto_crop, depth, coverage.as_ref());
            let noise = match rect {
                Some(r) => crop_stack(&mut frame, coverage.as_ref(), &r).unwrap(),
                None => coverage,
            };
            analysis.note_crop(rect);
            let rendered = process_preview_frame_with_analysis(
                &mut frame,
                &settings,
                &night_amplifier::plugins::Plugins::installed(),
                AnalysisContext { showing_stack: depth > 1, stack_depth: depth as u32 },
                &mut analysis,
            )
            .unwrap();
            let ready = night_amplifier::render::display::RenderReadyFrame {
                noise: noise.map(std::sync::Arc::new),
                linear_frame: std::sync::Arc::new(frame),
                pipeline_config: rendered.pipeline_config,
                stretch_result: rendered.stretch_result,
            };
            let (rgb8, w, _) = night_amplifier::render::display::frame_to_rgb8_downsampled(&ready, u32::MAX, u32::MAX).unwrap();
            let rect = rect.unwrap_or(CropRect { x: 0, y: 0, width, height });
            frames.push((rect, rgb8, w as usize));
        }
        // The patch every crop of the session holds.
        let x0 = frames.iter().map(|(r, _, _)| r.x).max().unwrap();
        let y0 = frames.iter().map(|(r, _, _)| r.y).max().unwrap();
        let x1 = frames.iter().map(|(r, _, _)| r.x + r.width).min().unwrap();
        let y1 = frames.iter().map(|(r, _, _)| r.y + r.height).min().unwrap();
        let sky: Vec<f64> = frames
            .iter()
            .map(|(r, rgb8, w)| {
                let mut g: Vec<u8> = (y0..y1)
                    .flat_map(|y| (x0..x1).map(move |x| (y - r.y) * w + (x - r.x)))
                    .map(|i| rgb8[i * 3 + 1])
                    .collect();
                g.sort_unstable();
                g[g.len() / 2] as f64
            })
            .collect();
        let (mut at_change, mut elsewhere) = (Vec::new(), Vec::new());
        // Past the first 8 snapshots, where the young stack's tone moves on its own.
        for i in 8..frames.len() {
            let step = (sky[i] - sky[i - 1]).abs();
            if frames[i].0 != frames[i - 1].0 { at_change.push(step) } else { elsewhere.push(step) }
        }
        (at_change, elsewhere)
    };
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let max = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
    let (on_change, on_else) = steps(true);
    let (_, off_all) = steps(false);
    println!(
        "sky step, crop on: at {} changes mean {:.2} max {:.0}; elsewhere mean {:.2} max {:.0}; crop off: mean {:.2} max {:.0}",
        on_change.len(), mean(&on_change), max(&on_change), mean(&on_else), max(&on_else), mean(&off_all), max(&off_all)
    );
    assert!(
        max(&on_change) <= max(&off_all).max(1.0),
        "a crop change stepped the sky by {} levels; without the crop no step exceeded {}",
        max(&on_change), max(&off_all)
    );
}
