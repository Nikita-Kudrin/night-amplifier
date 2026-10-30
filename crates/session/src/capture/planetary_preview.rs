//! The preview render of a Planetary frame: the camera's daylight balance, then levels read off
//! the target and a max-channel table (`render::planetary`). None of the deep-sky analysis runs:
//! no white balance grid, background model, statistics or noise window, so nothing is cached
//! either. A solve is one histogram pass over the frame.
//!
//! The frame leaves balanced but with its black point on, which travels in the `StretchResult`
//! for the kernel to take off: so the stream holds what the Moon measure reads (a clipped channel
//! still at 1; `capture::daylight_tracker`), and a full-resolution pass over three planes is saved.

use night_amplifier_core::frame::Frame;
use night_amplifier_core::render::display::StretchResult;
use night_amplifier_core::render::planetary::{self, PlanetaryLevels};
use night_amplifier_core::render::RenderPipelineConfig;

use super::pipeline::PreviewRender;

/// Renders `frame` in place under a Planetary `pipeline_config`, through the balance it carries.
/// The balance applies with the stretch off too: colour is the camera's, not the curve's.
pub(super) fn render(frame: &mut Frame, pipeline_config: RenderPipelineConfig) -> PreviewRender {
    {
        let _span = tracing::info_span!("daylight_balance").entered();
        pipeline_config.daylight_balance.apply(frame);
    }
    if !pipeline_config.auto_stretch {
        return PreviewRender { pipeline_config, stretch_result: None, solve: None, levels: None };
    }
    let levels = {
        let _span = tracing::info_span!("planetary_levels").entered();
        PlanetaryLevels::solve(frame)
    };
    let stretch_result = StretchResult {
        black_point: levels.black,
        scale_lut: planetary::scale_lut(levels.span()),
        color_intensity: 1.0,
        deferred_shadow_floor: None,
        sky_shadow: None,
    };
    PreviewRender {
        pipeline_config,
        stretch_result: Some(stretch_result),
        solve: None,
        levels: Some(levels),
    }
}

#[cfg(test)]
mod tests {
    use super::super::analysis::{AnalysisContext, PreviewAnalysis};
    use super::super::pipeline::{process_preview_frame_with_analysis, PreviewRender};
    use night_amplifier_core::render::display::StretchResult;
    use crate::state::CaptureSettings;
    use night_amplifier_core::frame::Frame;
    use night_amplifier_core::plugins::Plugins;
    use night_amplifier_core::render::planetary::DaylightBalance;
    use night_amplifier_core::render::stretch::ScaleBasis;
    use night_amplifier_core::stacking::StackingType;

    /// A disc of 0.3-0.7 on a 0.004 sky, with a left-to-right ramp a gradient model would take.
    fn disc(tint: [f32; 3]) -> Frame {
        let (w, h) = (160, 120);
        let mut frame = Frame::zeros(w, h, 3).unwrap();
        for y in 0..h {
            for x in 0..w {
                let r = (x as f32 - 80.0).hypot(y as f32 - 60.0) / 40.0;
                let level = if r < 1.0 { 0.3 + 0.4 * (1.0 - r * r).sqrt() } else { 0.0 };
                for (c, t) in tint.iter().enumerate() {
                    frame.set_pixel(x, y, c, (level + 0.004 + 0.002 * x as f32 / w as f32) * t);
                }
            }
        }
        frame
    }

    fn render(frame: &mut Frame, settings: &CaptureSettings) -> PreviewRender {
        process_preview_frame_with_analysis(
            frame,
            settings,
            &Plugins::none(),
            AnalysisContext { showing_stack: true, stack_depth: 30 },
            &mut PreviewAnalysis::new(),
        )
        .unwrap()
    }

    /// Background subtraction on, as it ships: a Planetary frame leaves the preview as its input,
    /// every pixel, with its black point beside it, so no spatial stage ran.
    #[test]
    fn a_planetary_frame_leaves_as_its_input_with_its_black_point_beside_it() {
        let settings = CaptureSettings {
            stacking_type: StackingType::Planetary,
            daylight_balance: DaylightBalance::IDENTITY,
            ..Default::default()
        };
        assert!(settings.background_subtraction);
        let input = disc([1.0; 3]);
        let mut frame = input.clone();
        let rendered = render(&mut frame, &settings);

        let levels = rendered.levels.expect("Planetary reports its levels");
        assert!(rendered.solve.is_none());
        let stretch = rendered.stretch_result.expect("stretched");
        assert_eq!((stretch.scale_lut.basis(), stretch.black_point), (ScaleBasis::MaxChannel, levels.black));
        assert!(levels.black > 0.004 && levels.black < 0.006, "{levels:?}");
        assert_eq!(frame.data(), input.data(), "the frame is untouched");
    }

    /// The black point taken off in the kernel, not in place beforehand: the same bytes.
    #[test]
    fn the_kernel_takes_the_black_point_off_as_the_frame_once_had_it_taken_off() {
        use night_amplifier_core::render::display::{frame_to_rgb8_downsampled, RenderReadyFrame};
        use std::sync::Arc;
        let settings = CaptureSettings { stacking_type: StackingType::Planetary, ..Default::default() };
        let mut frame = disc([0.94, 1.0, 0.48]);
        let PreviewRender { pipeline_config, stretch_result, .. } = render(&mut frame, &settings);
        let stretch = stretch_result.expect("stretched");
        let ready = |linear: Frame, black_point: f32| RenderReadyFrame {
            noise: None,
            linear_frame: Arc::new(linear),
            pipeline_config: pipeline_config.clone(),
            stretch_result: Some(StretchResult { black_point, ..stretch.clone() }),
        };
        let mut subtracted = frame.clone();
        night_amplifier_core::render::subtract_black_point_uniform(&mut subtracted, stretch.black_point).unwrap();
        let (w, h) = (frame.width() as u32, frame.height() as u32);
        let bytes = |ready: RenderReadyFrame| frame_to_rgb8_downsampled(&ready, w, h).unwrap().0;
        assert_eq!(bytes(ready(frame, stretch.black_point)), bytes(ready(subtracted, 0.0)));
    }

    /// Colour is the camera's, not the curve's: the balance lands with the stretch off too.
    #[test]
    fn the_daylight_balance_reaches_a_planetary_frame_with_the_stretch_off() {
        const RAW: [f32; 3] = [0.943, 1.0, 0.484];
        let settings = CaptureSettings {
            stacking_type: StackingType::Planetary,
            auto_stretch: false,
            daylight_balance: DaylightBalance { red: 1.0 / 0.943, green: 1.0, blue: 1.0 / 0.484 }.sanitized(),
            ..Default::default()
        };
        let mut frame = disc(RAW);
        let rendered = render(&mut frame, &settings);
        assert!(rendered.stretch_result.is_none() && rendered.levels.is_none());
        assert_eq!(rendered.pipeline_config.daylight_balance, settings.daylight_balance, "carried with the frame");
        let (r, g, b) = (frame.get_pixel(80, 60, 0), frame.get_pixel(80, 60, 1), frame.get_pixel(80, 60, 2));
        assert!((r / g - 1.0).abs() < 1e-3 && (b / g - 1.0).abs() < 1e-3, "{r} {g} {b}");
    }

    /// The balance is Planetary's: a deep-sky frame renders the same with or without one.
    #[test]
    fn a_deep_sky_frame_ignores_the_daylight_balance() {
        let plain = CaptureSettings::default();
        let balanced = CaptureSettings {
            daylight_balance: DaylightBalance { red: 1.0, green: 1.4, blue: 2.0 },
            ..plain.clone()
        };
        let (mut a, mut b) = (disc([1.0; 3]), disc([1.0; 3]));
        render(&mut a, &plain);
        render(&mut b, &balanced);
        assert_eq!(a.data(), b.data());
    }
}
