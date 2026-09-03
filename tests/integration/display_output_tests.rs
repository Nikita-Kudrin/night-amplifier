//! Tests for the display output path: the black floor and ordered dither that
//! the fused encoders apply where a frame becomes 8-bit, and the resolution the
//! lossless stream encodes into.
//!
//! These measure the quantity that actually predicts what an observer sees at
//! the eyepiece — sky sigma expressed in **output 8-bit levels**, taken from the
//! bytes that reach the browser rather than from the linear frame. Every other
//! measurement in this repo is taken before the stretch, where it says nothing
//! about visible grain.
//!
//! Both fixtures are measured, not just the square one. They are the two ends of
//! the case: an IMX533 at 3008² arrives at a 1440 screen with a 2.1x downsample
//! of free averaging available, an IMX464 at 2712x1538 with essentially none —
//! so a change that only helps because of the resample shows up as helping one
//! and not the other.

use std::path::Path;

use serial_test::serial;

use crate::integration::common::FIXTURES_DIR;
use crate::integration::image_loading::load_image;

/// A fixture, and the region of its 1440-tier encode a denoiser must not eat.
struct Fixture {
    dir: &'static str,
    label: &'static str,
    /// `(x0, y0, x1, y1)` in the 1440-tier encode, around the target. Located by
    /// sweeping for the brightest 320 px block of the green channel, then
    /// pinned here so the measurement is against a fixed region rather than
    /// against whatever the denoiser left brightest.
    target_box: (usize, usize, usize, usize),
    /// Whether the frame carries any colour at all.
    ///
    /// The IMX464 fixture does not: it is a raw RGGB mosaic saved as a 16-bit
    /// mono PNG, and `load_first_frame` reads it through `to_rgb16`, which hands
    /// back three identical channels — so `channels() == 1` never fires and it
    /// is never demosaiced. Its measured sky chroma is exactly 0.00 everywhere.
    /// Any colour-sensitive assertion has to run on the FITS fixture alone.
    is_colour: bool,
}

/// The IMX533 fixture is 3008x3008, 2 s at gain 300 on a 250 mm Dobsonian —
/// square, so the 1440 tier takes it to exactly 1440x1440. The IMX464 one is
/// 2712x1538 on a 130 mm, which the same tier barely shrinks at all.
const FIXTURES: [Fixture; 2] = [
    Fixture {
        dir: "250mm-dob-imx533-dumbbell-fits",
        label: "IMX533 3008^2, 250 mm Dob",
        target_box: (560, 560, 880, 880),
        is_colour: true,
    },
    Fixture {
        dir: "130mm-imx464-dumbell-nebulae-png",
        label: "IMX464 2712x1538, 130 mm",
        target_box: (960, 640, 1280, 960),
        is_colour: false,
    },
];

/// Sky sigma of one channel of an interleaved RGB8 buffer, in 8-bit levels.
///
/// A MAD sets the clip and a clipped standard deviation is what gets reported.
/// The MAD alone is what this used to return, and on byte samples it can only
/// take integer values — so the figure snapped to multiples of 1.4826 levels and
/// could not resolve any change smaller than one output level, which is most of
/// them. The clip is what keeps stars and the target out of the variance; the
/// standard deviation of what survives it is continuous.
fn sky_sigma_levels(rgb8: &[u8], channel: usize) -> f64 {
    let mut samples: Vec<f64> = rgb8
        .iter()
        .skip(channel)
        .step_by(3)
        .map(|&v| v as f64)
        .collect();
    if samples.len() < 2 {
        return 0.0;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples[samples.len() / 2];

    let mut deviations: Vec<f64> = samples.iter().map(|v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mad_sigma = deviations[deviations.len() / 2] * 1.4826;

    // A floor of one level, or a sky already smooth enough to have a zero MAD
    // would clip away everything including its own noise.
    let clip = (mad_sigma * 3.0).max(1.0);
    let kept: Vec<f64> = samples
        .iter()
        .copied()
        .filter(|v| (v - median).abs() <= clip)
        .collect();
    if kept.len() < 2 {
        return mad_sigma;
    }
    let mean = kept.iter().sum::<f64>() / kept.len() as f64;
    (kept.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / kept.len() as f64).sqrt()
}

/// First frame of a fixture directory, as a linear `Frame`.
///
/// The shared `find_image_files_in_dir` / `load_image` pair only knows TIFF and
/// FITS, so the PNG fixture sets are invisible to it. Teaching those two about
/// PNG would silently pull three more fixture sets into every test that walks
/// them; this stays local instead.
fn load_first_frame(dir: &Path) -> Option<night_amplifier::Frame> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    let first = files.first()?;

    let is_png = first
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("png"))
        .unwrap_or(false);

    if !is_png {
        return load_image(first).ok().map(|loaded| loaded.frame);
    }

    // Planar, like every other `Frame` in the codebase — an interleaved fill
    // here would scramble two thirds of the colour and still measure a
    // plausible-looking sigma.
    let img = image::open(first).ok()?.to_rgb16();
    let (width, height) = img.dimensions();
    let area = (width * height) as usize;
    let mut data = vec![0.0f32; area * 3];
    for (i, px) in img.pixels().enumerate() {
        for (c, sample) in px.0.iter().enumerate() {
            data[c * area + i] = *sample as f32 / 65535.0;
        }
    }
    night_amplifier::Frame::from_f32_vec(data, width as usize, height as usize, 3).ok()
}

/// Run the real preview pipeline over a fixture frame and hand back the
/// `RenderReadyFrame` the encoders take, so these tests exercise the same tone
/// curve the eyepiece does rather than a synthetic one.
fn prepare_fixture(
    fixture: &Fixture,
    intensity: f32,
) -> Option<night_amplifier::server::state::RenderReadyFrame> {
    prepare_fixture_with(fixture, |settings| settings.eyepiece.intensity = intensity)
}

/// [`prepare_fixture`], with the capture settings open to the caller.
///
/// The black floor is resolved inside `process_preview_frame`, against a sky
/// level nothing has measured until the solver runs — so a test that reaches
/// past the settings and writes the resolved transform onto the config is
/// testing a curve the product cannot produce.
fn prepare_fixture_with(
    fixture: &Fixture,
    configure: impl FnOnce(&mut night_amplifier::server::state::CaptureSettings),
) -> Option<night_amplifier::server::state::RenderReadyFrame> {
    // Ensure fixtures are downloaded from Google Drive. Under nextest
    // partitioning this file may run in a shard with no other test that
    // downloads fixtures first, so it has to do it itself.
    crate::integration::common::ensure_fixtures_sync();

    let dir = Path::new(FIXTURES_DIR).join(fixture.dir);
    let mut frame = load_first_frame(&dir)?;
    if frame.channels() == 1 {
        frame = night_amplifier::debayer_auto(&frame).ok()?.0;
    }

    let mut settings = night_amplifier::server::state::CaptureSettings::default();
    settings.auto_stretch = true;
    configure(&mut settings);

    let (pipeline_config, stretch_result) =
        night_amplifier::server::capture::pipeline::process_preview_frame(&mut frame, &settings)
            .ok()?;

    Some(night_amplifier::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config,
        stretch_result,
    })
}

/// The 1440 tier's bounding box, which every measurement below encodes into.
const TIER_1440: (u32, u32) = (2560, 1440);

fn encode(
    ready: &night_amplifier::server::state::RenderReadyFrame,
    max_w: u32,
    max_h: u32,
) -> (Vec<u8>, usize, usize) {
    let (bytes, w, h) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(ready, max_w, max_h).unwrap();
    (bytes, w as usize, h as usize)
}

/// The headline number for Tier 0, reported rather than only bounded so a
/// regression is legible instead of just red.
///
/// Encoding into the viewport the eyepiece actually displays is an area average;
/// leaving the browser to minify a near-native frame is a four-tap bilinear
/// filter that discards most of that averaging as aliasing.
///
/// Denoising is switched off here on purpose: this measures what the *resample*
/// is worth, and leaving the filters on made the printed figures a mixture of
/// the two — which is how the numbers quoted in `AGENTS.md` came to describe a
/// configuration the test no longer ran.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn lossless_stream_at_display_resolution_is_measurably_quieter() {
    println!("\n=== Lossless stream resolution (denoising off) ===");
    let mut measured = 0;

    for fixture in &FIXTURES {
        let Some(mut ready) = prepare_fixture(fixture, 0.0) else {
            println!("  {} not present. Skipping.", fixture.dir);
            continue;
        };
        ready.pipeline_config.denoise = night_amplifier::render::DenoiseConfig::OFF;
        measured += 1;

        // What the stream used to send: capped at 4K, leaving the browser to
        // shrink the result down to the 1440 screen.
        let (native, nw, nh) = encode(&ready, 3840, 2160);
        // What a 1440p eyepiece now asks for and receives.
        let (tiered, tw, th) = encode(&ready, TIER_1440.0, TIER_1440.1);

        let native_sigma = sky_sigma_levels(&native, 1);
        let tiered_sigma = sky_sigma_levels(&tiered, 1);

        println!("  {}", fixture.label);
        println!("    4K cap (was):    {nw}x{nh}, sky sigma {native_sigma:.2} output levels");
        println!("    1440 tier (now): {tw}x{th}, sky sigma {tiered_sigma:.2} output levels");
        println!(
            "    grain reduction: {:.2}x, payload {:.2}x smaller",
            native_sigma / tiered_sigma,
            native.len() as f64 / tiered.len() as f64
        );

        assert!(th <= 1440 && tw <= 2560, "{} exceeded its tier", fixture.label);
        assert!(
            tiered_sigma <= native_sigma,
            "{}: encoding at display resolution should not raise sky sigma: \
             {tiered_sigma:.2} vs {native_sigma:.2}",
            fixture.label
        );
    }

    assert!(measured > 0, "no fixture was available to measure");
}

/// The dark blocks: with a black floor set, no pixel of a real stretched frame
/// may reach an OLED as a fully-off pixel.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn black_floor_removes_every_off_pixel_from_a_real_frame() {
    println!("\n=== Black floor ===");
    let mut measured = 0;

    for fixture in &FIXTURES {
        let Some(mut ready) = prepare_fixture(fixture, 1.0) else {
            println!("  {} not present. Skipping.", fixture.dir);
            continue;
        };
        measured += 1;

        // Denoising off: this test is about the black floor, and the chroma
        // filter redistributes the clipped samples enough to hide the defect
        // being measured.
        ready.pipeline_config.denoise = night_amplifier::render::DenoiseConfig::OFF;

        // Without a floor, a meaningful share of the sky clamps to zero.
        ready.pipeline_config.display = night_amplifier::render::DisplayOutput::PLAIN;
        let (plain, _, _) = encode(&ready, TIER_1440.0, TIER_1440.1);
        let zeros = plain.iter().filter(|&&b| b == 0).count();
        let zero_fraction = zeros as f64 / plain.len() as f64;

        ready.pipeline_config.display = night_amplifier::render::DisplayOutput::default()
            .with_pedestal(0.04)
            .with_dither(true);
        let (lifted, _, _) = encode(&ready, TIER_1440.0, TIER_1440.1);

        println!("  {}", fixture.label);
        println!(
            "    without floor: {:.2}% of samples at exactly 0",
            zero_fraction * 100.0
        );
        println!(
            "    with floor:    {} samples at 0",
            lifted.iter().filter(|&&b| b == 0).count()
        );

        assert!(
            lifted.iter().all(|&b| b > 0),
            "{}: black floor left samples on zero; an OLED switches those pixels off",
            fixture.label
        );
    }

    assert!(measured > 0, "no fixture was available to measure");
}

/// Raising the eyepiece intensity must smooth the sky, which is what the slider
/// claims to do. Measured end to end on real data, in the units the eye sees.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn eyepiece_intensity_reduces_visible_sky_grain() {
    println!("\n=== Eyepiece intensity vs visible grain ===");
    let mut measured = 0;

    for fixture in &FIXTURES {
        let (Some(low), Some(high)) = (
            prepare_fixture(fixture, 0.0),
            prepare_fixture(fixture, 1.0),
        ) else {
            println!("  {} not present. Skipping.", fixture.dir);
            continue;
        };
        measured += 1;

        let sigma = |ready: &night_amplifier::server::state::RenderReadyFrame| {
            let (bytes, _, _) = encode(ready, TIER_1440.0, TIER_1440.1);
            sky_sigma_levels(&bytes, 1)
        };
        let sigma_low = sigma(&low);
        let sigma_high = sigma(&high);

        println!(
            "  {}: intensity 0.0 -> {sigma_low:.2}, intensity 1.0 -> {sigma_high:.2} output levels",
            fixture.label
        );

        assert!(
            sigma_high < sigma_low,
            "{}: raising eyepiece intensity must reduce visible sky grain, got \
             {sigma_high:.2} from {sigma_low:.2} — the black point factor is moving \
             the wrong way again",
            fixture.label
        );
    }

    assert!(measured > 0, "no fixture was available to measure");
}

// ---------------------------------------------------------------------------
// Tier 2: the denoisers, measured on the fixture
// ---------------------------------------------------------------------------

/// The two things a denoiser must be judged on together. Grain reduction alone
/// is not a passing result — anything can smooth a sky.
struct DenoiseMeasurement {
    sky_sigma: f64,
    nebula_flux: f64,
    star_peak: u8,
}

fn measure(rgb8: &[u8], width: usize, target_box: (usize, usize, usize, usize)) -> DenoiseMeasurement {
    let (x0, y0, x1, y1) = target_box;
    let mut flux = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            flux += rgb8[(y * width + x) * 3 + 1] as f64;
        }
    }

    DenoiseMeasurement {
        sky_sigma: sky_sigma_levels(rgb8, 1),
        nebula_flux: flux,
        star_peak: *rgb8.iter().skip(1).step_by(3).max().unwrap(),
    }
}

fn encode_with(
    fixture: &Fixture,
    denoise: night_amplifier::render::DenoiseConfig,
) -> Option<(Vec<u8>, usize)> {
    let mut ready = prepare_fixture(fixture, 0.0)?;
    ready.pipeline_config.denoise = denoise;
    let (bytes, w, _) = encode(&ready, TIER_1440.0, TIER_1440.1);
    Some((bytes, w))
}

/// The headline for Tier 2, reported rather than only bounded.
///
/// Runs the shipped default, the chroma filter alone, and the two ends of the
/// star-protection control — so the trade the level-1 threshold makes is visible
/// as numbers on real data rather than as an argument.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn denoisers_reduce_grain_without_eating_the_nebula() {
    use night_amplifier::render::{ChromaDenoiseConfig, DenoiseConfig, LumaDenoiseConfig};

    let cases: [(&str, DenoiseConfig); 3] = [
        (
            "chroma only",
            DenoiseConfig {
                luma: LumaDenoiseConfig::OFF,
                chroma: ChromaDenoiseConfig::default(),
            },
        ),
        (
            "default (star protection 100 %)",
            DenoiseConfig {
                luma: LumaDenoiseConfig {
                    k: LumaDenoiseConfig::thresholds_for_star_protection(1.0),
                    ..Default::default()
                },
                chroma: ChromaDenoiseConfig::default(),
            },
        ),
        (
            "star protection 0 %",
            DenoiseConfig {
                luma: LumaDenoiseConfig {
                    k: LumaDenoiseConfig::thresholds_for_star_protection(0.0),
                    ..Default::default()
                },
                chroma: ChromaDenoiseConfig::default(),
            },
        ),
    ];

    println!("\n=== Tier 2 denoisers, 1440 tier ===");
    let mut measured = 0;

    for fixture in &FIXTURES {
        let Some((plain, width)) = encode_with(fixture, DenoiseConfig::OFF) else {
            println!("  {} not present. Skipping.", fixture.dir);
            continue;
        };
        measured += 1;
        let base = measure(&plain, width, fixture.target_box);

        println!("  {}", fixture.label);
        println!(
            "    {:<32} sky sigma {:.2}, target flux {:.3e}, peak {}",
            "off", base.sky_sigma, base.nebula_flux, base.star_peak
        );

        for (name, config) in cases {
            let (bytes, w) = encode_with(fixture, config).unwrap();
            let m = measure(&bytes, w, fixture.target_box);
            println!(
                "    {name:<32} sky sigma {:.2} ({:.2}x), target flux {:.3e} ({:+.2} %), peak {}",
                m.sky_sigma,
                base.sky_sigma / m.sky_sigma,
                m.nebula_flux,
                (m.nebula_flux / base.nebula_flux - 1.0) * 100.0,
                m.star_peak
            );

            assert!(
                m.sky_sigma <= base.sky_sigma + 0.01,
                "{}/{name} made the sky noisier: {:.2} from {:.2}",
                fixture.label,
                m.sky_sigma,
                base.sky_sigma
            );
            assert!(
                (m.nebula_flux / base.nebula_flux - 1.0).abs() < 0.05,
                "{}/{name} moved integrated target flux by {:.1} % — the filter is \
                 eating signal",
                fixture.label,
                (m.nebula_flux / base.nebula_flux - 1.0) * 100.0
            );
            assert!(
                m.star_peak >= base.star_peak.saturating_sub(2),
                "{}/{name} clipped the brightest star from {} to {}",
                fixture.label,
                base.star_peak,
                m.star_peak
            );
        }
    }

    assert!(measured > 0, "no fixture was available to measure");
}

/// Every spelling of "off" must reach the fused traversal, not a staged one that
/// happens to agree. Byte equality against `DenoiseConfig::OFF` is what makes
/// adding a stage to a path every client crosses a safe change — and it is
/// asserted on the real tone curve rather than on a synthetic frame.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn every_disabled_denoise_config_reproduces_the_stream_byte_for_byte() {
    use night_amplifier::render::{ChromaDenoiseConfig, DenoiseConfig, LumaDenoiseConfig};

    let Some(mut ready) = prepare_fixture(&FIXTURES[0], 1.0) else {
        println!("Fixture {} not present. Skipping.", FIXTURES[0].dir);
        return;
    };

    ready.pipeline_config.denoise = DenoiseConfig::OFF;
    let (baseline, _, _) = encode(&ready, TIER_1440.0, TIER_1440.1);

    // Flags cleared but parameters left set, and parameters set but strength at
    // zero: both are "off" and both must take the same path as `OFF` itself.
    let variants = [
        DenoiseConfig {
            luma: LumaDenoiseConfig {
                enabled: false,
                ..Default::default()
            },
            chroma: ChromaDenoiseConfig {
                enabled: false,
                ..Default::default()
            },
        },
        DenoiseConfig {
            luma: LumaDenoiseConfig {
                strength: 0.0,
                ..Default::default()
            },
            chroma: ChromaDenoiseConfig {
                strength: 0.0,
                ..Default::default()
            },
        },
    ];

    for (i, denoise) in variants.into_iter().enumerate() {
        ready.pipeline_config.denoise = denoise;
        let (bytes, _, _) = encode(&ready, TIER_1440.0, TIER_1440.1);
        assert_eq!(baseline, bytes, "variant {i} did not reproduce the stream");
    }
}

// ---------------------------------------------------------------------------
// Tier 3: the darkening half of the black floor
// ---------------------------------------------------------------------------

/// Everything that has to be judged together when darkening the sky.
///
/// Sky level alone is not a result: any monotone curve drives it to zero, and
/// the two ways of getting there fail in opposite, equally visible ways.
/// `excess` is the one that decides between them — a curve that darkens the sky
/// and the target by the same factor has moved nothing the eye can use.
struct SkyLevels {
    /// Median green level over the whole encode. The sky is most of every frame
    /// here, so the frame median is the sky level.
    sky: f64,
    /// Clipped standard deviation of the sky, in output levels.
    sigma: f64,
    /// `target - sky`: how far the object stands above the background, which is
    /// what is actually visible. Sky level is a pedestal the eye adapts to.
    excess: f64,
    star_peak: u8,
    /// Share of samples on exactly 0 — the panel's off state, and black speckle
    /// at eyepiece magnification.
    zeros: f64,
    /// Mean `|R-G| + |B-G|` over sky pixels, in output levels. Only meaningful
    /// on a colour fixture; see `Fixture::is_colour`.
    sky_chroma: f64,
}

fn measure_levels(rgb8: &[u8], width: usize, target_box: (usize, usize, usize, usize)) -> SkyLevels {
    let mut green: Vec<u8> = rgb8.iter().skip(1).step_by(3).copied().collect();
    green.sort_unstable();
    let sky = green[green.len() / 2] as f64;

    let (x0, y0, x1, y1) = target_box;
    let mut flux = 0.0;
    let mut count = 0.0;
    let mut star_peak = 0u8;
    for y in y0..y1 {
        for x in x0..x1 {
            let v = rgb8[(y * width + x) * 3 + 1];
            flux += v as f64;
            count += 1.0;
            star_peak = star_peak.max(v);
        }
    }

    let cut = green[green.len() * 60 / 100];
    let mut chroma = 0.0;
    let mut chroma_n = 0.0f64;
    for px in rgb8.chunks_exact(3) {
        if px[1] > cut {
            continue;
        }
        chroma += (px[0] as f64 - px[1] as f64).abs() + (px[2] as f64 - px[1] as f64).abs();
        chroma_n += 1.0;
    }

    SkyLevels {
        sky,
        sigma: sky_sigma_levels(rgb8, 1),
        excess: flux / count - sky,
        star_peak,
        zeros: rgb8.iter().filter(|&&b| b == 0).count() as f64 / rgb8.len() as f64,
        sky_chroma: chroma / chroma_n.max(1.0),
    }
}

fn measure_setting(fixture: &Fixture, black_floor: f32, darker_sky: bool) -> Option<SkyLevels> {
    let ready = prepare_fixture_with(fixture, |settings| {
        // The other slider held at zero: it darkens the sky too, by dimming the
        // whole tone curve, and leaving it at its default would mix the two
        // effects in every figure below.
        settings.eyepiece.intensity = 0.0;
        settings.eyepiece.black_floor = black_floor;
        settings.eyepiece.darker_sky = darker_sky;
    })?;
    let (bytes, w, _) = encode(&ready, TIER_1440.0, TIER_1440.1);
    Some(measure_levels(&bytes, w, fixture.target_box))
}

/// The darkening half of the black floor, measured end to end on real frames.
///
/// Headline figures are printed as well as bounded, because "the sky got 70 %
/// darker and the target kept its contrast" is the whole claim and a red test
/// that does not say which half moved is not much use at three in the morning.
///
/// The bounds are deliberately loose against the measured values — the point is
/// to catch the curve being wired up wrongly or drifting out from under the
/// settings, not to pin numbers that legitimately move when the stretch is
/// retuned.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn the_black_floor_darkens_the_sky_without_dimming_the_target() {
    /// Mid-travel on the darkening half: the floor lands at the sky's own level.
    const SOFT: f32 = -0.05;
    /// The slider's end stop, which has to stay usable rather than merely legal.
    const DEEPEST: f32 = -0.09;

    println!("\n=== Black floor, darkening half ===");
    let mut measured = 0;

    for fixture in &FIXTURES {
        let (Some(base), Some(soft), Some(deep), Some(hard)) = (
            measure_setting(fixture, 0.0, false),
            measure_setting(fixture, SOFT, false),
            measure_setting(fixture, DEEPEST, false),
            measure_setting(fixture, SOFT, true),
        ) else {
            println!("  {} not present. Skipping.", fixture.dir);
            continue;
        };
        measured += 1;

        println!("  {}", fixture.label);
        for (label, m) in [
            ("baseline (floor 0%)", &base),
            ("soft floor -5%", &soft),
            ("soft floor -9% (slider end)", &deep),
            ("hard floor -5% (Darker sky)", &hard),
        ] {
            println!(
                "    {label:<31} sky {:>4.0} ({:>5.1}% darker)  sigma {:>4.1}  \
                 excess {:>4.1} ({:>+6.1}%)  star {:>3}  zeros {:>5.2}%",
                m.sky,
                (1.0 - m.sky / base.sky) * 100.0,
                m.sigma,
                m.excess,
                (m.excess / base.excess - 1.0) * 100.0,
                m.star_peak,
                m.zeros * 100.0,
            );
        }

        // The complaint this exists for: the sky is a clearly visible grey.
        assert!(
            base.sky >= 10.0,
            "{}: baseline sky is only {:.0} levels — if the stretch now leaves it \
             this dark, the darkening half of the slider may no longer be needed",
            fixture.label,
            base.sky
        );

        // 1. The soft floor darkens the sky substantially. Measured 71 % and
        //    65 %; the bound sits below both so a retune does not trip it.
        assert!(
            soft.sky <= base.sky * 0.4,
            "{}: soft floor left the sky at {:.0} levels, from {:.0}",
            fixture.label,
            soft.sky,
            base.sky
        );

        // 2. Without taking the target with it. This is what separates the floor
        //    from the black level slider, which halves the sky and two thirds of
        //    the excess along with it.
        assert!(
            soft.excess >= base.excess * 0.85,
            "{}: soft floor cost {:.0}% of the target's contrast ({:.1} from {:.1}) \
             — it is dimming the image rather than lowering the background",
            fixture.label,
            (1.0 - soft.excess / base.excess) * 100.0,
            soft.excess,
            base.excess
        );
        assert!(
            soft.star_peak >= base.star_peak - 2,
            "{}: soft floor pulled star peaks down to {} from {}; white is \
             supposed to stay white",
            fixture.label,
            soft.star_peak,
            base.star_peak
        );

        // 3. And without putting a single sample on the panel's off state. The
        //    curve approaches zero without arriving and the pedestal underneath
        //    it is what makes that survive rounding.
        assert_eq!(
            soft.zeros, 0.0,
            "{}: soft floor put {:.2}% of samples on exactly 0 — that is the \
             black speckle the whole floor exists to avoid",
            fixture.label,
            soft.zeros * 100.0
        );

        // 4. The slider's end stop has to stay usable. It costs contrast — that
        //    is what the last of the travel is for — but an end position that
        //    flattens the target is a bug in the range, not a choice.
        assert!(
            deep.sky < soft.sky,
            "{}: the end of the slider is no darker than its middle",
            fixture.label
        );
        assert!(
            deep.excess >= base.excess * 0.5,
            "{}: the slider's end stop cost {:.0}% of the target's contrast; the \
             negative range reaches further than it should",
            fixture.label,
            (1.0 - deep.excess / base.excess) * 100.0
        );
        assert_eq!(deep.zeros, 0.0, "{}: end stop clipped", fixture.label);

        // 5. "Darker sky" is the opposite trade, and has to actually make it.
        assert!(
            hard.sky <= soft.sky,
            "{}: the hard floor is not darker than the soft one ({:.0} against {:.0})",
            fixture.label,
            hard.sky,
            soft.sky
        );
        assert!(
            hard.zeros > 0.01,
            "{}: the hard floor reached true black for only {:.2}% of samples; \
             if it no longer clips it is not the hard floor",
            fixture.label,
            hard.zeros * 100.0
        );
        assert!(
            hard.excess >= base.excess,
            "{}: the hard floor is supposed to buy separation, not spend it \
             ({:.1} from {:.1})",
            fixture.label,
            hard.excess,
            base.excess
        );

        // 6. Colour, on the one fixture that has any. A floor applied per
        //    channel takes a near-neutral shadow apart into its components; this
        //    one runs on luminance, so grey sky stays grey as it darkens.
        if fixture.is_colour {
            assert!(
                base.sky_chroma > 1.0,
                "{}: flagged as colour but its sky chroma is {:.2} — the \
                 assertion below cannot detect anything",
                fixture.label,
                base.sky_chroma
            );
            let ratio = soft.sky_chroma / soft.sky;
            let base_ratio = base.sky_chroma / base.sky;
            println!(
                "    chroma per sky level: {base_ratio:.3} -> {ratio:.3}",
            );
            assert!(
                ratio <= base_ratio * 1.5,
                "{}: the floor colourised the sky — {ratio:.3} of chroma per sky \
                 level against {base_ratio:.3}. A per-channel subtraction does \
                 exactly this.",
                fixture.label
            );
        }
    }

    assert!(measured > 0, "no fixture was available to measure");
}

/// A zero floor has to leave the stream exactly as it was, whatever the toggle
/// says — so that turning the feature on is the only thing that can change what
/// an existing observer sees.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_zero_black_floor_reproduces_the_plain_stream_byte_for_byte() {
    let fixture = &FIXTURES[0];
    let Some(mut plain) = prepare_fixture_with(fixture, |settings| {
        settings.eyepiece.black_floor = 0.0;
        settings.eyepiece.dither = false;
    }) else {
        println!("Fixture {} not present. Skipping.", fixture.dir);
        return;
    };

    assert!(
        plain.pipeline_config.display.is_plain(),
        "a zero floor must leave the 8-bit conversion plain"
    );
    plain.pipeline_config.display = night_amplifier::render::DisplayOutput::PLAIN;
    let (reference, _, _) = encode(&plain, TIER_1440.0, TIER_1440.1);

    for darker_sky in [false, true] {
        let Some(ready) = prepare_fixture_with(fixture, |settings| {
            settings.eyepiece.black_floor = 0.0;
            settings.eyepiece.dither = false;
            settings.eyepiece.darker_sky = darker_sky;
        }) else {
            return;
        };
        let (bytes, _, _) = encode(&ready, TIER_1440.0, TIER_1440.1);
        assert_eq!(
            reference, bytes,
            "darker_sky = {darker_sky} changed the stream at a zero floor"
        );
    }
}

/// The darkening half is anchored to a sky level the solver measures, so with
/// auto-stretch off there is nothing to anchor to — and no `StretchResult` for
/// the curve to travel on either.
///
/// It used to let the request through anyway, which left only the guard pedestal
/// underneath it: the sky went *up*, from 2 output levels to 3, on a control
/// labelled "darker". Byte equality rather than a level comparison, because the
/// claim is that the setting is inert, not that it moves the sky a little less.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_negative_floor_without_auto_stretch_leaves_the_stream_alone() {
    let fixture = &FIXTURES[0];
    let build = |black_floor: f32| {
        prepare_fixture_with(fixture, |settings| {
            settings.auto_stretch = false;
            settings.eyepiece.intensity = 0.0;
            settings.eyepiece.dither = false;
            settings.eyepiece.black_floor = black_floor;
        })
    };

    let (Some(zero), Some(deepest)) = (build(0.0), build(-0.09)) else {
        println!("Fixture {} not present. Skipping.", fixture.dir);
        return;
    };

    assert!(deepest.pipeline_config.shadow_floor.is_none());
    assert!(deepest.pipeline_config.display.is_plain());

    let (reference, width, _) = encode(&zero, TIER_1440.0, TIER_1440.1);
    let (bytes, _, _) = encode(&deepest, TIER_1440.0, TIER_1440.1);
    let before = measure_levels(&reference, width, fixture.target_box);
    let after = measure_levels(&bytes, width, fixture.target_box);
    assert_eq!(
        reference, bytes,
        "the slider moved the stream without a solve behind it: sky {:.0} -> {:.0}",
        before.sky, after.sky
    );
}

/// The floor rides the scale LUT when contrast does, and follows contrast out
/// into the encoder's row tail when saturation boost pushes it out. Two code
/// paths, one setting — so they have to agree, or the slider means one thing in
/// Community and another in Pro.
///
/// Reachable from here because the split is decided by the saturation *flag*,
/// not by the plugin: with the flag set and no Pro plugin registered, the boost
/// itself is a no-op while the floor still takes the deferred path.
///
/// # Why this is measured against a control rather than against zero
///
/// Fusing contrast into the scale LUT already disagrees with running it as its
/// own pass, on highlights where a channel clips: one path clamps once at the
/// end, the other clamps between the two stages, and a star whose red channel
/// saturates comes out with a different green. `probe_fused_render_clamp_
/// difference` calls that accepted divergence, and it is worth 34 output levels
/// on the brightest handful of pixels here *with no floor configured at all*.
/// So the question this test can answer is not "do the two paths agree" — they
/// already did not — but "does adding the floor make them agree any less".
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn the_deferred_floor_adds_no_disagreement_to_the_fused_one() {
    let fixture = &FIXTURES[0];

    let compare = |black_floor: f32| -> Option<(i32, f64)> {
        let prepare = |saturation: bool| {
            prepare_fixture_with(fixture, |settings| {
                settings.eyepiece.intensity = 0.0;
                settings.eyepiece.black_floor = black_floor;
                settings.saturation_boost = saturation;
            })
        };
        let (fused, deferred) = (prepare(false)?, prepare(true)?);

        // The split really did happen, or this measures nothing.
        let floor_of = |r: &night_amplifier::server::state::RenderReadyFrame| {
            r.stretch_result
                .as_ref()
                .unwrap()
                .deferred_shadow_floor
                .is_none()
        };
        assert!(floor_of(&fused), "floor should be in the scale LUT");
        assert_eq!(
            floor_of(&deferred),
            black_floor >= 0.0,
            "the row tail did not get the floor it was supposed to"
        );

        let (a, _, _) = encode(&fused, TIER_1440.0, TIER_1440.1);
        let (b, _, _) = encode(&deferred, TIER_1440.0, TIER_1440.1);
        let mut worst = 0i32;
        let mut total = 0i64;
        for (x, y) in a.iter().zip(b.iter()) {
            let d = (*x as i32 - *y as i32).abs();
            worst = worst.max(d);
            total += d as i64;
        }
        Some((worst, total as f64 / a.len() as f64))
    };

    let (Some(control), Some(floored)) = (compare(0.0), compare(-0.05)) else {
        println!("Fixture {} not present. Skipping.", fixture.dir);
        return;
    };

    println!("\n=== Fused against deferred shadow floor ===");
    println!(
        "  no floor:   worst {} levels, mean {:.4}",
        control.0, control.1
    );
    println!(
        "  floor -5%:  worst {} levels, mean {:.4}",
        floored.0, floored.1
    );

    assert!(
        floored.0 <= control.0,
        "the floor made the two paths disagree by {} output levels where they \
         already disagreed by {} — it is being applied in a different place, or \
         a different order, on one of them",
        floored.0,
        control.0
    );
    assert!(
        // Measured 0.0028 without the floor and 0.0099 with it: a hundredth of
        // an output level, which is the two tables' interpolation and nothing
        // else. The bound leaves room for a stretch retune to move both.
        floored.1 <= control.1 + 0.02,
        "the floor widened the average disagreement from {:.4} to {:.4}",
        control.1,
        floored.1
    );
}
