use criterion::{criterion_group, criterion_main, Criterion};
use night_amplifier::background::{
    BackgroundConfig, BackgroundExtractionAlgorithm, BackgroundExtractor,
};
use night_amplifier::frame::Frame;
use std::hint::black_box;

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

fn bench_background_estimation(c: &mut Criterion) {
    // 2712 x 1538 matches the resolution in the trace logs
    let frame = create_test_frame(2712, 1538, 3);

    let mut group = c.benchmark_group("background_estimation");
    group.sample_size(10); // Since it takes >1 second, reduce sample size

    let config_grid =
        BackgroundConfig::default().with_algorithm(BackgroundExtractionAlgorithm::GridBilinear);
    let extractor_grid = BackgroundExtractor::new(config_grid);

    group.bench_function("estimate_grid", |b| {
        b.iter(|| extractor_grid.estimate(black_box(&frame)).unwrap())
    });

    let config_rbf = BackgroundConfig::default().with_algorithm(BackgroundExtractionAlgorithm::Rbf);
    let extractor_rbf = BackgroundExtractor::new(config_rbf);

    group.bench_function("estimate_rbf", |b| {
        b.iter(|| extractor_rbf.estimate(black_box(&frame)).unwrap())
    });

    group.finish();
}

criterion_group!(benches, bench_background_estimation);
criterion_main!(benches);
