//! Benchmark for `compute_image_stats` — the robust per-channel median/MAD, run at least
//! once per frame inside `prepare_auto_stretch_frame`. Unbenchmarked, it stayed invisible
//! at 13.3 ms of a 300 ms `render_iteration` in production traces; uncontended it's
//! **3.3 ms** — the traced mean is contention from sharing one rayon pool with stacking,
//! not a slower stage. Separate binary: `render_benchmark` already fills ~28/30 s budget.
//! Cases bracket `max_samples` (shipped 100k / quarter / none) and `full_precision`
//! (**42x** the samples, no gather) — comparing it to `default_100k` isolates the gather
//! from the two `select_nth` passes; the gather was assumed to dominate and does not.

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

/// The shipped pass is ~0.74 ms, so 144 repeats clear the ~100 ms floor.
const DEFAULT_REPS: usize = 144;

/// A quarter of the samples is ~0.24 ms a pass.
const QUARTER_REPS: usize = 432;

/// `full_precision` reads all 4.2 M pixels of all three planes at ~20 ms a pass.
const FULL_REPS: usize = 6;

fn bench_image_stats(c: &mut Criterion) {
    let frame = create_sky_frame();

    let mut group = c.benchmark_group("image_stats");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_millis(1500));

    let default = StatsConfig::default();

    for (name, config, reps) in [
        ("default_100k", default, DEFAULT_REPS),
        ("sampled_25k", default.with_max_samples(25_000), QUARTER_REPS),
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
