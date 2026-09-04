//! Behaviour that only exists once a guide camera can be connected alongside the
//! imaging one: role-scoped settings, the plate-solve source, and guide-scope optics.

use std::sync::Arc;

use super::helpers::*;
use crate::camera::CameraInfo;
use crate::server::capture::solving::{plate_solve_available, SolveSource};
use crate::server::state::*;

/// Install a connected camera in `role` without opening a device.
async fn register_camera(state: &Arc<AppState>, role: CameraRole, id: &str, name: &str) {
    state.cameras.write().await.insert(
        id.to_string(),
        ConnectedCameraInfo {
            id: id.to_string(),
            provider: "Mock".to_string(),
            index: 0,
            role,
            info: CameraInfo {
                name: name.to_string(),
                has_cooler: true,
                min_temp_c: Some(-40.0),
                max_temp_c: Some(20.0),
                ..Default::default()
            },
        },
    );
    if role == CameraRole::Guide {
        state.set_guide_loop_running(true);
    }
}

/// The two cameras run at wildly different exposures — seconds versus minutes — so an
/// edit aimed at one must leave the other exactly where it was.
#[tokio::test]
async fn guide_settings_do_not_disturb_the_main_camera() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));

    let (status, _) = post_json(
        &app,
        "/api/settings",
        serde_json::json!({"exposure_us": 60_000_000, "gain": 100}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);

    let (status, body) = post_json(
        &app,
        "/api/settings",
        serde_json::json!({"camera_role": "guide", "exposure_us": 2_000_000, "gain": 300}),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK);

    assert_eq!(body["data"]["exposure_us"], 60_000_000);
    assert_eq!(body["data"]["gain"], 100);
    assert_eq!(body["data"]["guide_camera"]["exposure_us"], 2_000_000);
    assert_eq!(body["data"]["guide_camera"]["gain"], 300);

    let settings = state.settings.read().await;
    assert_eq!(settings.exposure_us, 60_000_000);
    assert_eq!(settings.guide_camera.exposure_us, 2_000_000);
}

/// A request with no `camera_role` is the imaging camera, so every client that predates
/// roles keeps working untouched.
#[tokio::test]
async fn a_settings_request_without_a_role_edits_the_main_camera() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));

    let (status, _) = post_json(&app, "/api/settings", serde_json::json!({"gain": 42})).await;
    assert_eq!(status, axum::http::StatusCode::OK);

    let settings = state.settings.read().await;
    assert_eq!(settings.gain, 42);
    assert_eq!(
        settings.guide_camera.gain, 0,
        "a role-less request leaked into the guide camera"
    );
}

/// `profile_for` is what every consumer reads a camera's hardware values through, so it
/// has to agree with what the settings endpoint just wrote.
#[tokio::test]
async fn profile_for_reads_each_roles_own_fields() {
    let mut settings = CaptureSettings {
        exposure_us: 30_000_000,
        ..Default::default()
    };
    settings.guide_camera.exposure_us = 1_500_000;

    assert_eq!(
        settings.profile_for(CameraRole::Main).exposure_us,
        30_000_000
    );
    assert_eq!(
        settings.profile_for(CameraRole::Guide).exposure_us,
        1_500_000
    );
}

/// Exactly one camera solves at a time. With no guide loop running it is the imaging
/// camera; with one it is the guide camera, and the imaging path must stand down —
/// otherwise which optics a solve was planned against becomes a race.
#[tokio::test]
async fn the_guide_camera_takes_over_plate_solving() {
    let state = create_test_state();

    assert!(!state.guide_loop_running());
    assert!(
        !plate_solve_available(&state, SolveSource::Guide),
        "the guide source offered frames with no guide camera connected"
    );

    register_camera(&state, CameraRole::Guide, "mock_1", "Guiding").await;

    assert!(
        !plate_solve_available(&state, SolveSource::Main),
        "the imaging camera kept offering frames after a guide camera connected"
    );
}

/// The guide camera sits on a guide scope. Handing ASTAP the imaging scope's field for
/// guide-scope frames sends it hunting at the wrong scale.
#[tokio::test]
async fn the_solver_uses_the_solving_cameras_own_optics() {
    let mut settings = CaptureSettings {
        telescope: TelescopeSettings {
            focal_length_mm: Some(1000.0),
            ..Default::default()
        },
        ..Default::default()
    };
    settings.camera_telescope_profiles.insert(
        "Guiding".to_string(),
        TelescopeSettings {
            focal_length_mm: Some(240.0),
            ..Default::default()
        },
    );

    assert_eq!(
        settings.solver_telescope(Some("Guiding")).focal_length_mm,
        Some(240.0)
    );
    // A camera with no profile of its own falls back to the shared block rather than
    // reporting nothing.
    assert_eq!(
        settings.solver_telescope(Some("Imaging")).focal_length_mm,
        Some(1000.0)
    );
    assert_eq!(settings.solver_telescope(None).focal_length_mm, Some(1000.0));
}

/// The camera list has to say which position each camera holds, or the UI cannot show a
/// role badge or know which Connect actions are still available.
#[tokio::test]
async fn the_camera_list_reports_each_cameras_role() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));

    register_camera(&state, CameraRole::Main, "mock_0", "Imaging").await;
    register_camera(&state, CameraRole::Guide, "mock_1", "Guiding").await;

    let (status, body) = get_json(&app, "/api/cameras").await;
    assert_eq!(status, axum::http::StatusCode::OK);

    let cameras = body["data"].as_array().expect("no camera list");
    let role_of = |name: &str| {
        cameras
            .iter()
            .find(|c| c["name"] == name)
            .and_then(|c| c["role"].as_str())
            .map(str::to_string)
    };
    assert_eq!(role_of("Imaging"), Some("main".to_string()));
    assert_eq!(role_of("Guiding"), Some("guide".to_string()));
}

/// Binning and sensor mode live in the camera's own profile, so a guide-camera request
/// must be compared against the guide camera's values — not the imaging camera's, which
/// would report a framing change that never happened (and restart a healthy solve).
#[tokio::test]
async fn a_framing_change_is_judged_against_the_role_it_was_sent_for() {
    use crate::server::api::settings::optics_change;
    use crate::server::dto::UpdateSettingsRequest;

    let mut settings = CaptureSettings {
        bin: 1,
        ..Default::default()
    };
    settings.guide_camera.bin = 2;

    let request = UpdateSettingsRequest {
        bin: Some(2),
        ..Default::default()
    };

    assert!(
        optics_change(&request, &settings, CameraRole::Main).framing,
        "bin 1 -> 2 on the imaging camera is a framing change"
    );
    assert!(
        !optics_change(&request, &settings, CameraRole::Guide).framing,
        "the guide camera was already at bin 2 — nothing changed"
    );
}

/// The per-camera map is what the solver reads, so rewriting it moves the FOV hint even
/// when the shared telescope block is untouched.
#[tokio::test]
async fn rewriting_the_optics_profiles_counts_as_a_telescope_change() {
    use crate::server::api::settings::optics_change;
    use crate::server::dto::UpdateSettingsRequest;

    let settings = CaptureSettings::default();
    let mut profiles = std::collections::HashMap::new();
    profiles.insert(
        "Guiding".to_string(),
        TelescopeSettings {
            focal_length_mm: Some(240.0),
            ..Default::default()
        },
    );

    let request = UpdateSettingsRequest {
        camera_telescope_profiles: Some(profiles),
        ..Default::default()
    };

    assert!(optics_change(&request, &settings, CameraRole::Guide).telescope);
}

/// A guide camera that has never been configured must still produce a config a real SDK
/// will take: `CaptureConfig::validate` rejects `bin: 0` and a sub-minimum exposure
/// outright, and `guide_task` would then reject every frame for the life of the session.
#[tokio::test]
async fn a_fresh_guide_profile_builds_a_usable_capture_config() {
    use crate::camera::{CameraInfo, ImageFormat, SensorType};

    let settings = CaptureSettings::default();
    let info = CameraInfo {
        name: "Guiding".to_string(),
        max_width: 1936,
        max_height: 1096,
        sensor_type: SensorType::Mono,
        supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
        ..Default::default()
    };

    // The loop clears `sensor_mode` for a camera with no modes; do the same here so the
    // comparison is about the profile's own fields.
    let mut guide = settings.to_capture_config_for(CameraRole::Guide);
    guide.sensor_mode = None;

    assert_eq!(guide.exposure_us, 1_000_000);
    assert_eq!(guide.bin, 1);
    assert!(
        guide.validate(&info).is_ok(),
        "a fresh guide profile must be capturable: {:?}",
        guide.validate(&info)
    );
    assert!(settings.guide_camera.dew_heater_enabled);
    assert_eq!(settings.guide_camera.dew_heater_power, 10);
}

/// The two live field sets are the same ten values in two places, so they must start
/// from one list — a second literal block is a place for them to drift apart.
#[test]
fn both_roles_start_from_the_same_hardware_defaults() {
    let settings = CaptureSettings::default();
    let main = settings.profile_for(CameraRole::Main);
    let guide = settings.profile_for(CameraRole::Guide);

    assert_eq!(main.exposure_us, guide.exposure_us);
    assert_eq!(main.gain, guide.gain);
    assert_eq!(main.offset, guide.offset);
    assert_eq!(main.bin, guide.bin);
    assert_eq!(main.dew_heater_enabled, guide.dew_heater_enabled);
    assert_eq!(main.dew_heater_power, guide.dew_heater_power);
}

/// A profile is stored under the camera's own key, so whatever a guide connect writes
/// there is what the imaging connect later reads. It must be a profile that camera can
/// actually capture with — including when the stored one predates a real `Default` and
/// holds zeros.
#[tokio::test]
async fn a_guide_connect_leaves_a_profile_the_imaging_camera_can_use() {
    use crate::camera::{CameraInfo, ImageFormat, SensorType};
    use crate::server::camera_session::lifecycle::apply_camera_profile_on_connect;

    let info = CameraInfo {
        name: "Dual Duty".to_string(),
        max_width: 1936,
        max_height: 1096,
        sensor_type: SensorType::Mono,
        supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
        ..Default::default()
    };
    let key = |role| {
        crate::server::camera_session::lifecycle::camera_profile_key("Mock", "Dual Duty", role)
    };

    let mut settings = CaptureSettings::default();

    // Night one: the camera is attached to the guide scope.
    apply_camera_profile_on_connect(&mut settings, key(CameraRole::Guide), CameraRole::Guide, &info);
    // Night two: the same camera is moved to the main scope.
    apply_camera_profile_on_connect(&mut settings, key(CameraRole::Main), CameraRole::Main, &info);

    assert_eq!(settings.exposure_us, 1_000_000);
    assert_eq!(settings.bin, 1);
    let mut config = settings.to_capture_config();
    config.sensor_mode = None;
    assert!(config.validate(&info).is_ok());
}

/// Settings files written before `CameraCaptureProfile` had a real `Default` hold
/// `exposure_us: 0, bin: 0`. Connecting must repair them rather than apply them.
#[tokio::test]
async fn a_zeroed_stored_profile_is_repaired_on_connect() {
    use crate::camera::{CameraInfo, ImageFormat, SensorType};
    use crate::server::camera_session::lifecycle::{
        apply_camera_profile_on_connect, camera_profile_key,
    };

    let info = CameraInfo {
        name: "Guiding".to_string(),
        supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
        sensor_type: SensorType::Mono,
        ..Default::default()
    };
    let key = camera_profile_key("Mock", "Guiding", CameraRole::Guide);

    let mut settings = CaptureSettings::default();
    settings.camera_profiles.insert(
        key.clone(),
        CameraCaptureProfile {
            exposure_us: 0,
            gain: 0,
            offset: 0,
            bin: 0,
            cooler_enabled: false,
            target_temp_c: None,
            sensor_mode_override: None,
            cooler_fast_mode: false,
            dew_heater_enabled: false,
            dew_heater_power: 0,
        },
    );

    apply_camera_profile_on_connect(&mut settings, key.clone(), CameraRole::Guide, &info);

    assert_eq!(settings.guide_camera.exposure_us, 1_000_000);
    assert_eq!(settings.guide_camera.bin, 1);
    assert_eq!(
        settings.camera_profiles[&key].bin, 1,
        "the repair must be written back, or the next connect reads the zeros again"
    );
}

/// Dual sampling trades frame rate for read noise, and only an integrated frame pays
/// that back. Nothing the guide camera produces is stacked, so starting a Deep Sky
/// session must not drag its sensor into `LowReadoutNoise`.
#[tokio::test]
async fn the_guide_camera_keeps_its_sensor_mode_when_the_stack_starts() {
    use crate::camera::DualSamplingMode;

    let settings = CaptureSettings {
        stacking: true,
        stacking_type: StackingType::DeepSky,
        ..Default::default()
    };

    assert_eq!(
        settings.to_capture_config_for(CameraRole::Main).sensor_mode,
        Some(DualSamplingMode::LowReadoutNoise),
        "the imaging camera is the one integrating frames"
    );
    assert_eq!(
        settings.to_capture_config_for(CameraRole::Guide).sensor_mode,
        Some(DualSamplingMode::Normal)
    );
}

/// An explicit override still wins for either camera — it is the user saying they know
/// what their sensor does.
#[tokio::test]
async fn a_guide_sensor_mode_override_still_wins() {
    use crate::camera::DualSamplingMode;

    let mut settings = CaptureSettings::default();
    settings.guide_camera.sensor_mode_override = Some(DualSamplingMode::LowReadoutNoise);

    assert_eq!(
        settings.to_capture_config_for(CameraRole::Guide).sensor_mode,
        Some(DualSamplingMode::LowReadoutNoise)
    );
}


// ---- What a refused capture says, and to whom ------------------------------------
//
// The panel on the left picks which camera the settings apply to, and that can be the
// guide camera. Pressing Start in that state used to answer with the camera's wire id
// and a remedy for a request the user had not made.

/// Errors name the camera the way the user does. `simulator_0` is not something anyone
/// chose, recognises, or can act on; the display name carries the model for a real
/// camera and the fixture directory for a simulated one.
#[tokio::test]
async fn a_capture_refused_on_the_guide_camera_names_the_camera() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));
    register_camera(
        &state,
        CameraRole::Guide,
        "simulator_0",
        "Simulator: 35mm-imx464-orion-tiff (17 files)",
    )
    .await;

    let (status, body) = post_json(
        &app,
        "/api/capture/start",
        serde_json::json!({"camera_id": "simulator_0"}),
    )
    .await;

    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("Simulator: 35mm-imx464-orion-tiff (17 files)"),
        "the message must name the camera, got: {error}"
    );
    assert!(
        !error.contains("simulator_0"),
        "the wire id has no business in a message a person reads: {error}"
    );
    assert_eq!(status, axum::http::StatusCode::CONFLICT);
}

/// The remedy has to match the request. Telling someone who pressed Start to
/// "disconnect it before connecting it as the main camera" describes an action they did
/// not take and do not want.
#[tokio::test]
async fn a_capture_refused_on_the_guide_camera_does_not_talk_about_connecting() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));
    register_camera(&state, CameraRole::Guide, "mock_1", "Guiding").await;

    let (_, body) = post_json(
        &app,
        "/api/capture/start",
        serde_json::json!({"camera_id": "mock_1"}),
    )
    .await;

    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        !error.contains("connect"),
        "a refused capture is not a connect problem: {error}"
    );
    assert!(
        error.contains("guide"),
        "it should still say which position the camera holds: {error}"
    );
}

/// A role conflict is something the client can act on, so it must not arrive as a 500.
/// The status used to come from a `match` in the handler that had no arm for this
/// error and fell through to "internal".
#[tokio::test]
async fn a_role_conflict_is_reported_as_a_conflict_not_an_internal_error() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));
    register_camera(&state, CameraRole::Guide, "mock_1", "Guiding").await;

    let (status, _) = post_json(
        &app,
        "/api/capture/start",
        serde_json::json!({"camera_id": "mock_1"}),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::CONFLICT);
}

/// With no imaging camera at all, the honest answer is that one is missing — not a
/// complaint about whichever camera the settings panel happened to be showing.
#[tokio::test]
async fn starting_with_only_a_guide_camera_asks_for_an_imaging_one() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));
    register_camera(&state, CameraRole::Guide, "mock_1", "Guiding").await;

    let (status, body) = post_json(&app, "/api/capture/start", serde_json::json!({})).await;

    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("imaging camera"),
        "the message should name what is missing: {error}"
    );
}

/// A camera that is not connected has no display name to look up, and inventing one
/// would be worse than echoing what the client sent.
#[tokio::test]
async fn an_unknown_camera_id_is_still_reported_verbatim() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));

    let (status, body) = post_json(
        &app,
        "/api/capture/start",
        serde_json::json!({"camera_id": "mock_missing"}),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
    assert!(body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("mock_missing"));
}

/// Starting on the imaging camera is unaffected by any of the above.
#[tokio::test]
async fn a_capture_on_the_imaging_camera_is_not_refused_for_its_role() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));
    register_camera(&state, CameraRole::Main, "mock_0", "Imaging").await;

    let (status, body) = post_json(
        &app,
        "/api/capture/start",
        serde_json::json!({"camera_id": "mock_0"}),
    )
    .await;

    assert_ne!(status, axum::http::StatusCode::CONFLICT, "body: {body}");
}

/// The name lookup is by id and role-blind: it answers for whichever camera holds the
/// id, and says nothing for one that is not connected.
#[tokio::test]
async fn the_name_lookup_answers_only_for_connected_cameras() {
    let state = create_test_state();
    register_camera(&state, CameraRole::Guide, "mock_1", "Guiding").await;
    register_camera(&state, CameraRole::Main, "mock_0", "Imaging").await;

    assert_eq!(
        state.connected_camera_name("mock_1").await.as_deref(),
        Some("Guiding")
    );
    assert_eq!(
        state.connected_camera_name("mock_0").await.as_deref(),
        Some("Imaging")
    );
    assert_eq!(state.connected_camera_name("mock_missing").await, None);
}
