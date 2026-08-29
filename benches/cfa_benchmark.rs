//! Benchmarks for the raw-CFA stage — the corrections that run per frame, on the
//! mosaic, before demosaic.
//!
//! Run with: cargo bench --bench cfa_benchmark -- --noplot
//!
//! Both filters mutate the frame in place, so the "repeat the kernel `reps` times"
//! trick the pure benches use (see `debayer_benchmark`) is unavailable: iteration two
//! would read iteration one's output, and for the hot-pixel filter that is not even a
//! stable workload — the defects it found the first time are gone. Each setup therefore
//! builds a `Vec` of `reps` clones and the routine walks it, exactly as
//! `render_benchmark` does. A mono 3008² CFA frame is 36 MB, a third of the RGB frames
//! that file clones, so `reps` can be larger here for the same live footprint.
//!
//! `debayer_superpixel` is pure, so it takes the cheap repeat route with no per-sample
//! clone.
//!
//! **The reported `time:` covers `reps` invocations.** Divide, or read the throughput.
//! Per-call figures the current `reps` were sized from (x86 dev box, not a Pi 5):
//!
//! | Stage | 3008² (IMX533) | 2712x1538 (IMX464) |
//! |---|---|---|
//! | hot pixels | 7.0 ms | 4.3 ms |
//! | row/column FPN | 5.9 ms | 2.7 ms |
//! | superpixel debayer | 2.7 ms | 0.8 ms |
//!
//! The whole binary is ~20 s warm.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, SamplingMode, Throughput};
use night_amplifier::cfa::{remove_fpn, reject_hot_pixels, CfaFrame, HotPixelConfig};
use night_amplifier::{CfaPattern, Frame};
use std::hint::black_box;
use std::time::Duration;

/// `(width, height, reps)` per case. IMX533 (3008², the sensor the fixture
/// measurements come from) and IMX464 (2712x1538, non-square with an odd height — the
/// geometry that exercises the column-block tail and the row-parity split).
///
/// `reps` is per *case*, not per group: the two sensors differ by 3.8x in area, so one
/// global figure would either leave the smaller case under the 100 ms floor or hold
/// four times the frames live for the larger one.
const HOT_PIXEL_CASES: [(usize, usize, usize); 2] = [(3008, 3008, 16), (2712, 1538, 25)];
const FPN_CASES: [(usize, usize, usize); 2] = [(3008, 3008, 20), (2712, 1538, 42)];
const SUPERPIXEL_CASES: [(usize, usize, usize); 2] = [(3008, 3008, 40), (2712, 1538, 130)];

/// A mosaic with per-site levels, a gradient, noise and scattered hot pixels, so the
/// filters do the work they would on a real sub rather than an all-flat fast path.
fn cfa_frame(width: usize, height: usize) -> CfaFrame {
    let mut data = vec![0.0f32; width * height];
    let mut seed = 0x9E37_79B9u32;
    for y in 0..height {
        for x in 0..width {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let site = 0.002 + 0.001 * (x & 1) as f32 + 0.0005 * (y & 1) as f32;
            let gradient = 0.0005 * (x as f32 / width as f32 + y as f32 / height as f32);
            data[y * width + x] = site + gradient + noise * 0.003;
        }
    }
    // ~0.06 % hot, matching the fixture's measured 5 189 sites on 9 MP.
    for i in (0..width * height).step_by(1700) {
        data[i] = 0.4;
    }

    let frame = Frame::from_f32_vec(data, width, height, 1).expect("frame");
    CfaFrame::mosaic(frame, CfaPattern::Rggb).expect("mosaic")
}

fn bench_hot_pixels(c: &mut Criterion) {
    let mut group = c.benchmark_group("cfa_hot_pixels");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(2));

    let config = HotPixelConfig::default();
    for (width, height, reps) in HOT_PIXEL_CASES {
        let frame = cfa_frame(width, height);
        group.throughput(Throughput::Elements((reps * width * height) as u64));
        group.bench_function(format!("{width}x{height}_x{reps}"), |b| {
            b.iter_batched_ref(
                || vec![frame.clone(); reps],
                |frames| {
                    for f in frames.iter_mut() {
                        black_box(reject_hot_pixels(black_box(f), &config).expect("filter"));
                    }
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench_fpn(c: &mut Criterion) {
    let mut group = c.benchmark_group("cfa_row_column_fpn");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(2));

    for (width, height, reps) in FPN_CASES {
        let frame = cfa_frame(width, height);
        group.throughput(Throughput::Elements((reps * width * height) as u64));
        group.bench_function(format!("{width}x{height}_x{reps}"), |b| {
            b.iter_batched_ref(
                || vec![frame.clone(); reps],
                |frames| {
                    for f in frames.iter_mut() {
                        black_box(remove_fpn(black_box(f)).expect("fpn"));
                    }
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench_superpixel(c: &mut Criterion) {
    let mut group = c.benchmark_group("debayer_superpixel");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(2));

    for (width, height, reps) in SUPERPIXEL_CASES {
        let frame = cfa_frame(width, height).into_frame();
        group.throughput(Throughput::Elements((reps * width * height) as u64));
        group.bench_function(format!("{width}x{height}_x{reps}"), |b| {
            b.iter(|| {
                for _ in 0..reps {
                    black_box(
                        night_amplifier::debayer_with_config(
                            black_box(&frame),
                            night_amplifier::DebayerConfig::new(CfaPattern::Rggb)
                                .with_algorithm(night_amplifier::DebayerAlgorithm::Superpixel),
                        )
                        .expect("superpixel"),
                    );
                }
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_hot_pixels, bench_fpn, bench_superpixel);
criterion_main!(benches);
