//! Benchmarks for the live stack accumulator's plain-mean path.
//!
//! Run with: cargo bench --bench stacking_benchmark -- --noplot
//!
//! `MasterStack::add_frame` without rejection is Community's per-frame stacking kernel, and
//! also what Pro runs with rejection set to None. Pro's `rejection_benchmark` covers the
//! clipping kernel; nothing measured this one, so its non-finite skip was priced by hand.

use criterion::{criterion_group, criterion_main, Criterion, SamplingMode, Throughput};
use night_amplifier::stacking::{MasterStack, RejectionMethod, StackingConfig};
use night_amplifier::Frame;
use std::hint::black_box;
use std::time::Duration;

/// A 3008x3008 colour sensor (IMX533 class): the accumulator's documented 434 MB case.
const WIDTH: usize = 3008;
const HEIGHT: usize = 3008;
const CHANNELS: usize = 3;

/// `add_frame` calls per measured iteration. One call is ~16 ms on x86, so eight clear
/// the ~100 ms floor. **The reported `time:` is for `REPS` frames, not one.**
const REPS: usize = 8;

/// Distinct frames cycled through, so the blend sees changing values, not a fixed point.
const FRAMES: usize = 4;

/// Rows of warp border (exactly 0.0) at the top and bottom, as a registered frame has:
/// the border branch must be taken, and mispredicted, as often as in production.
const BORDER_ROWS: usize = 40;

/// Background-subtracted sky with ~2e-5 of deterministic noise and a warp border.
fn sky_frame(seed: usize) -> Frame {
    let mut data = vec![0.0f32; WIDTH * HEIGHT * CHANNELS];
    for (i, v) in data.iter_mut().enumerate() {
        let row = (i % (WIDTH * HEIGHT)) / WIDTH;
        if row < BORDER_ROWS || row >= HEIGHT - BORDER_ROWS {
            continue;
        }
        let hash = (i ^ seed.wrapping_mul(0x9E37_79B9)).wrapping_mul(0x2545_F491) >> 16;
        *v = 0.0024 + ((hash % 1000) as f32 - 500.0) * 4e-8;
    }
    Frame::from_f32_vec(data, WIDTH, HEIGHT, CHANNELS).expect("frame")
}

/// One long-lived stack instead of a fresh one per batch: the plain-mean blend costs the
/// same whatever the pixel state, and cloning 434 MB per batch would dominate. The count
/// stays far below the 65,535 at which it saturates.
fn plain_mean_benchmark(c: &mut Criterion) {
    let frames: Vec<Frame> = (0..FRAMES).map(sky_frame).collect();
    let config = StackingConfig::default().with_rejection(RejectionMethod::None);
    let mut stack = MasterStack::new(WIDTH, HEIGHT, CHANNELS, config).expect("stack");
    for frame in &frames {
        stack.add_frame(frame).expect("add_frame");
    }

    let mut group = c.benchmark_group("master_stack");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));
    group.throughput(Throughput::Elements((REPS * WIDTH * HEIGHT * CHANNELS) as u64));

    let mut next = 0;
    group.bench_function(format!("plain_mean_3008x3008x3_x{REPS}"), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                stack.add_frame(black_box(&frames[next])).expect("add_frame");
                next = (next + 1) % FRAMES;
            }
        })
    });

    // The display copy with and without the coverage map. `compute_with_coverage` takes
    // both from one read of the 434 MB accumulator, so what it adds over `compute` is one
    // plane's block medians of counts — the whole of the per-frame noise-map cost.
    group.bench_function(format!("compute_3008x3008x3_x{REPS}"), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                black_box(stack.compute().expect("compute"));
            }
        })
    });
    group.bench_function(format!("compute_with_coverage_3008x3008x3_x{REPS}"), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                black_box(stack.compute_with_coverage().expect("compute_with_coverage"));
            }
        })
    });

    group.finish();
}

criterion_group!(benches, plain_mean_benchmark);
criterion_main!(benches);
