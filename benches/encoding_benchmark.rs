use std::hint::black_box;
use criterion::{criterion_group, criterion_main, Criterion};
use night_amplifier::frame::Frame;
use night_amplifier::server::encode_rgb8_lz4;
use std::time::Duration;

fn create_test_frame(width: usize, height: usize, channels: usize) -> Frame {
    let mut frame = Frame::zeros(width, height, channels).unwrap();
    // Fill with some data to ensure compression isn't trivial
    for y in 0..height {
        for x in 0..width {
            for c in 0..channels {
                let value = ((x * y * (c + 1)) % 255) as f32 / 255.0;
                frame.set_pixel(x, y, c, value);
            }
        }
    }
    frame
}

fn bench_encoding(c: &mut Criterion) {
    // IMX464 resolution (3 channels - fast path)
    let frame_imx464_rgb = create_test_frame(2712, 1538, 3);
    
    // IMX464 resolution (1 channel - slow path with debayer)
    let frame_imx464_mono = create_test_frame(2712, 1538, 1);
    
    // 4K resolution (to test downsampling threshold)
    let frame_4k = create_test_frame(3840, 2160, 3);
    
    // 8K resolution (simulating a very large sensor that will trigger downsampling)
    let frame_8k = create_test_frame(7680, 4320, 3);

    let mut group = c.benchmark_group("encode_rgb8_lz4");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("encode_imx464_rgb", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&frame_imx464_rgb)).unwrap())
    });

    group.bench_function("encode_imx464_mono", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&frame_imx464_mono)).unwrap())
    });
    
    group.bench_function("encode_4k", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&frame_4k)).unwrap())
    });

    group.bench_function("encode_8k", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&frame_8k)).unwrap())
    });

    group.finish();
}

criterion_group!(benches, bench_encoding);
criterion_main!(benches);
