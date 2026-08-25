use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
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
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("subtract_from", |b| {
        b.iter_batched_ref(
            || frame.clone(),
            |test_frame| model.subtract_from(black_box(test_frame)),
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
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("auto_stretch_frame", |b| {
        b.iter_batched_ref(
            || frame.clone(),
            |test_frame| auto_stretch_frame(black_box(test_frame), stretch_config, None),
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
    let row = vec![0.5f32; 2712 * 3];
    for (label, is_simd) in [("simd_row", true), ("scalar_row", false)] {
        group.bench_function(label, |b| {
            b.iter_batched_ref(
                || row.clone(),
                |test_row| {
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
                },
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

fn bench_fused_stretch(c: &mut Criterion) {
    let frame = create_test_frame(2712, 1538, 3);

    let mut group = c.benchmark_group("apply_fused_stretch");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("fused_stretch_frame", |b| {
        b.iter_batched_ref(
            || frame.clone(),
            |test_frame| {
                apply_fused_stretch_frame(
                    black_box(test_frame),
                    0.05,
                    night_amplifier::render::ToneMappingAlgorithm::Mtf,
                    0.15,
                    1.0,
                    None,
                )
                .unwrap()
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
