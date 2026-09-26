//! The AI denoiser's stage in the encoder, through a stand-in plugin: what the network is
//! handed, that its output is what the rest of the tail renders, and every gate that keeps
//! it from being called at all. The network itself is Pro's; its behaviour is tested there.
//!
//! Its own binary: the plugin registry is process-wide. Calls are recorded per thread,
//! and the encoder calls the plugin on the thread that asked for the conversion.

use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use night_amplifier::render::{AiDenoiseConfig, AiDenoisePlugin, DenoiseScratch, AI_DENOISE_PLUGIN};
use night_amplifier::server::capture::pipeline::process_preview_frame_with_analysis;
use night_amplifier::server::capture::{AnalysisContext, PreviewAnalysis};
use night_amplifier::server::encoding::frame_to_rgb8_downsampled;
use night_amplifier::server::state::{CaptureSettings, DenoiseSettings, RenderReadyFrame};
use night_amplifier::Frame;

thread_local! {
    /// `(width, height, median green)` of every image the network was handed.
    static CALLS: RefCell<Vec<(usize, usize, f32)>> = const { RefCell::new(Vec::new()) };
    /// When set, the stand-in replaces the image with this grey.
    static FILL: Cell<Option<f32>> = const { Cell::new(None) };
}

struct StandIn;

impl AiDenoisePlugin for StandIn {
    fn config(&self, _settings: &DenoiseSettings) -> AiDenoiseConfig {
        AiDenoiseConfig {
            enabled: true,
            strength: 1.0,
            highlight_floor: 0.2,
            compute: Default::default(),
        }
    }

    fn denoise_display_rgb(
        &self,
        buf: &mut [f32],
        width: usize,
        height: usize,
        _config: &AiDenoiseConfig,
        _scratch: &mut DenoiseScratch,
    ) {
        let mut green: Vec<f32> = buf.iter().skip(1).step_by(3).copied().collect();
        green.sort_by(f32::total_cmp);
        CALLS.with(|calls| calls.borrow_mut().push((width, height, green[green.len() / 2])));
        if let Some(grey) = FILL.get() {
            buf.fill(grey);
        }
    }
}

fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        AI_DENOISE_PLUGIN.set(Box::new(StandIn)).ok();
        night_amplifier::license::PRO_LICENSE_ACTIVE.store(true, Ordering::SeqCst);
    });
    CALLS.with(|calls| calls.borrow_mut().clear());
}

fn calls() -> Vec<(usize, usize, f32)> {
    CALLS.with(|calls| std::mem::take(&mut *calls.borrow_mut()))
}

/// A stack's worth of sky: a faint tinted background with a little noise and a few stars.
fn sky() -> Frame {
    let (w, h) = (256, 256);
    let mut frame = Frame::filled(w, h, 3, 0.0).unwrap();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let stars = [(40.0, 50.0), (200.0, 90.0), (120.0, 200.0), (70.0, 160.0)];
    for y in 0..h {
        for x in 0..w {
            let star: f32 = stars
                .iter()
                .map(|&(sx, sy): &(f32, f32)| 0.2 * (-((x as f32 - sx).powi(2) + (y as f32 - sy).powi(2)) / 4.5).exp())
                .sum();
            for c in 0..3 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let noise = ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.0008;
                frame.set_pixel(x, y, c, 0.004 + 0.0005 * c as f32 + noise + star);
            }
        }
    }
    frame
}

/// The network switched on, with a plain display so every output byte is the tail's.
fn asking_for_the_network() -> CaptureSettings {
    let mut settings = CaptureSettings {
        denoise: DenoiseSettings {
            ai: true,
            ..Default::default()
        },
        ..Default::default()
    };
    settings.eyepiece.dither = false;
    settings.eyepiece.black_floor = 0.0;
    settings
}

fn render(settings: &CaptureSettings, showing_stack: bool) -> (RenderReadyFrame, Vec<u8>) {
    let mut frame = sky();
    let rendered = process_preview_frame_with_analysis(
        &mut frame,
        settings,
        AnalysisContext {
            showing_stack,
            stack_depth: if showing_stack { 16 } else { 1 },
        },
        &mut PreviewAnalysis::new(),
    )
    .unwrap();
    let ready = RenderReadyFrame {
        linear_frame: Arc::new(frame),
        pipeline_config: rendered.pipeline_config,
        stretch_result: rendered.stretch_result,
        noise: None,
    };
    let (bytes, _, _) = frame_to_rgb8_downsampled(&ready, 1440, 1440).unwrap();
    (ready, bytes)
}

/// The network is handed the stretched image — the sky where the stretch put it — and
/// not linear light, nor the S-curve's darker sky: that curve stays out of the fused LUT
/// so it can run afterwards.
#[test]
fn the_network_reads_the_stretched_image_before_the_s_curve() {
    register();
    let (ready, _) = render(&asking_for_the_network(), true);
    let calls = calls();
    assert_eq!(calls.len(), 1, "one conversion, one pass");
    let (width, height, sky) = calls[0];
    assert_eq!((width, height), (256, 256));

    let config = &ready.pipeline_config;
    assert!(config.contrast, "the S-curve was fused into the LUT, ahead of the network");
    let target = config.stretch_config.target_background;
    let after_curve = night_amplifier::render::sky_level_after_contrast(target, Some(&config.contrast_config));
    assert!(
        (sky - target).abs() < 0.25 * target && (sky - target).abs() < (sky - after_curve).abs(),
        "the network was handed a sky at {sky}: the stretch puts it at {target}, the S-curve at {after_curve}"
    );

    let (without, _) = render(&CaptureSettings { denoise: DenoiseSettings::default(), ..asking_for_the_network() }, true);
    assert!(!without.pipeline_config.contrast, "without the network the S-curve rides the LUT, as before");
}

/// What the network returns is what the rest of the tail renders: the S-curve runs on
/// it, and nothing re-applies the stretch.
#[test]
fn the_networks_output_is_what_the_rest_of_the_tail_renders() {
    register();
    FILL.set(Some(0.5));
    let (ready, bytes) = render(&asking_for_the_network(), true);
    FILL.set(None);

    let mut grey = [0.5f32; 3];
    night_amplifier::render::output::apply_contrast_slice(&mut grey, &ready.pipeline_config.contrast_config);
    let expected = (grey[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    assert_ne!(expected, 128, "the S-curve must move a mid grey, or this proves nothing");
    assert!(
        bytes.iter().all(|&b| b == expected),
        "the network's grey did not come out as the S-curve's {expected}"
    );
}

/// Live view gets the network too; Focus/Finder mode is the way to hold it off while
/// framing.
#[test]
fn live_view_runs_the_network() {
    register();
    render(&asking_for_the_network(), false);
    assert_eq!(calls().len(), 1);
}

#[test]
fn nothing_calls_the_network_through_a_shut_gate() {
    register();
    let mut focus = asking_for_the_network();
    night_amplifier::server::state::focus_mode::set(&mut focus, true);
    let mut planetary = asking_for_the_network();
    planetary.stacking_type = night_amplifier::stacking::StackingType::Planetary;
    let mut master_off = asking_for_the_network();
    master_off.denoise.enabled = false;
    let mut switch_off = asking_for_the_network();
    switch_off.denoise.ai = false;

    for (gate, settings) in [
        ("Focus/Finder mode", focus),
        ("Planetary", planetary),
        ("the Denoise switch", master_off),
        ("the AI switch", switch_off),
    ] {
        let (ready, _) = render(&settings, true);
        assert!(calls().is_empty(), "{gate} let the network run");
        assert!(!ready.pipeline_config.denoise.ai.is_enabled(), "{gate}");
    }
}
