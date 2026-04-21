//! Benchmarks for image warping
//!
//! Run with: cargo bench --bench warp_benchmark

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use night_amplifier::{warp_frame, AffineTransform, Frame};
use std::f32::consts::PI;
use std::hint::black_box;
use std::time::Duration;

/// Generate a synthetic frame for benchmarking
fn generate_frame(width: usize, height: usize) -> Frame {
    let mut data = vec![0.0f32; width * height * 3];

    // Create a gradient pattern to simulate real image data
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 3;
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;
            // Simulate typical astronomical image: low background with some variation
            data[idx] = 0.05 + 0.1 * fx; // R
            data[idx + 1] = 0.05 + 0.1 * fy; // G
            data[idx + 2] = 0.1; // B
        }
    }

    Frame::from_f32_vec(data, width, height, 3).expect("Failed to create frame")
}

fn warp_identity_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("warp_identity");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));

    for size in [1024, 2048].iter() {
        let frame = generate_frame(*size, *size);
        let transform = AffineTransform::identity();

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x{}", size, size)),
            &(&frame, &transform),
            |b, (frame, transform)| {
                b.iter(|| {
                    warp_frame(black_box(frame), black_box(transform), 0.0).expect("Warp failed")
                })
            },
        );
    }

    group.finish();
}

fn warp_rotation_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("warp_rotation");
    group.sample_size(20);
    group.warm_up_time(Duration::from_secs(1));

    for size in [1024, 2048].iter() {
        let frame = generate_frame(*size, *size);
        // Small rotation typical for astronomical tracking
        let transform = AffineTransform::new(PI / 180.0 * 2.0, 1.0, 5.0, 3.0);

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x{}", size, size)),
            &(&frame, &transform),
            |b, (frame, transform)| {
                b.iter(|| {
                    warp_frame(black_box(frame), black_box(transform), 0.0).expect("Warp failed")
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    warp_identity_benchmark,
    warp_rotation_benchmark
);
criterion_main!(benches);
