use criterion::{criterion_group, criterion_main, Criterion, SamplingMode, Throughput};
use night_amplifier::background::{
    BackgroundConfig, BackgroundExtractionAlgorithm, BackgroundExtractor,
};
use night_amplifier::frame::Frame;
use std::hint::black_box;
use std::time::Duration;

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

fn bench_background_estimation_grid(c: &mut Criterion) {
    // 2712 x 1538 matches the resolution in the trace logs
    let frame = create_test_frame(2712, 1538, 3);

    let mut group = c.benchmark_group("background_estimation_grid");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let config_grid =
        BackgroundConfig::default().with_algorithm(BackgroundExtractionAlgorithm::GridBilinear);
    let extractor_grid = BackgroundExtractor::new(config_grid);

    // One estimate over a 2712x1538x3 frame is ~0.8 ms — an order of magnitude below the
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

    group.finish();
}

criterion_group!(benches, bench_background_estimation_grid);
criterion_main!(benches);
