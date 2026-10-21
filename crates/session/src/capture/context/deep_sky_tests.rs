use super::*;
use crate::state::CaptureSettings;
use night_amplifier_core::plugins::Plugins;
use night_amplifier_core::stacking::{RejectionMethod, WeightingPreset};

fn context() -> StackingContext {
    let settings = StackSettings::of(&CaptureSettings::default(), &Plugins::none());
    StackingContext::new(64, 64, 1, &settings).expect("context builds")
}

/// A 64² field of five Gaussian stars of width `sigma`.
fn starfield(sigma: f32) -> Frame {
    let mut frame = Frame::filled(64, 64, 1, 0.05).unwrap();
    for (cx, cy) in [(16.0f32, 16.0f32), (48.0, 16.0), (32.0, 32.0), (16.0, 48.0), (48.0, 48.0)] {
        for y in 0..64 {
            for x in 0..64 {
                let r2 = (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2);
                let v = frame.get_pixel(x, y, 0) + 0.6 * (-r2 / (2.0 * sigma * sigma)).exp();
                frame.set_pixel(x, y, 0, v);
            }
        }
    }
    frame
}

/// The session's star width starts at the reference's and follows the subs, a field
/// too wide to measure reading as the widest measurable star rather than nothing.
#[test]
fn the_star_width_follows_the_subs_and_counts_wide_stars() {
    let mut ctx = context();
    assert_eq!(ctx.star_fwhm(), None, "nothing measured yet");

    ctx.initialize_with_reference(&starfield(2.0)).expect("reference");
    let sharp = ctx.star_fwhm().expect("measured on the reference");
    assert!((sharp - 2.0 * night_amplifier_core::detection::FWHM_PER_SIGMA).abs() < 0.5, "{sharp}");

    // A soft sub need not register to be measured.
    ctx.add_frame(&starfield(6.0)).expect("offered");
    ctx.add_frame(&starfield(6.0)).expect("offered");
    assert_eq!(ctx.star_fwhm(), Some(night_amplifier_core::detection::widest_measurable_fwhm()));
}

/// A starless reference cannot anchor registration, and says so by count rather than
/// as a generic stack failure.
#[test]
fn a_starless_reference_is_too_few_stars() {
    let blank = Frame::filled(64, 64, 1, 0.1).unwrap();
    assert_eq!(
        context().initialize_with_reference(&blank),
        Err(LiveStackError::TooFewStars { found: 0 })
    );
}

#[test]
fn a_frame_before_the_reference_is_refused() {
    let frame = Frame::filled(64, 64, 1, 0.1).unwrap();
    assert_eq!(context().add_frame(&frame).err(), Some(LiveStackError::NoReference));
}

/// Starting a session and editing settings mid-session must agree.
///
/// They did not: `new` hardcoded `SigmaClip` from licence state while
/// `update_from_settings` read `settings.rejection_method`, so the observer's choice
/// only took effect if they happened to touch settings after capture began. Compared
/// through the config the accumulator actually ends up running, not through the
/// resolver both now call — going through the resolver twice would pass however the
/// two paths were wired.
#[test]
fn session_start_and_a_settings_edit_configure_the_stack_alike() {
    for method in [RejectionMethod::None, RejectionMethod::SigmaClip] {
        for (preset, sigma) in [
            (WeightingPreset::Balanced, 2.5),
            (WeightingPreset::Nebulae, 1.8),
            (WeightingPreset::Disabled, 3.2),
        ] {
            let settings = CaptureSettings {
                rejection_method: method,
                weighting_preset: preset,
                rejection_sigma: sigma,
                ..Default::default()
            };
            let settings = StackSettings::of(&settings, &night_amplifier_core::plugins::Plugins::none());

            let mut at_start =
                StackingContext::new(64, 64, 3, &settings).expect("context builds");
            let from_start = at_start.stacker.config().clone();

            at_start.update_from_settings(&settings);
            let after_edit = at_start.stacker.config();

            assert_eq!(from_start.rejection, after_edit.rejection, "{method:?}");
            assert_eq!(from_start.sigma_low, after_edit.sigma_low);
            assert_eq!(from_start.sigma_high, after_edit.sigma_high);
            assert_eq!(from_start.weighting, after_edit.weighting, "{preset:?}");
        }
    }
}
