//! Benchmarks for debayering algorithms
//!
//! Run with: cargo bench --bench debayer_benchmark

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use night_amplifier::{CfaPattern, DebayerAlgorithm, DebayerConfig, Frame};
use std::hint::black_box;

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

fn debayer_bilinear_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("debayer_bilinear");

    for size in [512, 1024, 2048, 4096].iter() {
        let frame = generate_bayer_frame(*size, *size);
        let config =
            DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Bilinear);

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x{}", size, size)),
            &frame,
            |b, frame| {
                b.iter(|| {
                    night_amplifier::debayer_with_config(black_box(frame), config.clone())
                        .expect("Debayer failed")
                })
            },
        );
    }

    group.finish();
}

fn debayer_vng_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("debayer_vng");

    for size in [512, 1024, 2048].iter() {
        let frame = generate_bayer_frame(*size, *size);
        let config = DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Vng);

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x{}", size, size)),
            &frame,
            |b, frame| {
                b.iter(|| {
                    night_amplifier::debayer_with_config(black_box(frame), config.clone())
                        .expect("Debayer failed")
                })
            },
        );
    }

    group.finish();
}

fn debayer_comparison_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("debayer_comparison");

    let size = 2048;
    let frame = generate_bayer_frame(size, size);

    let bilinear_config =
        DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Bilinear);
    let vng_config = DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Vng);

    group.bench_with_input(
        BenchmarkId::new("bilinear", format!("{}x{}", size, size)),
        &frame,
        |b, frame| {
            b.iter(|| {
                night_amplifier::debayer_with_config(black_box(frame), bilinear_config.clone())
                    .expect("Debayer failed")
            })
        },
    );

    group.bench_with_input(
        BenchmarkId::new("vng", format!("{}x{}", size, size)),
        &frame,
        |b, frame| {
            b.iter(|| {
                night_amplifier::debayer_with_config(black_box(frame), vng_config.clone())
                    .expect("Debayer failed")
            })
        },
    );

    group.finish();
}

criterion_group!(
    benches,
    debayer_bilinear_benchmark,
    debayer_vng_benchmark,
    debayer_comparison_benchmark
);
criterion_main!(benches);
