//! The settings update's rules, without a server: what `check` refuses, and what `apply`
//! changes and reports. One end-to-end case pins that a refused request applies nothing.

use std::sync::Arc;

use super::*;
use night_amplifier_core::planetary::AlignmentRoi;
use night_amplifier_core::stacking::StackingType;

const NO_PRO: ProFeatures = ProFeatures {
    saturation_boost: false,
    multi_point_planetary: false,
};

const ALL_PRO: ProFeatures = ProFeatures {
    saturation_boost: true,
    multi_point_planetary: true,
};

fn main_camera(capture_state: CaptureState) -> UpdateTarget {
    UpdateTarget {
        role: CameraRole::Main,
        profile_key: Some("ZWO/ASI533MC Pro".to_string()),
        capture_state,
    }
}

fn guide_camera() -> UpdateTarget {
    UpdateTarget {
        role: CameraRole::Guide,
        profile_key: Some("PlayerOne/Ares-C PRO".to_string()),
        capture_state: CaptureState::Idle,
    }
}

#[test]
fn the_stacking_type_only_changes_while_idle() {
    let request = UpdateSettingsRequest {
        stacking_type: Some(StackingType::Planetary),
        ..Default::default()
    };
    let current = CaptureSettings::default();

    assert!(check(&request, &current, CaptureState::Idle, NO_PRO).is_ok());
    for busy in [CaptureState::Capturing, CaptureState::Starting] {
        assert!(matches!(
            check(&request, &current, busy, NO_PRO),
            Err(ApiError::StackingTypeChangeNotAllowed)
        ));
    }
}

/// Judged on the mode the request leaves the capture in, so turning stacking on in the
/// same request as the mode cannot slip past a check of the current mode.
#[test]
fn focus_mode_is_refused_where_it_would_enter_a_running_stack() {
    let live_view = CaptureSettings {
        stacking: false,
        ..Default::default()
    };
    let enter = UpdateSettingsRequest {
        focus_mode: Some(true),
        ..Default::default()
    };
    let enter_and_stack = UpdateSettingsRequest {
        focus_mode: Some(true),
        stacking: Some(true),
        ..Default::default()
    };
    let leave = UpdateSettingsRequest {
        focus_mode: Some(false),
        ..Default::default()
    };

    assert!(check(&enter, &live_view, CaptureState::Capturing, NO_PRO).is_ok());
    assert!(matches!(
        check(&enter_and_stack, &live_view, CaptureState::Capturing, NO_PRO),
        Err(ApiError::FocusModeWhileStacking)
    ));
    assert!(check(&enter_and_stack, &live_view, CaptureState::Idle, NO_PRO).is_ok());
    let stacking = CaptureSettings::default();
    assert!(check(&leave, &stacking, CaptureState::Capturing, NO_PRO).is_ok(), "never trapped");
}

#[test]
fn pro_features_need_their_plugin() {
    let current = CaptureSettings::default();
    for request in [
        UpdateSettingsRequest {
            saturation_boost: Some(true),
            ..Default::default()
        },
        UpdateSettingsRequest {
            planetary_multi_point_alignment: Some(true),
            ..Default::default()
        },
    ] {
        let refused = check(&request, &current, CaptureState::Idle, NO_PRO).unwrap_err();
        assert!(matches!(refused, crate::error::ApiError::ProFeatureRequired(_)), "{refused:?}");
        assert!(refused.to_string().ends_with("is a Pro feature"));
        assert!(check(&request, &current, CaptureState::Idle, ALL_PRO).is_ok());
    }

    let switching_off = UpdateSettingsRequest {
        saturation_boost: Some(false),
        planetary_multi_point_alignment: Some(false),
        ..Default::default()
    };
    assert!(check(&switching_off, &current, CaptureState::Idle, NO_PRO).is_ok());
}

/// The fields before a refused Pro switch used to stay applied in memory, unsaved, so a
/// 403 still changed the session — and a restart then silently undid it.
#[tokio::test]
async fn a_refused_request_changes_nothing() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let snapshot = state.settings.snapshot();
    let before = (*snapshot).clone();

    let refused = SettingsService::update(
        &state,
        UpdateSettingsRequest {
            gain: Some(321),
            auto_stretch: Some(!before.auto_stretch),
            saturation_boost: Some(true),
            ..Default::default()
        },
    )
    .await;

    assert!(matches!(refused, Err(ApiError::ProFeatureRequired(_))));
    let after = state.settings.snapshot();
    assert_eq!(after.gain, before.gain);
    assert_eq!(after.auto_stretch, before.auto_stretch);
    assert!(Arc::ptr_eq(&snapshot, &after), "a refusal still replaced the settings in force");
}

/// A request's reactions run after awaits, so a later request can land between its update
/// and them. The resume plan then takes the settings in force: the late request's own
/// copy put the earlier edit back, and a resume after a reconnect undid the later one.
#[tokio::test]
async fn a_late_reaction_leaves_the_newer_settings_in_the_resume_plan() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.resume.record(crate::state::SessionResumePlan {
        camera_id: "mock_0".to_string(),
        settings: (*state.settings.snapshot()).clone(),
        disk_session_dir: None,
        next_frame: 1,
    });
    let target = main_camera(CaptureState::Idle);
    // Request A applied; request B lands before A's reactions run.
    let request_a = UpdateSettingsRequest {
        gain: Some(111),
        ..Default::default()
    };
    let delta = state
        .settings
        .update(|settings| apply(request_a, settings, &target, &state.plugins));
    state.settings.update(|settings| settings.gain = 222);

    SettingsService::react(&state, &target, delta).await;

    let planned = state.resume.plan().map(|plan| plan.settings.gain);
    assert_eq!(planned, Some(222), "the late reaction put the older request back");
}

/// A guide request edits the guide camera's own profile and leaves the imaging camera's
/// flat fields alone — and the profile is remembered against the guide camera's key.
#[test]
fn hardware_fields_land_on_the_camera_the_request_names() {
    let mut settings = CaptureSettings::default();
    let main_gain = settings.gain;
    let request = UpdateSettingsRequest {
        camera_role: Some(CameraRole::Guide),
        gain: Some(250),
        exposure_us: Some(500_000),
        ..Default::default()
    };

    apply(request, &mut settings, &guide_camera(), &Plugins::none());

    assert_eq!(settings.guide_camera.gain, 250);
    assert_eq!(settings.guide_camera.exposure_us, 500_000);
    assert_eq!(settings.gain, main_gain, "the imaging camera was not addressed");
    assert_eq!(settings.camera_profiles["PlayerOne/Ares-C PRO"].gain, 250);
}

#[test]
fn every_value_lands_inside_its_range() {
    let mut settings = CaptureSettings::default();
    let request = UpdateSettingsRequest {
        rejection_sigma: Some(50.0),
        auto_stretch_intensity: Some(2.0),
        saturation_boost_strength: Some(-1.0),
        simulated_preload_images: Some(0),
        target_temp_c: Some(-100.0),
        dew_heater_power: Some(150),
        ..Default::default()
    };

    apply(request, &mut settings, &main_camera(CaptureState::Idle), &Plugins::none());

    assert_eq!(settings.rejection_sigma, 10.0);
    assert_eq!(settings.auto_stretch_intensity, 1.0);
    assert_eq!(settings.saturation_boost_strength, 0.0);
    assert_eq!(settings.simulated_preload_images, 1);
    assert_eq!(settings.target_temp_c, Some(-60.0));
    assert_eq!(settings.dew_heater_power, 100);
    let remembered = &settings.camera_profiles["ZWO/ASI533MC Pro"];
    assert_eq!((remembered.target_temp_c, remembered.dew_heater_power), (Some(-60.0), 100));
}

/// Each reaction is keyed off what the request carried, never off a field it left out.
#[test]
fn the_delta_names_what_the_request_touched() {
    let target = main_camera(CaptureState::Idle);
    let quiet = apply(
        UpdateSettingsRequest {
            auto_stretch: Some(false),
            ..Default::default()
        },
        &mut CaptureSettings::default(),
        &target, &Plugins::none(),
    );
    assert_eq!(quiet, SettingsDelta::default());

    let busy = apply(
        UpdateSettingsRequest {
            bin: Some(2),
            target_temp_c: Some(-5.0),
            dew_heater_power: Some(30),
            ..Default::default()
        },
        &mut CaptureSettings::default(),
        &target, &Plugins::none(),
    );
    assert!(busy.exposure && busy.cooler && busy.dew_heater);
    assert!(busy.optics.framing, "binning moves the field of view");
    assert!(!busy.optics.telescope);
}

/// The regions are set by drawing them; a request that omits one leaves it drawn.
#[test]
fn a_region_is_set_by_sending_it_and_kept_by_leaving_it_out() {
    let roi = AlignmentRoi {
        x: 1,
        y: 2,
        width: 30,
        height: 40,
    };
    let mut settings = CaptureSettings::default();
    let target = main_camera(CaptureState::Idle);

    apply(
        UpdateSettingsRequest {
            comet_roi: Some(roi),
            ..Default::default()
        },
        &mut settings,
        &target, &Plugins::none(),
    );
    apply(UpdateSettingsRequest::default(), &mut settings, &target, &Plugins::none());

    assert_eq!(settings.comet_roi.map(|r| (r.x, r.width)), Some((1, 30)));
}

/// The toggle is a statement about the whole managed group, so it wins over a managed
/// setting sent in the same request.
#[test]
fn the_focus_mode_toggle_wins_over_a_managed_setting_beside_it() {
    let mut settings = CaptureSettings {
        stacking: false,
        background_subtraction: true,
        ..Default::default()
    };

    apply(
        UpdateSettingsRequest {
            focus_mode: Some(true),
            background_subtraction: Some(true),
            ..Default::default()
        },
        &mut settings,
        &main_camera(CaptureState::Idle), &Plugins::none(),
    );

    assert!(settings.focus_mode && settings.focus_mode_snapshot.is_some());
    assert!(!settings.background_subtraction, "held off by the mode");
}

/// A running live view switched to stacking: no start path sees it, so the update has to
/// end the mode itself, and say so.
#[test]
fn switching_a_running_live_view_to_stacking_leaves_focus_mode() {
    let mut settings = CaptureSettings {
        stacking: false,
        ..Default::default()
    };
    focus_mode::set(&mut settings, true, &night_amplifier_core::plugins::Plugins::none());

    let delta = apply(
        UpdateSettingsRequest {
            stacking: Some(true),
            ..Default::default()
        },
        &mut settings,
        &main_camera(CaptureState::Capturing), &Plugins::none(),
    );

    assert!(delta.left_focus_mode);
    assert!(!settings.focus_mode && settings.focus_mode_snapshot.is_none());
}
