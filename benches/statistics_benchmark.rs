//! Benchmark for `compute_image_stats` — the robust per-channel median/MAD.
//!
//! Run with: cargo bench --bench statistics_benchmark
//!
//! This runs at least once per rendered frame, inside `prepare_auto_stretch_frame`, and
//! its output is what the whole tone curve is solved against. It had no benchmark, which
//! is how a stage reported at 13.3 ms of a 300 ms `render_iteration` in production traces
//! stayed invisible to CI.
//!
//! Uncontended it is **3.3 ms**, near the 4.4 ms *minimum* the same traces recorded and
//! well under their 13.3 ms mean. That gap is not this stage getting slower under load;
//! it is the render and stacking threads sharing one rayon pool. Worth remembering before
//! reading any single trace figure as the cost of the work.
//!
//! A separate binary rather than a fifth group in `render_benchmark`: that file is
//! already at ~28 s of the ~30 s budget with four groups, which is the same reason
//! `scale_lut_benchmark` was split out of it.
//!
//! # What the cases are for
//!
//! `max_samples` is the one lever this stage has, and the cases bracket it: the shipped
//! 100 k, a quarter of it, and no sampling at all.
//!
//! `full_precision` is not a production candidate — it reads every pixel of every plane
//! — but it is the case that says where the time goes. It puts **42x** the samples
//! through the same median and MAD arithmetic with no gather at all, so comparing it
//! against `default_100k` separates the strided gather from the two `select_nth` passes
//! that follow it. That comparison is the reason this file exists: the gather was
//! assumed to dominate, and it does not.
//!
//! `compute_image_stats_with_config` is pure (`&Frame` in, fresh `ImageStats` out), so
//! each case repeats it `REPS` times inside `b.iter`.
//! **The reported `time:` is for `REPS` invocations, not one.**

use criterion::{criterion_group, criterion_main, Criterion, SamplingMode, Throughput};
use night_amplifier::frame::Frame;
use night_amplifier::{compute_image_stats_with_config, StatsConfig};
use std::hint::black_box;
use std::time::Duration;

/// IMX464 resolution — the sensor the rest of the suite is sized against.
const WIDTH: usize = 2712;
const HEIGHT: usize = 1538;

/// A light-pollution-shaped gradient with read noise and a bright object, so the median
/// and MAD have something to be robust *about*. A constant fixture would make
/// `select_nth_unstable` degenerate and measure the wrong thing.
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

/// A sampled pass is ~3.3 ms, so 24 repeats clears the ~100 ms floor.
const SAMPLED_REPS: usize = 24;

/// `full_precision` reads all 4.2 M pixels of all three planes at ~103 ms a pass, so a
/// single one already clears the floor. Two would put this binary over the ~30 s budget.
const FULL_REPS: usize = 1;

fn bench_image_stats(c: &mut Criterion) {
    let frame = create_sky_frame();

    let mut group = c.benchmark_group("image_stats");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_millis(1500));

    let default = StatsConfig::default();

    for (name, config, reps) in [
        ("default_100k", default, SAMPLED_REPS),
        ("sampled_25k", default.with_max_samples(25_000), SAMPLED_REPS),
        (
            "full_precision",
            StatsConfig::default().full_precision(),
            FULL_REPS,
        ),
    ] {
        group.throughput(Throughput::Elements((reps * WIDTH * HEIGHT * 3) as u64));
        group.bench_function(format!("{}_x{}", name, reps), |b| {
            b.iter(|| {
                for _ in 0..reps {
                    black_box(compute_image_stats_with_config(black_box(&frame), config).unwrap());
                }
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_image_stats);
criterion_main!(benches);
