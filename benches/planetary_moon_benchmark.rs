//! Planetary stacking per frame on real frames: 2026-10-24's Moon (`moon-drifting-imx533-ser`,
//! 3008², the Ares-C PRO's Bayer mosaic). That night alignment alone took 134 s a frame on the
//! Pi (~70 s here): an exhaustive ±50 px NCC over a 1504 px window, which a synthetic
//! 1354x768 bench never priced. Every case is one stacking-task step at production size,
//! plus their sum, so a regression in any of them shows as seconds, not milliseconds.

use criterion::{criterion_group, criterion_main, Criterion, SamplingMode};
use night_amplifier::camera::{Camera, CameraInfo, ImageFormat, RawFrame, SimulatedCamera};
use night_amplifier::frame::Frame;
use night_amplifier::planetary::{align, compute_centroid, AlignmentReference, AlignmentRoi, LuminancePyramid, Search};
use night_amplifier::session::capture::pipeline::{build_cfa_pipeline, convert_captured_frame, debayer_algorithm, stack_frame_with};
use night_amplifier::session::capture::{LiveStacker, StackSettings};
use night_amplifier::session::state::CaptureSettings;
use night_amplifier::stacking::StackingType;
use std::hint::black_box;
use std::time::Duration;

// The integration tests' fixture registry, download and Moon set; most of it is unused here.
#[allow(dead_code)]
#[path = "../tests/integration/common.rs"]
mod common;
#[allow(dead_code)]
#[path = "../tests/integration/planetary.rs"]
mod planetary;

/// Fixture frames held raw: the reference, the night's largest jump, the first past ±50 px,
/// the blurriest, and the last (255 px from the first).
const HELD: [usize; 5] = [0, 9, 14, 20, 29];

struct Moon {
    info: CameraInfo,
    settings: CaptureSettings,
    raws: Vec<RawFrame>,
}

impl Moon {
    fn load() -> Self {
        let settings = planetary::planetary_settings();
        let mut camera = SimulatedCamera::new(planetary::moon()).expect("the Moon replays");
        let capture = settings.to_capture_config().with_exposure_us(1);
        assert_eq!(capture.format, ImageFormat::Raw16);
        let mut raws = Vec::new();
        for index in 0..=HELD[HELD.len() - 1] {
            let raw = camera.capture(&capture).expect("the simulator captures");
            if HELD.contains(&index) {
                raws.push(raw);
            }
        }
        Self { info: camera.info().clone(), settings, raws }
    }

    fn convert(&self, raw: &RawFrame) -> Frame {
        let cfa = build_cfa_pipeline(&self.settings);
        convert_captured_frame(raw, &self.info, &cfa, debayer_algorithm(&self.settings)).expect("a Moon frame converts")
    }
}

fn bench_planetary_moon(c: &mut Criterion) {
    let moon = Moon::load();
    let frames: Vec<Frame> = moon.raws.iter().map(|raw| moon.convert(raw)).collect();
    let pyramids: Vec<LuminancePyramid> = frames.iter().map(LuminancePyramid::of).collect();
    let (w, h) = (frames[0].width(), frames[0].height());
    let reference_centroid = compute_centroid(pyramids[0].luminance(), w, h);
    let roi = AlignmentRoi::centered_at(reference_centroid.0, reference_centroid.1, w / 2, h / 2, w, h);
    let last = pyramids.len() - 1;
    let last_centroid = compute_centroid(pyramids[last].luminance(), w, h);
    let from_centroids = (reference_centroid.0 - last_centroid.0, reference_centroid.1 - last_centroid.1);
    let stack_settings = StackSettings::of(&moon.settings, &night_amplifier::plugins::Plugins::installed());

    let mut group = c.benchmark_group("planetary_moon");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // Repeats sized to clear ~100 ms on 20 x86 cores (2026-10-29: convert 23 ms, pyramid
    // 17 ms, reference window 6 ms, align 8 ms (25 with ±2 boxes and the window rebuilt per
    // frame), wide retry 14 ms, offer 78 ms). **`time:` is for all of them.**
    group.bench_function("convert_x5", |b| {
        b.iter(|| {
            for raw in &moon.raws {
                black_box(moon.convert(raw));
            }
        })
    });
    group.bench_function("luminance_pyramid_x8", |b| {
        b.iter(|| {
            for frame in frames.iter().cycle().take(8) {
                black_box(LuminancePyramid::of(frame));
            }
        })
    });
    // Once per reference: the reference's window on every level the search uses.
    group.bench_function("reference_window_x16", |b| {
        b.iter(|| {
            for _ in 0..16 {
                black_box(AlignmentReference::new(&pyramids[0], &roi));
            }
        })
    });
    // Around the centroids' estimate of a frame 255 px from the reference: the session's
    // first search, and its retry over four times the radius.
    let window = AlignmentReference::new(&pyramids[0], &roi);
    for (label, radius, reps) in [("align_drifted_x13", 50, 13), ("align_wide_retry_x8", 200, 8)] {
        let search = Search { expected: from_centroids, radius };
        group.bench_function(label, |b| {
            b.iter(|| {
                for _ in 0..reps {
                    black_box(align(&window, &pyramids[last], search, 2));
                }
            })
        });
    }

    // The stacking task's step after conversion: pyramid, centroid, search, warp, accumulate.
    // Offering the same held frames in turn keeps the accumulator's cost what a session's is.
    let mut stacker: Option<Box<dyn LiveStacker>> = None;
    let mut failed = false;
    stack_frame_with(&frames[0], StackingType::Planetary, &stack_settings, &mut stacker, &mut failed, false);
    let mut next = 0usize;
    group.bench_function("offer_x2", |b| {
        b.iter(|| {
            for _ in 0..2 {
                next = next % (frames.len() - 1) + 1;
                black_box(stack_frame_with(&frames[next], StackingType::Planetary, &stack_settings, &mut stacker, &mut failed, false));
            }
        })
    });
    group.bench_function("stacking_iteration", |b| {
        b.iter(|| {
            next = next % (moon.raws.len() - 1) + 1;
            let frame = moon.convert(&moon.raws[next]);
            black_box(stack_frame_with(&frame, StackingType::Planetary, &stack_settings, &mut stacker, &mut failed, false))
        })
    });
    assert!(!failed, "the Moon's stack failed");
    group.finish();
}

criterion_group!(benches, bench_planetary_moon);
criterion_main!(benches);
