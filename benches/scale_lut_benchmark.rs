//! Benchmarks for the luminance scale LUT kernel (SIMD vs scalar).
//!
//! Split out of `render_benchmark` rather than added as a fourth group there: at the
//! ~100 ms floor the frame-mutation kernels in that file already fill its ~30 s budget
//! (their setup clone dominates wall clock — see the comment there), and this group's
//! four cases did not fit alongside them.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, SamplingMode, Throughput};
use std::hint::black_box;
use std::time::Duration;

/// Black point of [`stack_like`]'s sky: 2 sigma under it, where the solver puts it.
const BLACK_POINT: f32 = 0.044;

/// `len` interleaved samples shaped like a stack's: a 0.05 sky with 0.003 grain, 1 % stars up
/// to white. The table is indexed by luminance, so a constant input (as this bench once fed)
/// reads one entry and hides what a frame's spread across its octaves costs.
fn stack_like(len: usize) -> Vec<f32> {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut uniform = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let level = if uniform() < 0.01 {
            0.1 + 0.9 * uniform()
        } else {
            let (u, v) = (uniform().max(1e-7), uniform());
            0.05 + 0.003 * (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
        };
        out.extend([level * 1.02, level, level * 0.97]);
    }
    out.truncate(len);
    out
}

fn bench_scale_lut(c: &mut Criterion) {
    let data = stack_like(2712 * 1538 * 3);

    let scale_lut = night_amplifier::render::stretch::scale_lut_from(|l| 1.0 + l);

    let mut group = c.benchmark_group("scale_lut");
    // Every case here is >= 100 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // The kernel mutates its buffer in place, so each iteration gets its own fresh clone
    // rather than being repeated over one buffer (see `render_benchmark`'s module docs
    // for why that trick isn't available here). On a stack-shaped frame the scalar kernel
    // takes ~9.3 ms a call (x86, 20 cores), so eleven clones (~550 MB live) clear ~100 ms.
    const WHOLE_BUFFER_REPS: usize = 11;

    group.bench_function(format!("simd_x{}", WHOLE_BUFFER_REPS), |b| {
        b.iter_batched_ref(
            || vec![data.clone(); WHOLE_BUFFER_REPS],
            |buffers| {
                for test_data in buffers.iter_mut() {
                    night_amplifier::render::simd::apply_luminance_scale_lut_simd(
                        black_box(test_data),
                        BLACK_POINT,
                        black_box(&scale_lut),
                        1.0,
                    )
                }
            },
            BatchSize::LargeInput,
        )
    });

    group.bench_function(format!("scalar_x{}", WHOLE_BUFFER_REPS), |b| {
        b.iter_batched_ref(
            || vec![data.clone(); WHOLE_BUFFER_REPS],
            |buffers| {
                for test_data in buffers.iter_mut() {
                    night_amplifier::render::simd::apply_luminance_scale_lut_scalar(
                        black_box(test_data),
                        BLACK_POINT,
                        black_box(&scale_lut),
                        1.0,
                    )
                }
            },
            BatchSize::LargeInput,
        )
    });

    // The whole-buffer cases above aren't the shape production uses: the streaming encoder
    // calls these per interleaved row inside a rayon task, so the working set is one row, not
    // the frame — measured at both sizes to see whether the interleaved SIMD variant earns its
    // keep next to the scalar one. One row takes ~6 us, too short to compare kernels already
    // within each other's confidence intervals, so two frames' worth of rows run per iteration
    // instead — same call shape, same one-row working set, just enough to measure (8192 rows
    // ~53 ms scalar, 16384 clear the ~100 ms floor).
    // **The reported `time:` is for `ROWS` rows, not one.**
    const ROWS: usize = 16384;
    const ROW_LEN: usize = 2712 * 3;
    let rows = stack_like(ROW_LEN * ROWS);

    group.throughput(Throughput::Elements((ROWS * 2712) as u64));
    for (label, is_simd) in [("simd_row", true), ("scalar_row", false)] {
        group.bench_function(format!("{}_x{}", label, ROWS), |b| {
            b.iter_batched_ref(
                || rows.clone(),
                |buf| {
                    for test_row in buf.chunks_mut(ROW_LEN) {
                        if is_simd {
                            night_amplifier::render::simd::apply_luminance_scale_lut_simd(
                                black_box(test_row),
                                BLACK_POINT,
                                black_box(&scale_lut),
                                1.0,
                            )
                        } else {
                            night_amplifier::render::simd::apply_luminance_scale_lut_scalar(
                                black_box(test_row),
                                BLACK_POINT,
                                black_box(&scale_lut),
                                1.0,
                            )
                        }
                    }
                },
                BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

criterion_group!(benches, bench_scale_lut);
criterion_main!(benches);
