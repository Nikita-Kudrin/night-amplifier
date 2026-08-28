use criterion::{criterion_group, criterion_main, BatchSize, Criterion, SamplingMode, Throughput};
use night_amplifier::background::{BackgroundConfig, BackgroundExtractor};
use night_amplifier::frame::Frame;
use night_amplifier::render::stretch::apply_fused_stretch_frame;
use night_amplifier::{auto_stretch_frame, AutoStretchConfig};
use std::hint::black_box;
use std::time::Duration;

// Every group below hands its input to `iter_batched_ref` rather than cloning inside
// `b.iter`. That is not a style preference: a 2712x1538x3 `Frame::clone` measures ~14 ms
// on its own, so an in-loop clone was 77 % of the reported `fused_stretch_frame` figure
// (18.7 ms for ~4.3 ms of kernel) and roughly three quarters of the others. The suite
// could not resolve a sub-30 % regression in the very kernels the planar migration
// exists to speed up. `BatchSize::LargeInput` keeps one input live at a time, which
// matters at ~50 MB per frame.
//
// These kernels all mutate the frame in place, so the "run it `REPS` times" trick the
// pure benches use (see `debayer_benchmark`) is not available: iteration two would see
// iteration one's output. Instead each setup produces a `Vec` of `REPS` clones and the
// routine walks it, which raises the measured region above ~10 ms — below that, thermal
// management moves the figure by more than a real regression would — while every clone
// still starts from the same input. `REPS` clones are live at once, so it is chosen per
// group from what the kernel costs, not set globally.
fn create_test_frame(width: usize, height: usize, channels: usize) -> Frame {
    let mut frame = Frame::zeros(width, height, channels).unwrap();
    // Fill with some gradient data
    for y in 0..height {
        for x in 0..width {
            for c in 0..channels {
                let value =
                    0.1 + (x as f32 / width as f32) * 0.2 + (y as f32 / height as f32) * 0.1;
                frame.set_pixel(x, y, c, value);
            }
        }
    }
    frame
}

fn bench_subtract_from(c: &mut Criterion) {
    let frame = create_test_frame(2712, 1538, 3);
    let config = BackgroundConfig::default();
    let extractor = BackgroundExtractor::new(config);
    let model = extractor.estimate(&frame).unwrap();

    let mut group = c.benchmark_group("background_subtract");
    // Every case here is >= 10 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // ~2.25 ms per call; five clones is ~250 MB live and lands the measurement at ~11 ms.
    const REPS: usize = 5;
    group.bench_function(format!("subtract_from_x{}", REPS), |b| {
        b.iter_batched_ref(
            || vec![frame.clone(); REPS],
            |frames| {
                for test_frame in frames.iter_mut() {
                    model.subtract_from(black_box(test_frame));
                }
            },
            BatchSize::LargeInput,
        )
    });

    group.finish();
}

fn bench_auto_stretch(c: &mut Criterion) {
    let frame = create_test_frame(2712, 1538, 3);
    use night_amplifier::render::ToneMappingAlgorithm;
    let stretch_config = AutoStretchConfig::default().with_tone_mapping(ToneMappingAlgorithm::Mtf);

    let mut group = c.benchmark_group("auto_stretch");
    // Every case here is >= 10 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // ~7.2 ms per call, so two clones is enough to clear 10 ms without doubling the
    // memory further.
    const REPS: usize = 2;
    group.bench_function(format!("auto_stretch_frame_x{}", REPS), |b| {
        b.iter_batched_ref(
            || vec![frame.clone(); REPS],
            |frames| {
                for test_frame in frames.iter_mut() {
                    let _ = auto_stretch_frame(black_box(test_frame), stretch_config, None);
                }
            },
            BatchSize::LargeInput,
        )
    });

    group.finish();
}

fn bench_scale_lut(c: &mut Criterion) {
    let data = vec![0.5f32; 2712 * 1538 * 3];

    let mut scale_lut = vec![0.0f32; 8192];
    for (i, v) in scale_lut.iter_mut().enumerate() {
        *v = 1.0 + (i as f32 / 8191.0);
    }

    let mut group = c.benchmark_group("scale_lut");
    // Every case here is >= 10 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("simd", |b| {
        b.iter_batched_ref(
            || data.clone(),
            |test_data| {
                night_amplifier::render::simd::apply_luminance_scale_lut_simd(
                    black_box(test_data),
                    0.05,
                    black_box(&scale_lut),
                    1.0,
                )
            },
            BatchSize::LargeInput,
        )
    });

    group.bench_function("scalar", |b| {
        b.iter_batched_ref(
            || data.clone(),
            |test_data| {
                night_amplifier::render::simd::apply_luminance_scale_lut_scalar(
                    black_box(test_data),
                    0.05,
                    black_box(&scale_lut),
                    1.0,
                )
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
    // does: `expand_to_rgb8_fused` runs this kernel once per row of the frame.
    //
    // **The reported `time:` is for `ROWS` rows, not one.**
    const ROWS: usize = 1024;
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

fn bench_fused_stretch(c: &mut Criterion) {
    let frame = create_test_frame(2712, 1538, 3);

    let mut group = c.benchmark_group("apply_fused_stretch");
    // Every case here is >= 10 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // ~2.3 ms per call — the fastest full-frame kernel here, and the one the planar
    // migration exists to speed up, so it is the one that most needs a stable figure.
    const REPS: usize = 5;
    group.bench_function(format!("fused_stretch_frame_x{}", REPS), |b| {
        b.iter_batched_ref(
            || vec![frame.clone(); REPS],
            |frames| {
                for test_frame in frames.iter_mut() {
                    apply_fused_stretch_frame(
                        black_box(test_frame),
                        0.05,
                        night_amplifier::render::ToneMappingAlgorithm::Mtf,
                        0.15,
                        1.0,
                        None,
                    )
                    .unwrap();
                }
            },
            BatchSize::LargeInput,
        )
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_subtract_from,
    bench_auto_stretch,
    bench_fused_stretch,
    bench_scale_lut
);
criterion_main!(benches);
