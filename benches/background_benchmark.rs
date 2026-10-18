use criterion::{criterion_group, criterion_main, Criterion, SamplingMode, Throughput};
use night_amplifier::background::{
    BackgroundConfig, BackgroundExtractionAlgorithm, BackgroundExtractor,
};
use night_amplifier::frame::Frame;
use std::hint::black_box;
use std::time::Duration;

/// A linear gradient, plus (`target`) a galaxy off-centre and a glow in one corner: the
/// gradient alone is the classifier's cheapest input (one degree, no rejection rounds).
fn create_test_frame(width: usize, height: usize, channels: usize, target: bool) -> Frame {
    let mut frame = Frame::zeros(width, height, channels).unwrap();
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32 / width as f32, y as f32 / height as f32);
            let mut value = 0.1 + fx * 0.2 + fy * 0.1;
            if target {
                let r2 = (fx - 0.55).powi(2) + (fy - 0.45).powi(2);
                value += 0.08 * (-r2 / (2.0 * 0.12f32.powi(2))).exp();
                value += 0.06 * (-(fx.powi(2) + (1.0 - fy).powi(2)).sqrt() / 0.2).exp();
            }
            for c in 0..channels {
                frame.set_pixel(x, y, c, value);
            }
        }
    }
    frame
}

fn bench_background_estimation_grid(c: &mut Criterion) {
    // 2712 x 1538 matches the resolution in the trace logs
    let frame = create_test_frame(2712, 1538, 3, false);
    let target = create_test_frame(2712, 1538, 3, true);

    let mut group = c.benchmark_group("background_estimation_grid");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let config_grid =
        BackgroundConfig::default().with_algorithm(BackgroundExtractionAlgorithm::GridBilinear);
    let extractor_grid = BackgroundExtractor::new(config_grid);

    // One estimate over a 2712x1538x3 frame is ~1-2 ms — an order of magnitude below the
    // point where the reported figure is stable run to run. `estimate` takes `&Frame` and
    // returns a fresh model, so repeating it measures the same work each time.
    // **The reported `time:` is for `REPS` estimates, not one.**
    const REPS: usize = 150;

    group.throughput(Throughput::Elements((REPS * 2712 * 1538) as u64));
    group.bench_function(format!("estimate_grid_x{}", REPS), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                black_box(extractor_grid.estimate(black_box(&frame)).unwrap());
            }
        })
    });
    group.bench_function(format!("estimate_grid_target_x{}", REPS), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                black_box(extractor_grid.estimate(black_box(&target)).unwrap());
            }
        })
    });

    group.finish();
}

criterion_group!(benches, bench_background_estimation_grid);
criterion_main!(benches);
