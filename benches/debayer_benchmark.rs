//! Benchmarks for debayering algorithms. Run with: cargo bench --bench debayer_benchmark
//!
//! A single 2712x1538 bilinear pass is ~4.4 ms — at that scale criterion's timing noise and
//! thermal throttling move the number more than a real regression would, so every case repeats
//! the kernel until the measured region clears ~100 ms, declaring `Throughput::Elements(REPS *
//! pixels)` for a readable per-pixel rate. **The reported `time:` is for `REPS` invocations, not
//! one** — divide by `REPS`. Sound because debayering is pure (reads `&Frame`, returns a fresh
//! buffer); in-place kernels need `iter_batched_ref` instead (see `render_benchmark`).

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput};
use night_amplifier::{CfaPattern, DebayerAlgorithm, DebayerConfig, Frame};
use std::hint::black_box;
use std::time::Duration;

/// Kernel invocations per measured iteration. See the module docs.
const REPS: usize = 25;

/// Generate a synthetic Bayer frame for benchmarking
fn generate_bayer_frame(width: usize, height: usize) -> Frame {
    let mut data = vec![0.0f32; width * height];

    // Create a gradient pattern to simulate real image data
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;
            // Simulate typical astronomical image: low background with some variation
            data[idx] = 0.05 + 0.1 * fx + 0.1 * fy + 0.02 * ((x + y) as f32 * 0.1).sin();
        }
    }

    Frame::from_f32_vec(data, width, height, 1).expect("Failed to create frame")
}

/// `2712x1538` is the IMX464, the sensor the live-view work is tuned against — non-square, odd
/// height, and the geometry whose rayon task count the chunk granularity actually has to serve.
/// Squares alone hid a 10 % granularity loss.
const BILINEAR_SIZES: [(usize, usize); 2] = [(2048, 2048), (2712, 1538)];

fn debayer_bilinear_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("debayer_bilinear");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    for (width, height) in BILINEAR_SIZES.iter() {
        let frame = generate_bayer_frame(*width, *height);
        let config =
            DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Bilinear);

        group.throughput(Throughput::Elements((REPS * width * height) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x{}_x{}", width, height, REPS)),
            &frame,
            |b, frame| {
                b.iter(|| {
                    for _ in 0..REPS {
                        black_box(
                            night_amplifier::debayer_with_config(black_box(frame), config.clone())
                                .expect("Debayer failed"),
                        );
                    }
                })
            },
        );
    }

    // GRBG puts green at both column parities and interpolates red and blue on the opposite axes
    // from RGGB, so it exercises the other half of the per-row layout.
    let frame = generate_bayer_frame(2712, 1538);
    let config = DebayerConfig::new(CfaPattern::Grbg).with_algorithm(DebayerAlgorithm::Bilinear);
    group.throughput(Throughput::Elements((REPS * 2712 * 1538) as u64));
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("2712x1538_grbg_x{}", REPS)),
        &frame,
        |b, frame| {
            b.iter(|| {
                for _ in 0..REPS {
                    black_box(
                        night_amplifier::debayer_with_config(black_box(frame), config.clone())
                            .expect("Debayer failed"),
                    );
                }
            })
        },
    );

    // The 8-bit streaming path, which had no coverage. Note it is not a like-for-like
    // control against the f32 path above: 8-bit output is a quarter of the write
    // volume, so the gap between them is mostly bytes written, not layout. At ~2.4 ms
    // per call it needs more repeats than the f32 path to clear the ~100 ms floor.
    const TO_RGB8_REPS: usize = 50;
    let frame = generate_bayer_frame(2712, 1538);
    group.throughput(Throughput::Elements((TO_RGB8_REPS * 2712 * 1538) as u64));
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("2712x1538_to_rgb8_x{}", TO_RGB8_REPS)),
        &frame,
        |b, frame| {
            b.iter(|| {
                for _ in 0..TO_RGB8_REPS {
                    black_box(
                        night_amplifier::debayer::debayer_bilinear_to_rgb8_fast(
                            black_box(frame),
                            CfaPattern::Rggb,
                        )
                        .expect("Debayer failed"),
                    );
                }
            })
        },
    );

    group.finish();
}

fn debayer_vng_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("debayer_vng");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // One size: VNG at 2048x2048 is ~19 ms on its own, and the 1024x1024 case only
    // re-measured the same kernel. `VNG_REPS` repeats clear the ~100 ms floor.
    const VNG_REPS: usize = 7;
    let size = 2048;
    let frame = generate_bayer_frame(size, size);
    let config = DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Vng);

    group.throughput(Throughput::Elements((VNG_REPS * size * size) as u64));
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("{}x{}_x{}", size, size, VNG_REPS)),
        &frame,
        |b, frame| {
            b.iter(|| {
                for _ in 0..VNG_REPS {
                    black_box(
                        night_amplifier::debayer_with_config(black_box(frame), config.clone())
                            .expect("Debayer failed"),
                    );
                }
            })
        },
    );

    group.finish();
}

criterion_group!(benches, debayer_bilinear_benchmark, debayer_vng_benchmark);
criterion_main!(benches);
