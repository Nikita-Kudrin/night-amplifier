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

use criterion::{criterion_group, criterion_main, Criterion, SamplingMode};
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

    // Groups here run a 1 s measurement window rather than 2 s: this binary carries 16
    // cases, and at 2 s each the wall clock reached 50 s against a ~30 s budget. Every
    // case clears the ~100 ms floor, so a 1 s window still collects ten samples of real
    // work. This is the tightest binary in the suite even so: `bench_encoding`'s 16
    // cases at ~100-220 ms each land the binary at ~32 s. Every REPS below is already the
    // minimum that clears the floor with margin, not padded further, and the mono LZ4
    // case (`encode_imx464_mono`) runs unusually noisy (~140-220 ms swings) for reasons
    // unrelated to this change.
    let mut group = c.benchmark_group("encode_rgb8_lz4");
    // Flat sampling: the 8K case is ~140 ms, and criterion's default linear scheme runs
    // 1+2+...+10 = 55 iterations per case, i.e. 8 s for that one alone. Flat runs a fixed
    // count per sample and keeps the binary inside its ~30 s budget.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(1));

    // ~21 ms per call. `encode_rgb8_lz4` takes the frame by reference and returns a
    // fresh buffer, so repeating it measures the same work each time.
    const RGB_REPS: usize = 5;
    group.bench_function(format!("encode_imx464_rgb_x{}", RGB_REPS), |b| {
        b.iter(|| {
            for _ in 0..RGB_REPS {
                black_box(encode_rgb8_lz4(black_box(&to_ready_frame(&frame_imx464_rgb))).unwrap());
            }
        })
    });

    // ~6.25 ms per call.
    const MONO_REPS: usize = 20;
    group.bench_function(format!("encode_imx464_mono_x{}", MONO_REPS), |b| {
        b.iter(|| {
            for _ in 0..MONO_REPS {
                black_box(encode_rgb8_lz4(black_box(&to_ready_frame(&frame_imx464_mono))).unwrap());
            }
        })
    });

    // ~42 ms per call.
    const FOUR_K_REPS: usize = 3;
    group.bench_function(format!("encode_4k_x{}", FOUR_K_REPS), |b| {
        b.iter(|| {
            for _ in 0..FOUR_K_REPS {
                black_box(encode_rgb8_lz4(black_box(&to_ready_frame(&frame_4k))).unwrap());
            }
        })
    });

    // Already ~145 ms on its own — clears the floor without repeating.
    group.bench_function("encode_8k", |b| {
        b.iter(|| encode_rgb8_lz4(black_box(&to_ready_frame(&frame_8k))).unwrap())
    });

    group.finish();

    // --- frame_to_rgb8_downsampled ---
    let mut group_conv = c.benchmark_group("frame_to_rgb8");
    group_conv.sampling_mode(SamplingMode::Flat);
    group_conv.sample_size(10);
    group_conv.warm_up_time(Duration::from_millis(500));
    group_conv.measurement_time(Duration::from_secs(1));

    // ~19 ms per call.
    const NATIVE_REPS: usize = 6;
    group_conv.bench_function(format!("imx464_to_native_x{}", NATIVE_REPS), |b| {
        b.iter(|| {
            for _ in 0..NATIVE_REPS {
                black_box(
                    frame_to_rgb8_downsampled(
                        black_box(&to_ready_frame(&frame_imx464_rgb)),
                        3840,
                        2160,
                    )
                    .unwrap(),
                );
            }
        })
    });

    // Already ~138 ms on its own — clears the floor without repeating.
    group_conv.bench_function("8k_to_4k", |b| {
        b.iter(|| {
            frame_to_rgb8_downsampled(black_box(&to_ready_frame(&frame_8k)), 3840, 2160).unwrap()
        })
    });

    group_conv.finish();

    // --- Chunked LZ4 (SA09) ---
    let mut group_chunked = c.benchmark_group("encode_chunked_lz4");
    group_chunked.sampling_mode(SamplingMode::Flat);
    group_chunked.sample_size(10);
    group_chunked.warm_up_time(Duration::from_millis(500));
    group_chunked.measurement_time(Duration::from_secs(1));

    // ~20-21 ms per call. 1 chunk = stacking mode (sequential, no parallelism)
    const CHUNK_REPS: usize = 6;
    group_chunked.bench_function(format!("imx464_rgb_1chunk_x{}", CHUNK_REPS), |b| {
        b.iter(|| {
            for _ in 0..CHUNK_REPS {
                black_box(
                    encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_rgb)), 1)
                        .unwrap(),
                );
            }
        })
    });

    // 4 chunks = Raspberry Pi 5 (4 cores)
    group_chunked.bench_function(format!("imx464_rgb_4chunks_x{}", CHUNK_REPS), |b| {
        b.iter(|| {
            for _ in 0..CHUNK_REPS {
                black_box(
                    encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_rgb)), 4)
                        .unwrap(),
                );
            }
        })
    });

    // 8 chunks = max parallelism
    group_chunked.bench_function(format!("imx464_rgb_8chunks_x{}", CHUNK_REPS), |b| {
        b.iter(|| {
            for _ in 0..CHUNK_REPS {
                black_box(
                    encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_rgb)), 8)
                        .unwrap(),
                );
            }
        })
    });

    // mono grey-replication path with 4 chunks. ~5.2 ms on its own; the encoder takes the
    // frame by reference and returns a fresh buffer.
    // **The reported `time:` is for `CHUNKED_MONO_REPS` encodes, not one.**
    const CHUNKED_MONO_REPS: usize = 22;
    group_chunked.bench_function(format!("imx464_mono_4chunks_x{}", CHUNKED_MONO_REPS), |b| {
        b.iter(|| {
            for _ in 0..CHUNKED_MONO_REPS {
                black_box(
                    encode_rgb8_lz4_chunked(black_box(&to_ready_frame(&frame_imx464_mono)), 4)
                        .unwrap(),
                );
            }
        })
    });

    group_chunked.finish();

    // --- LZ4-only (no debayer, no f32→u8 conversion) ---
    let rgb8_imx464 = frame_imx464_rgb.to_rgb8_fast();

    let mut group_lz4 = c.benchmark_group("lz4_only");
    group_lz4.sampling_mode(SamplingMode::Flat);
    group_lz4.sample_size(10);
    group_lz4.warm_up_time(Duration::from_millis(500));
    group_lz4.measurement_time(Duration::from_secs(1));

    // The compression step alone, without the debayer or the f32 -> u8 conversion, is
    // ~1.75 ms — this is the control the other LZ4 figures are read against, so it needs
    // to be at least as well resolved as they are.
    // **The reported `time:` is for `LZ4_REPS` compressions, not one.**
    const LZ4_REPS: usize = 64;
    group_lz4.bench_function(format!("lz4_imx464_x{}", LZ4_REPS), |b| {
        b.iter(|| {
            for _ in 0..LZ4_REPS {
                black_box(lz4_flex::compress_prepend_size(black_box(&rgb8_imx464)));
            }
        })
    });

    group_lz4.finish();

    // --- Dynamic JPEG (SA10) ---
    let mut group_jpeg = c.benchmark_group("encode_jpeg_dynamic");
    group_jpeg.sampling_mode(SamplingMode::Flat);
    group_jpeg.sample_size(10);
    group_jpeg.warm_up_time(Duration::from_millis(500));
    group_jpeg.measurement_time(Duration::from_secs(1));

    // ~29 ms per call for all four cases below.
    const JPEG_REPS: usize = 4;

    group_jpeg.bench_function(format!("imx464_rgb_to_1080p_x{}", JPEG_REPS), |b| {
        b.iter(|| {
            for _ in 0..JPEG_REPS {
                black_box(
                    encode_rgb8_jpeg_dynamic(
                        black_box(&to_ready_frame(&frame_imx464_rgb)),
                        Some(1920),
                        Some(1080),
                    )
                    .unwrap(),
                );
            }
        })
    });

    group_jpeg.bench_function(format!("imx464_rgb_to_720p_x{}", JPEG_REPS), |b| {
        b.iter(|| {
            for _ in 0..JPEG_REPS {
                black_box(
                    encode_rgb8_jpeg_dynamic(
                        black_box(&to_ready_frame(&frame_imx464_rgb)),
                        Some(1280),
                        Some(720),
                    )
                    .unwrap(),
                );
            }
        })
    });

    group_jpeg.bench_function(format!("imx464_rgb_full_res_x{}", JPEG_REPS), |b| {
        b.iter(|| {
            for _ in 0..JPEG_REPS {
                black_box(
                    encode_rgb8_jpeg_dynamic(black_box(&to_ready_frame(&frame_imx464_rgb)), None, None)
                        .unwrap(),
                );
            }
        })
    });

    // Mono sensor large enough to need downsampling — the path that previously
    // ran a full-resolution debayer to produce grey output.
    group_jpeg.bench_function(format!("mono_asi1600mm_to_1080p_x{}", JPEG_REPS), |b| {
        b.iter(|| {
            for _ in 0..JPEG_REPS {
                black_box(
                    encode_rgb8_jpeg_dynamic(
                        black_box(&to_ready_frame(&frame_mono_large)),
                        Some(1920),
                        Some(1080),
                    )
                    .unwrap(),
                );
            }
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

/// Repeats per measured iteration to clear the ~100 ms floor, sized from one real call's
/// duration rather than a hardcoded figure.
///
/// This group runs the same six codecs over nine image/resolution combinations, and per-
/// call cost varies by well over 100x across them (lz4_flex on a 1080p frame vs.
/// `image_jpeg_95` on a 4K one) — a single constant could not clear the floor for the
/// cheap cases without making the expensive ones blow the wall-clock budget. `elapsed` is
/// a real call already made for the size printout above, not a throwaway warm-up.
fn calibrated_reps(elapsed: Duration) -> usize {
    let target = Duration::from_millis(120);
    if elapsed.is_zero() {
        return 1;
    }
    (target.as_secs_f64() / elapsed.as_secs_f64()).ceil() as usize
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

    // Helper to get TurboJPEG size and save. Returns the call's own duration alongside
    // the size, so the one real compression already needed for the printout can also
    // size `REPS` for the benchmark below instead of a second throwaway call.
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
        let start = std::time::Instant::now();
        let out = compressor.compress_to_vec(image_tj).unwrap();
        let elapsed = start.elapsed();
        save_output(
            picture_name,
            res_name,
            &format!("turbojpeg_{}", quality),
            "jpg",
            &out,
        );
        (out.len(), elapsed)
    };

    println!("\n=== {} ({}x{}) ===", group_name, width, height);
    let (tj100_len, tj100_elapsed) = save_tj(100);
    println!("turbojpeg_100 size: {} bytes", tj100_len);
    let (tj95_len, tj95_elapsed) = save_tj(95);
    println!("turbojpeg_95 size: {} bytes", tj95_len);
    let (tj90_len, tj90_elapsed) = save_tj(90);
    println!("turbojpeg_90 size: {} bytes", tj90_len);

    let start = std::time::Instant::now();
    let lz4_out = lz4_flex::compress_prepend_size(rgb8_data);
    let lz4_elapsed = start.elapsed();
    save_output(picture_name, res_name, "lz4_flex", "lz4", &lz4_out);
    println!("lz4_flex size: {} bytes", lz4_out.len());

    let start = std::time::Instant::now();
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
    let jpeg95_elapsed = start.elapsed();
    save_output(picture_name, res_name, "image_jpeg_95", "jpg", &buf);
    println!("image_jpeg_95 size: {} bytes", buf.len());

    // PNG (Lossless, using default compression)
    let start = std::time::Instant::now();
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
    let png_elapsed = start.elapsed();
    save_output(picture_name, res_name, "image_png_lossless", "png", &buf);
    println!("image_png_lossless size: {} bytes", buf.len());

    let mut group = c.benchmark_group(format!("encoding_comparison_{}", group_name));
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(1));

    let tj100_reps = calibrated_reps(tj100_elapsed);
    group.bench_function(format!("turbojpeg_100_x{}", tj100_reps), |b| {
        b.iter(|| {
            for _ in 0..tj100_reps {
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
                black_box(compressor.compress_to_vec(image).unwrap());
            }
        })
    });

    let tj95_reps = calibrated_reps(tj95_elapsed);
    group.bench_function(format!("turbojpeg_95_x{}", tj95_reps), |b| {
        b.iter(|| {
            for _ in 0..tj95_reps {
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
                black_box(compressor.compress_to_vec(image).unwrap());
            }
        })
    });

    let tj90_reps = calibrated_reps(tj90_elapsed);
    group.bench_function(format!("turbojpeg_90_x{}", tj90_reps), |b| {
        b.iter(|| {
            for _ in 0..tj90_reps {
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
                black_box(compressor.compress_to_vec(image).unwrap());
            }
        })
    });

    let lz4_reps = calibrated_reps(lz4_elapsed);
    group.bench_function(format!("lz4_flex_x{}", lz4_reps), |b| {
        b.iter(|| {
            for _ in 0..lz4_reps {
                black_box(lz4_flex::compress_prepend_size(black_box(rgb8_data)));
            }
        })
    });

    let jpeg95_reps = calibrated_reps(jpeg95_elapsed);
    group.bench_function(format!("image_jpeg_95_x{}", jpeg95_reps), |b| {
        b.iter(|| {
            for _ in 0..jpeg95_reps {
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
            }
        })
    });

    let png_reps = calibrated_reps(png_elapsed);
    group.bench_function(format!("image_png_lossless_x{}", png_reps), |b| {
        b.iter(|| {
            for _ in 0..png_reps {
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
            }
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
