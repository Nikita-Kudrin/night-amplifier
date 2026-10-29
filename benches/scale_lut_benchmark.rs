//! Benchmarks for the scale LUT kernels (SIMD vs scalar, luminance vs Planetary's max channel)
//! and Planetary's levels solve.
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

    // Planetary's table, read by each pixel's brightest channel, in the same row shape: its
    // cost against `simd_row` above is the whole price of the basis.
    let planetary_lut = night_amplifier::render::planetary::scale_lut(0.6);
    group.bench_function(format!("max_channel_row_x{ROWS}"), |b| {
        b.iter_batched_ref(
            || rows.clone(),
            |buf| {
                for test_row in buf.chunks_mut(ROW_LEN) {
                    night_amplifier::render::simd::apply_max_channel_scale_lut_simd(
                        black_box(test_row),
                        BLACK_POINT,
                        black_box(&planetary_lut),
                    )
                }
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

/// Planetary's levels, solved every frame: one full-resolution histogram pass of a 3008²
/// Moon-sized disc. One solve is a few ms, so `SOLVES` run per iteration.
fn bench_planetary_levels(c: &mut Criterion) {
    const SIZE: usize = 3008;
    const SOLVES: usize = 16;
    let mut data = vec![0.0f32; SIZE * SIZE * 3];
    let plane = SIZE * SIZE;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let r = ((x as f32 - 1504.0).hypot(y as f32 - 1504.0)) / 1350.0;
            let level = if r < 1.0 { 0.25 + 0.5 * (1.0 - r * r).sqrt() } else { 0.003 };
            for (c, tint) in [0.94f32, 1.0, 0.48].into_iter().enumerate() {
                data[c * plane + y * SIZE + x] = level * tint;
            }
        }
    }
    let frame = night_amplifier::Frame::from_f32_vec(data, SIZE, SIZE, 3).unwrap();

    let mut group = c.benchmark_group("planetary_levels");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));
    group.bench_function(format!("solve_3008sq_x{SOLVES}"), |b| {
        b.iter(|| {
            for _ in 0..SOLVES {
                black_box(night_amplifier::render::planetary::PlanetaryLevels::solve(black_box(&frame)));
            }
        })
    });
    group.finish();
}

criterion_group!(benches, bench_scale_lut, bench_planetary_levels);
criterion_main!(benches);
