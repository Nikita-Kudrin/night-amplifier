use std::hint::black_box;
use criterion::{criterion_group, criterion_main, Criterion};
use night_amplifier::frame::Frame;
use night_amplifier::server::{encode_rgb8_lz4, encode_rgb8_lz4_chunked};
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

    // --- Chunked LZ4 (SA09) ---
    let mut group_chunked = c.benchmark_group("encode_chunked_lz4");
    group_chunked.sample_size(10);
    group_chunked.warm_up_time(Duration::from_millis(500));
    group_chunked.measurement_time(Duration::from_secs(2));

    // 1 chunk = stacking mode (sequential, no parallelism)
    group_chunked.bench_function("imx464_rgb_1chunk", |b| {
        b.iter(|| encode_rgb8_lz4_chunked(black_box(&frame_imx464_rgb), 1).unwrap())
    });

    // 4 chunks = Raspberry Pi 5 (4 cores)
    group_chunked.bench_function("imx464_rgb_4chunks", |b| {
        b.iter(|| encode_rgb8_lz4_chunked(black_box(&frame_imx464_rgb), 4).unwrap())
    });

    // 8 chunks = max parallelism
    group_chunked.bench_function("imx464_rgb_8chunks", |b| {
        b.iter(|| encode_rgb8_lz4_chunked(black_box(&frame_imx464_rgb), 8).unwrap())
    });

    // mono debayer path with 4 chunks
    group_chunked.bench_function("imx464_mono_4chunks", |b| {
        b.iter(|| encode_rgb8_lz4_chunked(black_box(&frame_imx464_mono), 4).unwrap())
    });

    group_chunked.finish();

    // --- LZ4-only (no debayer, no f32→u8 conversion) ---
    let rgb8_imx464 = frame_imx464_rgb.to_rgb8_fast();

    let mut group_lz4 = c.benchmark_group("lz4_only");
    group_lz4.sample_size(10);
    group_lz4.warm_up_time(Duration::from_millis(500));
    group_lz4.measurement_time(Duration::from_secs(2));

    group_lz4.bench_function("lz4_imx464", |b| {
        b.iter(|| {
            lz4_flex::compress_prepend_size(black_box(&rgb8_imx464))
        })
    });

    group_lz4.finish();
}

criterion_group!(benches, bench_encoding);
criterion_main!(benches);
