fn to_ready_frame(
    frame: &night_amplifier::frame::Frame,
) -> night_amplifier::server::state::RenderReadyFrame {
    let mut config = night_amplifier::render::RenderPipelineConfig::default();
    config.contrast = false;
    config.auto_stretch = false;
    config.saturation_boost = false;
    night_amplifier::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame.clone()),
        pipeline_config: config,
        stretch_result: None,
    }
}

use criterion::{criterion_group, criterion_main, Criterion};
use image::imageops::FilterType as ResizeFilterType;
use image::{
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
    ColorType, ImageEncoder,
};
use night_amplifier::frame::Frame;
use night_amplifier::server::encoding::frame_to_rgb8_downsampled;
use night_amplifier::server::{encode_rgb8_jpeg_dynamic, encode_rgb8_lz4, encode_rgb8_lz4_chunked};
use std::fs;
use std::hint::black_box;
use std::io::Cursor;
use std::path::Path;
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

    // IMX464 resolution (1 channel — mono sensor, no downsampling at 4K box)
    let frame_imx464_mono = create_test_frame(2712, 1538, 1);

    // ASI1600MM (4656x3520 mono) — a large mono sensor, so the JPEG tiers must
    // box-average it down. This is the case that used to debayer at full
    // resolution into a ~196 MB f32 RGB frame before emitting grey pixels.
    let frame_mono_large = create_test_frame(4656, 3520, 1);

    // 4K resolution (to test downsampling threshold)
    let frame_4k = create_test_frame(3840, 2160, 3);

    // 8K resolution (simulating a very large sensor that will trigger downsampling)
    let frame_8k = create_test_frame(7680, 4320, 3);

    let mut group = c.benchmark_group("encode_rgb8_lz4");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("encode_imx464_rgb", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&to_ready_frame(&frame_imx464_rgb))).unwrap())
    });

    group.bench_function("encode_imx464_mono", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&to_ready_frame(&frame_imx464_mono))).unwrap())
    });

    group.bench_function("encode_4k", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&to_ready_frame(&frame_4k))).unwrap())
    });

    group.bench_function("encode_8k", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&to_ready_frame(&frame_8k))).unwrap())
    });

    group.finish();

    // --- frame_to_rgb8_downsampled ---
    let mut group_conv = c.benchmark_group("frame_to_rgb8");
    group_conv.sample_size(10);
    group_conv.warm_up_time(Duration::from_millis(500));
    group_conv.measurement_time(Duration::from_secs(2));

    group_conv.bench_function("imx464_to_native", |b| {
        b.iter(|| {
            frame_to_rgb8_downsampled(black_box(&to_ready_frame(&frame_imx464_rgb)), 3840, 2160)
                .unwrap()
        })
    });

    group_conv.bench_function("8k_to_4k", |b| {
        b.iter(|| {
            frame_to_rgb8_downsampled(black_box(&to_ready_frame(&frame_8k)), 3840, 2160).unwrap()
        })
    });

    group_conv.finish();

    // --- Chunked LZ4 (SA09) ---
    let mut group_chunked = c.benchmark_group("encode_chunked_lz4");
    group_chunked.sample_size(10);
    group_chunked.warm_up_time(Duration::from_millis(500));
    group_chunked.measurement_time(Duration::from_secs(2));

    // 1 chunk = stacking mode (sequential, no parallelism)
    group_chunked.bench_function("imx464_rgb_1chunk", |b| {
        b.iter(|| {
            encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_rgb)), 1).unwrap()
        })
    });

    // 4 chunks = Raspberry Pi 5 (4 cores)
    group_chunked.bench_function("imx464_rgb_4chunks", |b| {
        b.iter(|| {
            encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_rgb)), 4).unwrap()
        })
    });

    // 8 chunks = max parallelism
    group_chunked.bench_function("imx464_rgb_8chunks", |b| {
        b.iter(|| {
            encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_rgb)), 8).unwrap()
        })
    });

    // mono grey-replication path with 4 chunks
    group_chunked.bench_function("imx464_mono_4chunks", |b| {
        b.iter(|| {
            encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_mono)), 4).unwrap()
        })
    });

    group_chunked.finish();

    // --- LZ4-only (no debayer, no f32→u8 conversion) ---
    let rgb8_imx464 = frame_imx464_rgb.to_rgb8_fast();

    let mut group_lz4 = c.benchmark_group("lz4_only");
    group_lz4.sample_size(10);
    group_lz4.warm_up_time(Duration::from_millis(500));
    group_lz4.measurement_time(Duration::from_secs(2));

    group_lz4.bench_function("lz4_imx464", |b| {
        b.iter(|| lz4_flex::compress_prepend_size(black_box(&rgb8_imx464)))
    });

    group_lz4.finish();

    // --- Dynamic JPEG (SA10) ---
    let mut group_jpeg = c.benchmark_group("encode_jpeg_dynamic");
    group_jpeg.sample_size(10);
    group_jpeg.warm_up_time(Duration::from_millis(500));
    group_jpeg.measurement_time(Duration::from_secs(2));

    group_jpeg.bench_function("imx464_rgb_to_1080p", |b| {
        b.iter(|| {
            encode_rgb8_jpeg_dynamic(
                black_box(&to_ready_frame(&frame_imx464_rgb)),
                Some(1920),
                Some(1080),
            )
            .unwrap()
        })
    });

    group_jpeg.bench_function("imx464_rgb_to_720p", |b| {
        b.iter(|| {
            encode_rgb8_jpeg_dynamic(
                black_box(&to_ready_frame(&frame_imx464_rgb)),
                Some(1280),
                Some(720),
            )
            .unwrap()
        })
    });

    group_jpeg.bench_function("imx464_rgb_full_res", |b| {
        b.iter(|| {
            encode_rgb8_jpeg_dynamic(black_box(&to_ready_frame(&frame_imx464_rgb)), None, None)
                .unwrap()
        })
    });

    // Mono sensor large enough to need downsampling — the path that previously
    // ran a full-resolution debayer to produce grey output.
    group_jpeg.bench_function("mono_asi1600mm_to_1080p", |b| {
        b.iter(|| {
            encode_rgb8_jpeg_dynamic(
                black_box(&to_ready_frame(&frame_mono_large)),
                Some(1920),
                Some(1080),
            )
            .unwrap()
        })
    });

    group_jpeg.finish();
}

// ---------------------------------------------------------
// EXTENDED BENCHMARKS
// ---------------------------------------------------------

fn save_output(picture_name: &str, resolution: &str, name: &str, ext: &str, data: &[u8]) {
    let path_dir = Path::new("tests/fixtures/processed")
        .join(picture_name)
        .join(resolution);
    fs::create_dir_all(&path_dir).unwrap();
    let file_path = path_dir.join(format!("{}.{}", name, ext));
    fs::write(file_path, data).unwrap();
}

fn load_and_resize_fixture(path: &str, width: u32, height: u32) -> Vec<u8> {
    let img = image::open(path).expect(&format!("Failed to open {}", path));
    let resized = img.resize_exact(width, height, ResizeFilterType::Lanczos3);
    resized.to_rgb8().into_raw()
}

fn run_benchmarks(
    c: &mut Criterion,
    picture_name: &str,
    res_name: &str,
    rgb8_data: &[u8],
    width: usize,
    height: usize,
) {
    let group_name = format!("{}_{}", picture_name, res_name);

    // Helper to get TurboJPEG size and save
    let save_tj = |quality: i32| {
        let mut compressor = turbojpeg::Compressor::new().unwrap();
        compressor.set_quality(quality).unwrap();
        compressor.set_subsamp(turbojpeg::Subsamp::Sub2x2).unwrap();
        let image_tj = turbojpeg::Image {
            pixels: rgb8_data,
            width,
            pitch: 3 * width,
            height,
            format: turbojpeg::PixelFormat::RGB,
        };
        let out = compressor.compress_to_vec(image_tj).unwrap();
        save_output(
            picture_name,
            res_name,
            &format!("turbojpeg_{}", quality),
            "jpg",
            &out,
        );
        out.len()
    };

    println!("\n=== {} ({}x{}) ===", group_name, width, height);
    println!("turbojpeg_100 size: {} bytes", save_tj(100));
    println!("turbojpeg_95 size: {} bytes", save_tj(95));
    println!("turbojpeg_90 size: {} bytes", save_tj(90));

    let lz4_out = lz4_flex::compress_prepend_size(rgb8_data);
    save_output(picture_name, res_name, "lz4_flex", "lz4", &lz4_out);
    println!("lz4_flex size: {} bytes", lz4_out.len());

    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);
    let encoder = JpegEncoder::new_with_quality(&mut cursor, 95);
    encoder
        .write_image(
            rgb8_data,
            width as u32,
            height as u32,
            ColorType::Rgb8.into(),
        )
        .unwrap();
    save_output(picture_name, res_name, "image_jpeg_95", "jpg", &buf);
    println!("image_jpeg_95 size: {} bytes", buf.len());

    // PNG (Lossless, using default compression)
    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);
    let encoder = PngEncoder::new(&mut cursor);
    encoder
        .write_image(
            rgb8_data,
            width as u32,
            height as u32,
            ColorType::Rgb8.into(),
        )
        .unwrap();
    save_output(picture_name, res_name, "image_png_lossless", "png", &buf);
    println!("image_png_lossless size: {} bytes", buf.len());

    let mut group = c.benchmark_group(format!("encoding_comparison_{}", group_name));
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("turbojpeg_100", |b| {
        b.iter(|| {
            let mut compressor = turbojpeg::Compressor::new().unwrap();
            let _ = compressor.set_quality(100);
            let _ = compressor.set_subsamp(turbojpeg::Subsamp::Sub2x2);
            let image = turbojpeg::Image {
                pixels: black_box(rgb8_data),
                width,
                pitch: 3 * width,
                height,
                format: turbojpeg::PixelFormat::RGB,
            };
            compressor.compress_to_vec(image).unwrap()
        })
    });

    group.bench_function("turbojpeg_95", |b| {
        b.iter(|| {
            let mut compressor = turbojpeg::Compressor::new().unwrap();
            let _ = compressor.set_quality(95);
            let _ = compressor.set_subsamp(turbojpeg::Subsamp::Sub2x2);
            let image = turbojpeg::Image {
                pixels: black_box(rgb8_data),
                width,
                pitch: 3 * width,
                height,
                format: turbojpeg::PixelFormat::RGB,
            };
            compressor.compress_to_vec(image).unwrap()
        })
    });

    group.bench_function("turbojpeg_90", |b| {
        b.iter(|| {
            let mut compressor = turbojpeg::Compressor::new().unwrap();
            let _ = compressor.set_quality(90);
            let _ = compressor.set_subsamp(turbojpeg::Subsamp::Sub2x2);
            let image = turbojpeg::Image {
                pixels: black_box(rgb8_data),
                width,
                pitch: 3 * width,
                height,
                format: turbojpeg::PixelFormat::RGB,
            };
            compressor.compress_to_vec(image).unwrap()
        })
    });

    group.bench_function("lz4_flex", |b| {
        b.iter(|| lz4_flex::compress_prepend_size(black_box(rgb8_data)))
    });

    group.bench_function("image_jpeg_95", |b| {
        b.iter(|| {
            let mut buffer = Vec::with_capacity(1024 * 1024 * 4);
            let mut cursor = Cursor::new(&mut buffer);
            let encoder = JpegEncoder::new_with_quality(&mut cursor, 95);
            encoder
                .write_image(
                    black_box(rgb8_data),
                    width as u32,
                    height as u32,
                    ColorType::Rgb8.into(),
                )
                .unwrap();
        })
    });

    group.bench_function("image_png_lossless", |b| {
        b.iter(|| {
            let mut buffer = Vec::with_capacity(1024 * 1024 * 4);
            let mut cursor = Cursor::new(&mut buffer);
            let encoder = PngEncoder::new(&mut cursor);
            encoder
                .write_image(
                    black_box(rgb8_data),
                    width as u32,
                    height as u32,
                    ColorType::Rgb8.into(),
                )
                .unwrap();
        })
    });

    group.finish();
}

fn process_image(c: &mut Criterion, picture_name: &str, path: &str) {
    // 1080p Downscale
    let rgb8_1080p = load_and_resize_fixture(path, 1920, 1080);
    run_benchmarks(c, picture_name, "1080p_downscale", &rgb8_1080p, 1920, 1080);

    // Original Resolution
    let rgb8_imx464 = load_and_resize_fixture(path, 2712, 1538);
    run_benchmarks(
        c,
        picture_name,
        "original_resolution",
        &rgb8_imx464,
        2712,
        1538,
    );

    // 4K Upscale
    let rgb8_4k = load_and_resize_fixture(path, 3840, 2160);
    run_benchmarks(c, picture_name, "4K_upscale", &rgb8_4k, 3840, 2160);
}

fn bench_encoding_extended(c: &mut Criterion) {
    // Ignore running on CI unless explicitly requested
    if std::env::var("RUN_EXTENDED_ENCODING_BENCH").is_err() {
        println!(
            "Skipping extended encoding benchmarks. Set RUN_EXTENDED_ENCODING_BENCH=1 to run."
        );
        return;
    }

    let images = [
        (
            "Dumbbell",
            "tests/fixtures/stacked/09-08-2026_12-36-18_stretched.png",
        ),
        (
            "Orion_Wide",
            "tests/fixtures/stacked/09-08-2026_12-36-48_stretched.png",
        ),
        (
            "Orion",
            "tests/fixtures/stacked/09-08-2026_12-38-14_stretched.png",
        ),
    ];

    for (name, path) in images.iter() {
        process_image(c, name, path);
    }
}

criterion_group!(benches, bench_encoding, bench_encoding_extended);
criterion_main!(benches);
