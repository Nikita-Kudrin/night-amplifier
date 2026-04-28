use std::hint::black_box;
use criterion::{criterion_group, criterion_main, Criterion};
use night_amplifier::background::{BackgroundConfig, BackgroundExtractor};
use night_amplifier::frame::Frame;
use night_amplifier::{auto_stretch_frame, AutoStretchConfig};
use std::time::Duration;

fn create_test_frame(width: usize, height: usize, channels: usize) -> Frame {
    let mut frame = Frame::zeros(width, height, channels).unwrap();
    // Fill with some gradient data
    for y in 0..height {
        for x in 0..width {
            for c in 0..channels {
                let value = 0.1 + (x as f32 / width as f32) * 0.2 + (y as f32 / height as f32) * 0.1;
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
        b.iter(|| {
            let mut test_frame = frame.clone();
            model.subtract_from(black_box(&mut test_frame));
        })
    });

    group.finish();
}

fn bench_auto_stretch(c: &mut Criterion) {
    let frame = create_test_frame(2712, 1538, 3);
    let stretch_config = AutoStretchConfig::default();

    let mut group = c.benchmark_group("auto_stretch");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("auto_stretch_frame", |b| {
        b.iter(|| {
            let mut test_frame = frame.clone();
            let _ = auto_stretch_frame(black_box(&mut test_frame), stretch_config);
        })
    });

    group.finish();
}

criterion_group!(benches, bench_subtract_from, bench_auto_stretch);
criterion_main!(benches);
