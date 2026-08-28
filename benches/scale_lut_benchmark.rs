//! Benchmarks for the luminance scale LUT kernel (SIMD vs scalar).
//!
//! Split out of `render_benchmark` rather than added as a fourth group there: at the
//! ~100 ms floor the frame-mutation kernels in that file already fill its ~30 s budget
//! (their setup clone dominates wall clock — see the comment there), and this group's
//! four cases did not fit alongside them.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, SamplingMode, Throughput};
use std::hint::black_box;
use std::time::Duration;

fn bench_scale_lut(c: &mut Criterion) {
    let data = vec![0.5f32; 2712 * 1538 * 3];

    let mut scale_lut = vec![0.0f32; 8192];
    for (i, v) in scale_lut.iter_mut().enumerate() {
        *v = 1.0 + (i as f32 / 8191.0);
    }

    let mut group = c.benchmark_group("scale_lut");
    // Every case here is >= 100 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // The kernel mutates its buffer in place, so each iteration gets its own fresh clone
    // rather than being repeated over one buffer (see `render_benchmark`'s module docs
    // for why that trick isn't available here). ~21.5 ms per call; five clones
    // (~250 MB live) clears ~107 ms.
    const WHOLE_BUFFER_REPS: usize = 5;

    group.bench_function(format!("simd_x{}", WHOLE_BUFFER_REPS), |b| {
        b.iter_batched_ref(
            || vec![data.clone(); WHOLE_BUFFER_REPS],
            |buffers| {
                for test_data in buffers.iter_mut() {
                    night_amplifier::render::simd::apply_luminance_scale_lut_simd(
                        black_box(test_data),
                        0.05,
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
                        0.05,
                        black_box(&scale_lut),
                        1.0,
                    )
                }
            },
            BatchSize::LargeInput,
        )
    });

    // The whole-buffer cases above are not the shape production uses: the streaming
    // encoder calls these per interleaved row from inside a rayon task, so the working
    // set is one row, not the frame. Measured at both sizes because that is what decides
    // whether the interleaved SIMD variant earns its keep next to the scalar one.
    //
    // One 2712-pixel row takes ~14 us, which is far too short to compare two kernels
    // that measured within each other's confidence intervals to begin with. A frame's
    // worth of rows is applied per iteration instead — the same call shape and the same
    // one-row working set, just enough of them to measure. That is also what the encoder
    // does: `expand_to_rgb8_fused` runs this kernel once per row of the frame. 1024 rows
    // landed at ~14 ms; 8192 clears the ~100 ms floor.
    //
    // **The reported `time:` is for `ROWS` rows, not one.**
    const ROWS: usize = 8192;
    const ROW_LEN: usize = 2712 * 3;
    let rows = vec![0.5f32; ROW_LEN * ROWS];

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
                                0.05,
                                black_box(&scale_lut),
                                1.0,
                            )
                        } else {
                            night_amplifier::render::simd::apply_luminance_scale_lut_scalar(
                                black_box(test_row),
                                0.05,
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
