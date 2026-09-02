//! Benchmark for `process_preview_frame` — the whole linear half of the render thread.
//!
//! Run with: cargo bench --bench preview_pipeline_benchmark
//!
//! Everything the render task does before the encoders happens here: background
//! neutralisation, background subtraction, SCNR, and the auto-stretch preparation. In
//! production traces it was 190 ms of a 300 ms `render_iteration`, and it had no
//! benchmark — the pieces were covered individually by `render_benchmark`,
//! `background_benchmark` and `statistics_benchmark`, but not the sum, so there was no
//! number to measure a *structural* change against.
//!
//! # What the cases are for
//!
//! `analysis` is the part a cross-frame cache could skip: the white-balance grid, the
//! background model estimate, and the image statistics. All three are statistical
//! descriptions of a stack that moves by 1/N per frame, so recomputing them every frame
//! is arguably waste — but only if they are a large enough share of the total to be
//! worth the invalidation logic. This case measures that share directly, against
//! `full`, so the decision is evidence rather than arithmetic.
//!
//! The pipeline mutates its input, so this uses `iter_batched_ref` over a `Vec` of
//! clones — plain repetition would feed iteration N+1 iteration N's output, and a
//! neutralised, background-subtracted frame is not the workload the first iteration ran.
//! At ~50 MB a clone that is also why `REPS` is small and the measurement window short.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, SamplingMode};
use night_amplifier::background::BackgroundConfig;
use night_amplifier::frame::Frame;
use night_amplifier::server::capture::pipeline::{
    process_preview_frame, process_preview_frame_with_analysis,
};
use night_amplifier::server::capture::{AnalysisContext, PreviewAnalysis};
use night_amplifier::server::state::CaptureSettings;
use night_amplifier::{compute_image_stats, WhiteBalanceConfig};
use std::hint::black_box;
use std::time::Duration;

/// IMX464 resolution — the sensor the rest of the suite is sized against.
const WIDTH: usize = 2712;
const HEIGHT: usize = 1538;

/// A light-pollution gradient with a colour cast, read noise and a bright object, so
/// every stage has something real to do: a cast to neutralise, a gradient to model, and
/// a signal the percentile rejections are supposed to protect.
fn create_sky_frame() -> Frame {
    let mut frame = Frame::zeros(WIDTH, HEIGHT, 3).unwrap();
    let mut seed = 0xC0FF_EE11u32;
    let mut rand = move || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / 16_777_216.0
    };

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let grad = 0.02 + 0.06 * (x as f32 / WIDTH as f32) + 0.03 * (y as f32 / HEIGHT as f32);
            let noise = (rand() - 0.5) * 0.01;
            frame.set_pixel(x, y, 0, grad * 1.25 + noise);
            frame.set_pixel(x, y, 1, grad + noise);
            frame.set_pixel(x, y, 2, grad * 0.85 + noise);
        }
    }

    for y in HEIGHT / 3..HEIGHT / 2 {
        for x in WIDTH / 3..WIDTH / 2 {
            for c in 0..3 {
                frame.set_pixel(x, y, c, 0.85);
            }
        }
    }

    frame
}

/// One pass is ~20 ms, so 6 clones clear the ~100 ms floor. Kept low because each one is
/// a live ~50 MB frame.
const REPS: usize = 6;

fn bench_preview_pipeline(c: &mut Criterion) {
    let frame = create_sky_frame();
    let settings = CaptureSettings::default();

    let mut group = c.benchmark_group("preview_pipeline");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_millis(2000));

    group.bench_function(format!("full_x{}", REPS), |b| {
        b.iter_batched_ref(
            || vec![frame.clone(); REPS],
            |frames| {
                for f in frames.iter_mut() {
                    black_box(process_preview_frame(f, &settings).unwrap());
                }
            },
            BatchSize::LargeInput,
        )
    });

    // The three estimates a cross-frame cache would serve from a previous frame, run
    // against an unmodified frame — which is what the cached values would describe.
    // Read-only, so no clone per sample is needed.
    let bg_config = BackgroundConfig::default();
    group.bench_function(format!("analysis_x{}", REPS), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                black_box(
                    night_amplifier::render::compute_white_balance_grid_with_config(
                        black_box(&frame),
                        16,
                        25.0,
                        WhiteBalanceConfig::preview(),
                    )
                    .unwrap(),
                );
                black_box(
                    night_amplifier::background::BackgroundExtractor::new(bg_config.clone())
                        .estimate(black_box(&frame))
                        .unwrap(),
                );
                black_box(compute_image_stats(black_box(&frame)).unwrap());
            }
        })
    });

    // The same work with the analysis cache live: the first frame measures, the rest of
    // the batch reuse. This is what the render task actually runs on a deep stack, and
    // the gap against `full_x6` is what the cache is worth.
    group.bench_function(format!("cached_x{}", REPS), |b| {
        b.iter_batched_ref(
            || (vec![frame.clone(); REPS], PreviewAnalysis::new()),
            |(frames, analysis)| {
                // A deep, slowly growing stack — the case the cache exists for. Depth
                // moves by one per frame, which stays inside the refresh band.
                for (i, f) in frames.iter_mut().enumerate() {
                    let ctx = AnalysisContext {
                        showing_stack: true,
                        stack_depth: 1000 + i as u32,
                    };
                    black_box(process_preview_frame_with_analysis(f, &settings, ctx, analysis).unwrap());
                }
            },
            BatchSize::LargeInput,
        )
    });

    group.finish();
}

criterion_group!(benches, bench_preview_pipeline);
criterion_main!(benches);
