//! Planetary renders on real discs: `Moon_Mosaic`'s eleven full-Moon panels, `Planets`' Jupiters
//! and Uranus, and the drifting Moon's SER, rendered through the production preview path and
//! measured on the target the linear frame shows. The measures are those that judged the tone
//! (2026-10-29): how much of the target clips or crushes, where its levels land, and whether
//! anything but one curve of each pixel's own brightness ran.

use std::path::{Path, PathBuf};

use night_amplifier::frame::{Frame, LUMA_REC709};
use night_amplifier::render::planetary::PlanetaryLevels;
use night_amplifier::session::capture::pipeline::{process_preview_frame_with_analysis, PreviewRender};
use night_amplifier::session::capture::{AnalysisContext, PreviewAnalysis};
use night_amplifier::session::state::CaptureSettings;
use night_amplifier::stacking::StackingType;

use super::common::{ensure_fixtures_sync_named, missing_fixture_message, FIXTURES_DIR};

pub const MOSAIC: &str = "Moon_Mosaic";
pub const PLANETS: &str = "Planets";

/// `Moon_Mosaic`'s panels (SharpCap stacks, 2712x1538) by the time they were taken, and why.
pub const MOSAIC_PANELS: [(&str, &str); 11] = [
    ("19_35_32", "sky bottom right, 32 % of the frame"),
    ("19_37_31", "sky bottom left, 37 %"),
    ("19_39_41", "the limb along an edge, 12 % sky"),
    ("19_41_40", "no sky at all: its darkest population is maria"),
    ("19_43_24", "5 % sky in a corner"),
    ("19_45_33", "8 % sky"),
    ("19_47_45", "sky across the top, 46 %: the gradient model took its median to 4 of 255"),
    ("19_50_14", "9 % sky, the panel's brightest highlands"),
    ("19_52_15", "8 % sky"),
    ("20_15_43", "sky across the top, 35 %"),
    ("20_21_04", "the whole disc, 45 % sky"),
];

/// `Planets`' files and why each is in the set.
pub const PLANET_FILES: [(&str, &str); 4] = [
    ("jupiter.tif", "near saturation: its top at 0.97 of full scale"),
    ("jupiter_io_transit.tif", "Io and its shadow on the disc, a moon beside it"),
    ("jupiter_orange.tif", "a small disc, 0.16 % of the frame"),
    ("uranus.tif", "a 675 px dot on a twilight sky at 0.029"),
];

/// The fixture's directory, downloaded if missing; panics rather than skips without it.
pub fn fixture(name: &str) -> PathBuf {
    ensure_fixtures_sync_named(&[name]);
    let dir = Path::new(FIXTURES_DIR).join(name);
    assert!(dir.is_dir(), "{}", missing_fixture_message(name));
    dir
}

/// A mosaic panel's file, by the time [`MOSAIC_PANELS`] names it by.
pub fn mosaic_panel(dir: &Path, time: &str) -> PathBuf {
    dir.join(format!("Snapshot at {time} of Stack_00001.tif"))
}

/// Planetary at the shipped settings, background subtraction among them, which it must ignore.
pub fn planetary_settings(stacking: bool) -> CaptureSettings {
    CaptureSettings { stacking, stacking_type: StackingType::Planetary, ..Default::default() }
}

/// `frame` through the production preview and encoder at its own size: the RGB8 bytes, and the
/// levels the render solved.
pub fn render(frame: &Frame, settings: &CaptureSettings, stack_depth: u32) -> (Vec<u8>, PlanetaryLevels) {
    let mut linear = frame.clone();
    let PreviewRender { pipeline_config, stretch_result, levels, .. } = process_preview_frame_with_analysis(
        &mut linear,
        settings,
        &night_amplifier::plugins::Plugins::installed(),
        AnalysisContext { showing_stack: stack_depth > 1, stack_depth },
        &mut PreviewAnalysis::new(),
    )
    .unwrap();
    let ready = night_amplifier::render::display::RenderReadyFrame {
        noise: None,
        linear_frame: std::sync::Arc::new(linear),
        pipeline_config,
        stretch_result,
    };
    let (bytes, w, h) = night_amplifier::render::display::frame_to_rgb8_downsampled(&ready, 3840, 3840).unwrap();
    assert_eq!((w as usize, h as usize), (frame.width(), frame.height()), "rendered at its own size");
    (bytes, levels.expect("a Planetary render reports its levels"))
}

/// The target's render, in output levels.
#[derive(Debug)]
pub struct TargetRender {
    /// Share of the frame that is target.
    pub target_share: f64,
    /// Percent of the target with a channel at 254 or more.
    pub clipped: f64,
    /// Percent of the target at a luminance of 3 or less.
    pub crushed: f64,
    pub p1: f64,
    pub p50: f64,
    pub p99: f64,
    /// The median luminance well below the target, where there is a sky.
    pub sky: Option<f64>,
    /// RMS, over the target, of the output's brightest channel about the median of those of
    /// the same input brightest channel: nothing but a per-pixel curve leaves it near 0.
    pub residual: f64,
}

/// The brightest channel of each pixel of `frame`.
fn brightest(frame: &Frame) -> Vec<f32> {
    let area = frame.width() * frame.height();
    (0..area)
        .map(|i| (0..frame.channels()).map(|c| frame.channel_data(c)[i]).fold(0.0f32, f32::max))
        .collect()
}

/// The 16th brightest of each pixel's dimmer with its right-hand neighbour: a disc's top, never
/// an isolated hot pixel's, however small the disc is against its frame.
fn peak(v: &[f32], width: usize) -> f32 {
    let mut pairs: Vec<f32> = (0..v.len() - 1).filter(|i| (i + 1) % width != 0).map(|i| v[i].min(v[i + 1])).collect();
    pairs.sort_by(f32::total_cmp);
    pairs[pairs.len() - 16]
}

/// Measures `rgb8` against the target `linear` shows: pixels brighter than 12 % of its [`peak`].
pub fn measure(linear: &Frame, rgb8: &[u8]) -> TargetRender {
    let v = brightest(linear);
    let mut sorted = v.clone();
    sorted.sort_by(f32::total_cmp);
    let peak = peak(&v, linear.width());
    let pixel = |i: usize| [rgb8[i * 3], rgb8[i * 3 + 1], rgb8[i * 3 + 2]];
    let luma = |i: usize| {
        let [r, g, b] = pixel(i);
        f64::from(LUMA_REC709[0]) * f64::from(r) + f64::from(LUMA_REC709[1]) * f64::from(g) + f64::from(LUMA_REC709[2]) * f64::from(b)
    };
    let target: Vec<usize> = (0..v.len()).filter(|&i| v[i] > 0.12 * peak).collect();
    let mut levels: Vec<f64> = target.iter().map(|&i| luma(i)).collect();
    levels.sort_by(f64::total_cmp);
    let at = |q: f64| levels[((levels.len() - 1) as f64 * q) as usize];
    let share = |hit: &dyn Fn(usize) -> bool| 100.0 * target.iter().filter(|&&i| hit(i)).count() as f64 / target.len() as f64;

    let mut sky: Vec<f64> = (0..v.len()).filter(|&i| v[i] < 0.06 * peak).map(luma).collect();
    sky.sort_by(f64::total_cmp);

    // Output brightest channel, binned by input brightest channel: 1024 bins up to the
    // brightest pixel, not the peak, which a small disc's top 6 % lie above.
    let top = sorted[sorted.len() - 1];
    let bin = |i: usize| ((v[i] / top * 1023.0) as usize).min(1023);
    let mut by_bin: Vec<Vec<u8>> = vec![Vec::new(); 1024];
    for &i in &target {
        by_bin[bin(i)].push(pixel(i).into_iter().max().unwrap());
    }
    let medians: Vec<f64> = by_bin
        .iter_mut()
        .map(|outs| {
            outs.sort_unstable();
            outs.get(outs.len() / 2).map_or(0.0, |&m| f64::from(m))
        })
        .collect();
    let residual = (target
        .iter()
        .map(|&i| (f64::from(pixel(i).into_iter().max().unwrap()) - medians[bin(i)]).powi(2))
        .sum::<f64>()
        / target.len() as f64)
        .sqrt();

    TargetRender {
        target_share: target.len() as f64 / v.len() as f64,
        clipped: share(&|i| pixel(i).into_iter().max().unwrap() >= 254),
        crushed: share(&|i| luma(i) <= 3.0),
        p1: at(0.01),
        p50: at(0.50),
        p99: at(0.99),
        sky: (sky.len() * 100 >= v.len()).then(|| sky[sky.len() / 2]),
        residual,
    }
}

/// Mean of each channel over the target, as ratios to green.
pub fn target_colour(frame_or_rgb: impl Fn(usize, usize) -> f64, target: &[usize]) -> [f64; 2] {
    let mean = |c: usize| target.iter().map(|&i| frame_or_rgb(i, c)).sum::<f64>() / target.len() as f64;
    [mean(0) / mean(1), mean(2) / mean(1)]
}

/// The target pixels of `linear` that no channel of `rgb8` clipped.
pub fn unclipped_target(linear: &Frame, rgb8: &[u8]) -> Vec<usize> {
    let v = brightest(linear);
    let peak = peak(&v, linear.width());
    (0..v.len())
        .filter(|&i| v[i] > 0.3 * peak && (0..3).all(|c| rgb8[i * 3 + c] < 250))
        .collect()
}
