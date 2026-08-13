use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use lz4_flex::decompress_size_prepended;
use night_amplifier::frame::Frame;
use night_amplifier::render::{RenderPipeline, RenderPipelineConfig};
use night_amplifier::server::{encode_rgb8_lz4, encode_rgb8_lz4_chunked};
use night_amplifier::PixelFormat;
use serial_test::serial;

use crate::integration::common::FIXTURES_DIR;
use crate::integration::image_loading::load_tiff;

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_encode_imx464_no_downsample() {
    println!("\n=== Encoding Test: IMX464 No Downsample ===\n");
    let width = 2712;
    let height = 1538;
    let frame = Frame::zeros(width, height, 3).unwrap();

    let encoded = encode_rgb8_lz4(&frame).unwrap();
    let enc_width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let enc_height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);

    // IMX464 should not be downsampled since it is < 3840x2160
    assert_eq!(enc_width as usize, width);
    assert_eq!(enc_height as usize, height);

    let decompressed = decompress_size_prepended(&encoded[16..]).unwrap();
    assert_eq!(decompressed.len(), width * height * 3);
}

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_encode_8k_downsamples_to_4k() {
    println!("\n=== Encoding Test: 8K Downsamples to 4K ===\n");
    let width = 7680;
    let height = 4320;
    let mut frame = Frame::zeros(width, height, 3).unwrap();

    // Set a known pattern to verify downsampling math
    for y in 0..height {
        for x in 0..width {
            frame.set_pixel(x, y, 0, 0.5);
        }
    }

    let encoded = encode_rgb8_lz4(&frame).unwrap();
    let enc_width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let enc_height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);

    // 8K should be downsampled by 2x to 4K (3840x2160)
    assert_eq!(enc_width as usize, 3840);
    assert_eq!(enc_height as usize, 2160);

    let decompressed = decompress_size_prepended(&encoded[16..]).unwrap();
    assert_eq!(decompressed.len(), 3840 * 2160 * 3);

    // The average of 0 and 1 should be around 0.5 (which scales to ~128)
    let val = decompressed[0];
    assert!((val as i32 - 128).abs() <= 1);
}

// ============================================================================
// Live-view streaming baseline on real fixtures
// ============================================================================

/// Fixture sets used for the streaming baseline (raw mono Bayer frames).
const BASELINE_FIXTURE_PREFIXES: &[&str] = &["35mm", "130mm", "250mm"];

/// Assumed effective network throughput for the implied-fps column (megabits/s).
const BASELINE_NETWORK_MBPS: f64 = 60.0;

const ENCODE_TIMING_ITERATIONS: u32 = 5;

fn baseline_fixture_dirs() -> Vec<PathBuf> {
    let fixtures = Path::new(FIXTURES_DIR);
    let mut dirs: Vec<PathBuf> = fs::read_dir(fixtures)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|name| {
                    BASELINE_FIXTURE_PREFIXES
                        .iter()
                        .any(|p| name.starts_with(p))
                })
                .unwrap_or(false)
        })
        .collect();
    dirs.sort();
    dirs
}

/// Loads a grayscale PNG as a raw Bayer frame, matching the simulated-camera loader.
fn load_png_mono(path: &Path) -> Result<Frame, String> {
    let file = fs::File::open(path).map_err(|e| format!("open {:?}: {}", path, e))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("read PNG info {:?}: {}", path, e))?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("decode PNG {:?}: {}", path, e))?;

    let (width, height) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    match (info.color_type, info.bit_depth) {
        (png::ColorType::Grayscale, png::BitDepth::Sixteen) => {
            Frame::from_raw(bytes, width, height, 1, PixelFormat::Bayer16Be)
        }
        (png::ColorType::Grayscale, png::BitDepth::Eight) => {
            Frame::from_raw(bytes, width, height, 1, PixelFormat::Bayer8)
        }
        (ct, bd) => {
            return Err(format!(
                "unsupported PNG format {:?}/{:?} in {:?}",
                ct, bd, path
            ))
        }
    }
    .map_err(|e| format!("create frame from {:?}: {}", path, e))
}

/// Loads the first frame (sorted by name) of a fixture directory.
fn load_first_fixture_frame(dir: &Path) -> Option<(PathBuf, Frame)> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .as_deref(),
                Some("png") | Some("tif") | Some("tiff")
            )
        })
        .collect();
    files.sort();
    let path = files.into_iter().next()?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    let frame = match ext.as_deref() {
        Some("png") => load_png_mono(&path),
        _ => load_tiff(&path).map(|img| img.frame),
    };

    match frame {
        Ok(frame) => Some((path, frame)),
        Err(e) => {
            println!("  Failed to load {:?}: {}", path, e);
            None
        }
    }
}

/// Encodes once for size, then times `ENCODE_TIMING_ITERATIONS` runs.
/// Returns (avg_ms, encoded_wire_bytes).
fn time_encode(frame: &Frame, chunks: usize) -> (f64, usize) {
    let encoded = encode_rgb8_lz4_chunked(frame, chunks).expect("encode failed");
    let size = encoded.len();

    let start = Instant::now();
    for _ in 0..ENCODE_TIMING_ITERATIONS {
        let _ = encode_rgb8_lz4_chunked(frame, chunks).expect("encode failed");
    }
    let avg_ms = start.elapsed().as_secs_f64() * 1000.0 / ENCODE_TIMING_ITERATIONS as f64;
    (avg_ms, size)
}

struct BaselineRow {
    name: String,
    width: usize,
    height: usize,
    raw_rgb8_bytes: usize,
    linear_wire_bytes: usize,
    stretched_wire_bytes: usize,
    render_ms: f64,
    encode_ms_live: f64,
    encode_ms_stacking: f64,
}

/// Baseline for the current live-view streaming path on real capture data.
///
/// Mirrors the production render task: raw mono Bayer frame → preview render
/// (background subtraction + autostretch + contrast, i.e. default settings) →
/// `encode_rgb8_lz4_chunked` (debayer + f32→RGB8 + LZ4 inside). Reports wire
/// size, compression ratio and implied fps on a saturated link.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --release --test integration_pipeline -- --ignored --test-threads=1"]
fn baseline_live_view_stream_encoding() {
    println!("\n=== Live View Streaming Baseline (current RGB8+LZ4 approach) ===\n");

    let dirs = baseline_fixture_dirs();
    if dirs.is_empty() {
        println!(
            "No baseline fixture sets found in {}. Skipping.",
            FIXTURES_DIR
        );
        return;
    }

    // Mirror render_task chunk selection: max parallelism live, 1 while stacking
    let live_chunks = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);

    let mut rows: Vec<BaselineRow> = Vec::new();

    for dir in dirs {
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Some((path, frame)) = load_first_fixture_frame(&dir) else {
            println!("{}: no loadable frame, skipping", name);
            continue;
        };
        println!(
            "{} ({:?}, {}x{}, {} ch)",
            name,
            path.file_name().unwrap_or_default(),
            frame.width(),
            frame.height(),
            frame.channels()
        );

        // Encode of the *linear* (unstretched) frame — isolates how much the
        // stretch costs in LZ4 compressibility.
        let linear_encoded = encode_rgb8_lz4_chunked(&frame, live_chunks).expect("linear encode");

        // Preview render with default capture settings (live-view path)
        let mut preview = frame.clone();
        let pipeline =
            RenderPipeline::new(RenderPipelineConfig::new().with_background_subtraction(true));
        let render_start = Instant::now();
        pipeline
            .process(&mut preview)
            .expect("preview render failed");
        let render_ms = render_start.elapsed().as_secs_f64() * 1000.0;

        let (encode_ms_live, stretched_wire_bytes) = time_encode(&preview, live_chunks);
        let (encode_ms_stacking, _) = time_encode(&preview, 1);

        rows.push(BaselineRow {
            name,
            width: frame.width(),
            height: frame.height(),
            raw_rgb8_bytes: frame.width() * frame.height() * 3,
            linear_wire_bytes: linear_encoded.len(),
            stretched_wire_bytes,
            render_ms,
            encode_ms_live,
            encode_ms_stacking,
        });
    }

    if rows.is_empty() {
        println!("No fixture sets produced results.");
        return;
    }

    let network_bytes_per_sec = BASELINE_NETWORK_MBPS * 1e6 / 8.0;
    println!(
        "\n--- Baseline Summary ({} chunks live / 1 chunk stacking) ---\n",
        live_chunks
    );
    println!(
        "{:<38} {:>10} {:>11} {:>11} {:>7} {:>9} {:>9} {:>10} {:>9}",
        "fixture",
        "dims",
        "rawRGB8 MB",
        "wire MB",
        "ratio",
        "linear MB",
        "render ms",
        "encode ms",
        "fps@60Mb"
    );
    for r in &rows {
        let mb = |b: usize| b as f64 / 1e6;
        println!(
            "{:<38} {:>10} {:>11.2} {:>11.2} {:>7.2} {:>9.2} {:>9.1} {:>10.1} {:>9.2}",
            r.name,
            format!("{}x{}", r.width, r.height),
            mb(r.raw_rgb8_bytes),
            mb(r.stretched_wire_bytes),
            r.raw_rgb8_bytes as f64 / r.stretched_wire_bytes as f64,
            mb(r.linear_wire_bytes),
            r.render_ms,
            r.encode_ms_live,
            network_bytes_per_sec / r.stretched_wire_bytes as f64,
        );
    }
    println!(
        "\nencode ms = SA09 encode (debayer + f32→RGB8 + LZ4), avg of {} runs, live chunk count.",
        ENCODE_TIMING_ITERATIONS
    );
    for r in &rows {
        println!(
            "{:<38} 1-chunk (stacking) encode: {:.1} ms",
            r.name, r.encode_ms_stacking
        );
    }
    println!(
        "\nfps@60Mb = frames/s that fit through a {} Mb/s link at the measured wire size.",
        BASELINE_NETWORK_MBPS
    );
    println!("=== Baseline Complete ===\n");
}

// ============================================================================
// Candidate probe: lossy JPEG vs the LZ4 baseline on the same fixtures
// ============================================================================

/// 2x2 box downsample of an RGB8 buffer (probe-only, approximates a
/// phone-resolution stream at half linear resolution).
fn downsample_rgb8_2x2(rgb8: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let out_w = width / 2;
    let out_h = height / 2;
    let mut out = vec![0u8; out_w * out_h * 3];
    for y in 0..out_h {
        for x in 0..out_w {
            for c in 0..3 {
                let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .iter()
                    .map(|&(dx, dy)| rgb8[((y * 2 + dy) * width + x * 2 + dx) * 3 + c] as u32)
                    .sum();
                out[(y * out_w + x) * 3 + c] = (sum / 4) as u8;
            }
        }
    }
    (out, out_w, out_h)
}

/// Encodes RGB8 as JPEG at `quality`, returns (avg_ms, bytes).
fn time_jpeg(rgb8: &[u8], width: usize, height: usize, quality: u8) -> (f64, usize) {
    use image::codecs::jpeg::JpegEncoder;
    use image::ExtendedColorType;

    let encode = || {
        let mut out = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut out, quality);
        encoder
            .encode(rgb8, width as u32, height as u32, ExtendedColorType::Rgb8)
            .expect("jpeg encode failed");
        out
    };

    let size = encode().len();
    let start = Instant::now();
    for _ in 0..ENCODE_TIMING_ITERATIONS {
        let _ = encode();
    }
    let avg_ms = start.elapsed().as_secs_f64() * 1000.0 / ENCODE_TIMING_ITERATIONS as f64;
    (avg_ms, size)
}

/// Probes lossy JPEG (already a dependency via the `image` crate) against the
/// LZ4 baseline on the same rendered fixtures: full resolution at q90/q80 and
/// half resolution at q80 (phone-screen scenario).
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --release --test integration_pipeline -- --ignored --test-threads=1"]
fn probe_jpeg_encoding_candidates() {
    println!("\n=== Candidate Probe: JPEG vs RGB8+LZ4 baseline ===\n");

    let dirs = baseline_fixture_dirs();
    if dirs.is_empty() {
        println!(
            "No baseline fixture sets found in {}. Skipping.",
            FIXTURES_DIR
        );
        return;
    }

    let live_chunks = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);
    let network_bytes_per_sec = BASELINE_NETWORK_MBPS * 1e6 / 8.0;

    println!(
        "{:<38} {:>16} {:>10} {:>10} {:>9}",
        "fixture", "candidate", "size MB", "enc ms", "fps@60Mb"
    );

    for dir in dirs {
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Some((_, frame)) = load_first_fixture_frame(&dir) else {
            println!("{}: no loadable frame, skipping", name);
            continue;
        };

        let mut preview = frame.clone();
        let pipeline =
            RenderPipeline::new(RenderPipelineConfig::new().with_background_subtraction(true));
        pipeline
            .process(&mut preview)
            .expect("preview render failed");

        // Mirrors what `frame_to_rgb8` does: a 1-channel frame at encode time is
        // genuine monochrome (colour is debayered at capture), so the channel is
        // replicated across RGB rather than debayered.
        let rgb8 = if preview.channels() == 1 {
            preview
                .data()
                .iter()
                .flat_map(|&v| {
                    let val = (v.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                    [val, val, val]
                })
                .collect()
        } else {
            preview.to_rgb8_fast()
        };
        let (w, h) = (preview.width(), preview.height());

        let (lz4_ms, lz4_size) = time_encode(&preview, live_chunks);
        let (half_rgb8, half_w, half_h) = downsample_rgb8_2x2(&rgb8, w, h);

        let candidates: Vec<(String, f64, usize)> = vec![
            (format!("LZ4 {}x{}", w, h), lz4_ms, lz4_size),
            {
                let (ms, size) = time_jpeg(&rgb8, w, h, 90);
                (format!("JPEG q90 {}x{}", w, h), ms, size)
            },
            {
                let (ms, size) = time_jpeg(&rgb8, w, h, 80);
                (format!("JPEG q80 {}x{}", w, h), ms, size)
            },
            {
                let (ms, size) = time_jpeg(&half_rgb8, half_w, half_h, 80);
                (format!("JPEG q80 {}x{}", half_w, half_h), ms, size)
            },
        ];

        for (label, ms, size) in candidates {
            println!(
                "{:<38} {:>16} {:>10.2} {:>10.1} {:>9.2}",
                name,
                label,
                size as f64 / 1e6,
                ms,
                network_bytes_per_sec / size as f64,
            );
        }
        println!();
    }

    println!(
        "JPEG = single-threaded `image` crate encoder (pure Rust); libjpeg-turbo would be faster."
    );
    println!("=== Probe Complete ===\n");
}

// ============================================================================
// Render-task stage breakdown
// ============================================================================

/// Times each stage the render task runs per frame, to locate the live-view FPS
/// ceiling once bandwidth is no longer the constraint (e.g. ethernet).
///
/// Mirrors production live view: debayered 3-channel frame → `process_preview_frame`
/// (autostretch + contrast, background subtraction off) → per-tier JPEG encode.
/// Also times the pipeline on a pre-downsampled frame to size up how much of the
/// stretch cost is spent on pixels no client ever sees.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --release --test integration_pipeline -- --ignored --test-threads=1"]
fn probe_render_task_stage_breakdown() {
    use night_amplifier::render::{AutoStretchConfig, StretchAggressiveness};
    use night_amplifier::server::encode_rgb8_jpeg_bounded;

    println!("\n=== Render Task Stage Breakdown (live view, per frame) ===\n");

    // Live-view UI settings: Auto Stretch High, background subtraction off.
    let live_config = || {
        RenderPipelineConfig::new()
            .with_background_subtraction(false)
            .with_stretch_config(AutoStretchConfig::from_profile(
                false,
                StretchAggressiveness::High,
            ))
            .with_auto_stretch(true)
            .with_contrast(true)
    };

    let time_ms = |mut f: Box<dyn FnMut()>| {
        f();
        let start = Instant::now();
        for _ in 0..ENCODE_TIMING_ITERATIONS {
            f();
        }
        start.elapsed().as_secs_f64() * 1000.0 / ENCODE_TIMING_ITERATIONS as f64
    };

    for dir in baseline_fixture_dirs() {
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Some((_, bayer)) = load_first_fixture_frame(&dir) else {
            continue;
        };
        let Ok((rgb, _)) = night_amplifier::debayer::debayer_auto(&bayer) else {
            println!("{}: debayer failed, skipping", name);
            continue;
        };

        println!(
            "{} — {}x{} x{} ch",
            name,
            rgb.width(),
            rgb.height(),
            rgb.channels()
        );

        let render_ms = time_ms(Box::new({
            let rgb = rgb.clone();
            move || {
                let mut preview = rgb.clone();
                RenderPipeline::new(live_config())
                    .process(&mut preview)
                    .unwrap();
            }
        }));
        let clone_ms = time_ms(Box::new({
            let rgb = rgb.clone();
            move || {
                let _ = rgb.clone();
            }
        }));

        // Rendered frame is what the JPEG tiers actually encode.
        let mut rendered = rgb.clone();
        RenderPipeline::new(live_config())
            .process(&mut rendered)
            .unwrap();

        println!(
            "  preview render (autostretch+contrast) {:>8.1} ms   (frame clone alone {:.1} ms)",
            render_ms - clone_ms,
            clone_ms
        );

        for (label, (bw, bh)) in [
            ("Hd1080  (S22)", (1920u32, 1080u32)),
            ("Qhd1440", (2560, 1440)),
            ("Uhd2160 (native here)", (3840, 2160)),
        ] {
            let payload = encode_rgb8_jpeg_bounded(&rendered, bw, bh).unwrap();
            let w = u32::from_le_bytes(payload[4..8].try_into().unwrap());
            let h = u32::from_le_bytes(payload[8..12].try_into().unwrap());
            let ms = time_ms(Box::new({
                let rendered = rendered.clone();
                move || {
                    let _ = encode_rgb8_jpeg_bounded(&rendered, bw, bh).unwrap();
                }
            }));
            let mbits = payload.len() as f64 * 8.0 / 1e6;
            println!(
                "  tier {:<22} {:>4}x{:<4} {:>7.1} ms  {:>6.2} MB   link-bound {:>5.1} fps @ {} Mb/s",
                label,
                w,
                h,
                ms,
                payload.len() as f64 / 1e6,
                BASELINE_NETWORK_MBPS / mbits,
                BASELINE_NETWORK_MBPS as u32
            );
        }
    }

    println!("=== Breakdown Complete ===\n");
}

// ============================================================================
// Fused render kernel — intermediate clamp impact
// ============================================================================

/// Quantifies, on real fixtures, how far the shipped fused stretch+contrast kernel
/// (Phase 2 of the live-view performance plan) diverges from the three separate passes
/// it replaces.
///
/// Three passes: `clamp(clamp(c * s_stretch) * s_contrast)`.
/// Fused:        `clamp(c * s_stretch * s_contrast)`, scale read from an interpolated LUT.
///
/// Two distinct sources of difference, reported separately because they carry completely
/// different weight:
///
/// - **Clipping pixels** — a channel exceeded 1.0 *between* the stages, so the reference
///   fed a clamped (and therefore too dark) luminance into contrast. This is the accepted
///   trade of the fusion and can move a blown star core by tens of LSB.
/// - **Everything else** — pure LUT quantisation, which must stay under a single LSB.
///   This is the number that regresses if the LUT loses interpolation or if entry 0 stops
///   carrying the curve's `L -> 0` limit.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --release --test integration_pipeline -- --ignored --test-threads=1"]
fn probe_fused_render_clamp_difference() {
    use night_amplifier::render::{
        apply_contrast_frame, auto_stretch_frame, AutoStretchConfig, ContrastConfig,
        StretchAggressiveness,
    };

    const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

    println!("\n=== Fused render kernel vs. the three passes it replaces ===\n");

    for dir in baseline_fixture_dirs() {
        let name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Some((_, bayer)) = load_first_fixture_frame(&dir) else {
            continue;
        };
        let Ok((rgb, _)) = night_amplifier::debayer::debayer_auto(&bayer) else {
            continue;
        };

        let stretch_config = AutoStretchConfig::from_profile(false, StretchAggressiveness::High);
        let contrast_config = ContrastConfig::default();

        // Reference: the three separate passes, i.e. stretch with no contrast fused in,
        // then contrast as its own luminance-preserving pass. The intermediate is captured
        // so we can tell which pixels the stretch stage clamped.
        let mut reference = rgb.clone();
        let stretch = auto_stretch_frame(&mut reference, stretch_config, None).unwrap();
        let after_stretch = reference.data().to_vec();
        apply_contrast_frame(&mut reference, &contrast_config).unwrap();

        // Fused: the shipped kernel, driven exactly as RenderPipeline drives it. Running the
        // real code rather than a hand-written model of it is the point — a reimplementation
        // here would measure an implementation nobody ships.
        let mut fused = rgb.clone();
        auto_stretch_frame(&mut fused, stretch_config, Some(&contrast_config)).unwrap();

        let midtone = stretch.stretch_factor;
        let black_point = stretch.black_point;

        // Compare in 8-bit, which is what the JPEG stream actually carries.
        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 as i32;

        let mut clipped_px = 0u64;
        let mut clipped_changed_px = 0u64;
        let mut clipped_max_delta = 0i32;
        let mut clipped_sum_delta = 0i64;
        let mut clipped_changed_samples = 0u64;
        let mut clipped_brighter = 0u64;
        let mut clipped_darker = 0u64;

        let mut clean_changed_px = 0u64;
        let mut clean_changed_samples = 0u64;
        let mut clean_max_delta = 0i32;
        let mut clean_min_lum = f32::MAX;

        let total_px = (reference.data().len() / 3) as u64;

        for ((pa, pb), mid) in reference
            .data()
            .chunks_exact(3)
            .zip(fused.data().chunks_exact(3))
            .zip(after_stretch.chunks_exact(3))
        {
            // Clipping is a per-pixel property: one clamped channel drags down the
            // luminance the reference feeds into contrast, moving all three channels.
            let clipped = mid.iter().any(|&v| v >= 1.0);
            let mut px_changed = false;
            let mut px_max = 0i32;

            for i in 0..3 {
                let d = to_u8(pb[i]) - to_u8(pa[i]);
                if d == 0 {
                    continue;
                }
                px_changed = true;
                px_max = px_max.max(d.abs());
                if clipped {
                    clipped_changed_samples += 1;
                    clipped_sum_delta += d as i64;
                    if d > 0 {
                        clipped_brighter += 1
                    } else {
                        clipped_darker += 1
                    }
                } else {
                    clean_changed_samples += 1;
                }
            }

            if clipped {
                clipped_px += 1;
                if px_changed {
                    clipped_changed_px += 1;
                    clipped_max_delta = clipped_max_delta.max(px_max);
                }
            } else if px_changed {
                clean_changed_px += 1;
                clean_max_delta = clean_max_delta.max(px_max);
                let lum = LUMA[0] * pa[0] + LUMA[1] * pa[1] + LUMA[2] * pa[2];
                clean_min_lum = clean_min_lum.min(lum);
            }
        }

        println!("{} — {}x{}", name, rgb.width(), rgb.height());
        println!("  midtone {:.4}  black_point {:.4}", midtone, black_point);
        println!(
            "  clipping between stages: {} of {} px ({:.4} %) — accepted divergence",
            clipped_px,
            total_px,
            clipped_px as f64 / total_px as f64 * 100.0
        );
        if clipped_changed_px > 0 {
            println!(
                "    changed {} px, max delta {} / 255, mean signed {:+.3} LSB, \
                 brighter {} / darker {}",
                clipped_changed_px,
                clipped_max_delta,
                clipped_sum_delta as f64 / clipped_changed_samples as f64,
                clipped_brighter,
                clipped_darker
            );
        }
        println!(
            "  non-clipping: {} of {} px changed ({:.4} %) — LUT quantisation only",
            clean_changed_px,
            total_px - clipped_px,
            clean_changed_px as f64 / (total_px - clipped_px).max(1) as f64 * 100.0
        );
        if clean_changed_px > 0 {
            println!(
                "    {} samples, max delta {} / 255, dimmest affected luminance {:.4}",
                clean_changed_samples, clean_max_delta, clean_min_lum
            );
        }

        // The whole point of interpolating the LUT: away from clipping the fused kernel has
        // to be indistinguishable from the exact three-pass math at 8-bit output.
        assert!(
            clean_max_delta <= 1,
            "{}: non-clipping pixels differ by up to {} LSB — LUT accuracy regressed",
            name,
            clean_max_delta
        );
        println!();
    }

    println!("=== Probe Complete ===\n");
}
