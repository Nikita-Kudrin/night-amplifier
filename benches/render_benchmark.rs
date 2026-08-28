use criterion::{criterion_group, criterion_main, BatchSize, Criterion, SamplingMode};
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
// routine walks it, which raises the measured region above ~100 ms — below that, thermal
// management moves the figure by more than a real regression would — while every clone
// still starts from the same input. `REPS` clones are live at once, so it is chosen per
// group from what the kernel costs, not set globally.
//
// At a 100 ms floor, `REPS` here is large enough that the *setup* clone (~14-16 ms per
// 2712x1538x3 frame, ~50 MB) costs more wall clock per sample than the measured region
// itself — setup isn't in the reported figure, but it still runs once per sample, and
// with three such groups in one binary the ~30 s budget is the binding constraint. This
// is also why `scale_lut_benchmark` is a separate binary rather than a fourth group here:
// the four extra clone-heavy cases it would add did not fit alongside these three.
// `REPS` is sized to the minimum that clears 100 ms, not padded with the margin other
// files use, and `warm_up_time` is shortened: warm-up pays the same setup cost without
// needing the statistical rigor the measured samples do.
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
    // Every case here is >= 100 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(2));

    // ~2.4 ms per call, so 44 clones (~2.2 GB live, see the module comment on why that's
    // the minimum rather than a padded figure) clears ~106 ms.
    const REPS: usize = 44;
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
    // Every case here is >= 100 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(2));

    // ~7 ms per call, so 16 clones (~800 MB live) clears ~112 ms.
    const REPS: usize = 16;
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

fn bench_fused_stretch(c: &mut Criterion) {
    let frame = create_test_frame(2712, 1538, 3);

    let mut group = c.benchmark_group("apply_fused_stretch");
    // Every case here is >= 100 ms, and criterion's default linear scheme wants
    // 1+2+...+10 = 55 iterations per case. Flat keeps each inside its 2 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(2));

    // ~2.2 ms per call — the fastest full-frame kernel here, and the one the planar
    // migration exists to speed up, so it is the one that most needs a stable figure.
    // 44 clones (~2.2 GB live) clears ~105 ms.
    const REPS: usize = 44;
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
    bench_fused_stretch
);
criterion_main!(benches);
