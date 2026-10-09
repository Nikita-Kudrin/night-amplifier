//! The viewed camera through the camera lifecycle: what each way of losing, recovering or
//! replacing a camera does to what the eyepiece pages show.

use std::sync::Arc;
use std::time::Duration;

use super::lifecycle::{self, DisconnectCause, WarmupPolicy};
use super::recovery_tests::{
    connect, drain, id_of, rig, teardown, wait_recovered, FakeCatalog, ARES, NEPTUNE,
};
use super::reconnect;
use super::tests::{eventually, install_camera};
use crate::events::ServerEvent;
use crate::services::CameraService;
use crate::state::{AppState, CameraPhase, CameraRole, ConnectedCameraInfo};

fn viewed_events(rx: &mut tokio::sync::broadcast::Receiver<ServerEvent>) -> Vec<CameraRole> {
    drain(rx)
        .into_iter()
        .filter_map(|event| match event {
            ServerEvent::ViewedCameraChanged { camera } => Some(camera),
            _ => None,
        })
        .collect()
}

fn view_guide(state: &AppState) {
    CameraService::select_viewed_camera(state, CameraRole::Guide).expect("guide camera connected");
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnecting_the_viewed_guide_camera_returns_the_view_to_the_imaging_camera() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &ARES, CameraRole::Main).await;
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    view_guide(&state);
    let mut events = state.subscribe_events();

    lifecycle::disconnect(&state, &id_of(&NEPTUNE), WarmupPolicy::Skip).await.unwrap();

    assert_eq!(state.viewed_camera.get(), CameraRole::Main);
    assert_eq!(viewed_events(&mut events), [CameraRole::Main], "announced once");
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guide_disconnect_while_the_imaging_camera_is_viewed_says_nothing() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &ARES, CameraRole::Main).await;
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    let mut events = state.subscribe_events();

    lifecycle::disconnect(&state, &id_of(&NEPTUNE), WarmupPolicy::Skip).await.unwrap();

    assert_eq!(state.viewed_camera.get(), CameraRole::Main);
    assert!(viewed_events(&mut events).is_empty());
    teardown(&state).await;
}

/// A USB hiccup recovery absorbs must not cost the viewers their picture: the eyepiece
/// keeps the guide camera, and shows it again once it is back.
#[tokio::test(flavor = "multi_thread")]
async fn a_recovered_guide_fault_keeps_the_view_on_the_guide_camera() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &ARES, CameraRole::Main).await;
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    view_guide(&state);
    let mut events = state.subscribe_events();

    lifecycle::finalize_disconnect(&state, CameraRole::Guide, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert_eq!(state.viewed_camera.get(), CameraRole::Guide, "while recovering");
    assert!(wait_recovered(&state, CameraRole::Guide).await, "guide camera never came back");

    assert_eq!(state.viewed_camera.get(), CameraRole::Guide, "after recovering");
    assert!(viewed_events(&mut events).is_empty());
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guide_recovery_that_gives_up_returns_the_view_to_the_imaging_camera() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    view_guide(&state);
    let mut events = state.subscribe_events();

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Guide, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(
        eventually(
            || state.camera_in_role(CameraRole::Guide).is_none(),
            reconnect::TOTAL_BUDGET + Duration::from_secs(3)
        )
        .await,
        "recovery should give up once the budget is spent"
    );

    assert_eq!(state.viewed_camera.get(), CameraRole::Main);
    assert_eq!(viewed_events(&mut events), [CameraRole::Main]);
}

/// Replacing the guide body goes through a disconnect, so the operator re-selects it: the
/// view never points at a role mid-swap, and the new body is what it then shows.
#[tokio::test(flavor = "multi_thread")]
async fn swapping_the_guide_camera_returns_the_view_until_the_operator_picks_it_again() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    view_guide(&state);

    lifecycle::connect(&state, &id_of(&ARES), CameraRole::Guide).await.unwrap();

    assert_eq!(state.viewed_camera.get(), CameraRole::Main);
    view_guide(&state);
    assert_eq!(state.camera_in_role(CameraRole::Guide).unwrap().info.name, ARES.name);
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn losing_the_imaging_camera_leaves_the_guide_view_alone() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &ARES, CameraRole::Main).await;
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    view_guide(&state);

    lifecycle::disconnect(&state, &id_of(&ARES), WarmupPolicy::Skip).await.unwrap();

    assert_eq!(state.viewed_camera.get(), CameraRole::Guide);
    teardown(&state).await;
}

/// The operator's toggle racing the guide camera's disconnect: whichever lands first, the
/// view must never be left on a role nobody holds — every viewer would freeze on a
/// stream nothing produces, with no way for them to change it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_select_racing_a_guide_disconnect_never_strands_the_view() {
    const NAME: &str = "Racing Guide";
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    for iteration in 0..3000 {
        state.roster.install(
            ConnectedCameraInfo {
                id: "mock_guide".to_string(),
                provider: "Mock".to_string(),
                index: 1,
                role: CameraRole::Guide,
                info: night_amplifier_core::camera::CameraInfo {
                    name: NAME.to_string(),
                    ..Default::default()
                },
            },
            false,
        );
        let disconnecting = Arc::clone(&state);
        let disconnect = tokio::spawn(async move {
            lifecycle::finalize_disconnect(&disconnecting, CameraRole::Guide, NAME, DisconnectCause::Requested).await;
        });
        let selecting = Arc::clone(&state);
        let select = tokio::task::spawn_blocking(move || {
            CameraService::select_viewed_camera(&selecting, CameraRole::Guide)
        });
        // Either answer is fine; only the end state is the invariant.
        let _ = select.await.unwrap();
        disconnect.await.unwrap();

        assert!(
            state.camera_in_role(CameraRole::Guide).is_none(),
            "iteration {iteration}: the disconnect did not finish"
        );
        assert_eq!(
            state.viewed_camera.get(),
            CameraRole::Main,
            "iteration {iteration}: the view was left on an empty guide role"
        );
    }
}

/// A cooled guide camera's Disconnect stops its loop at once but closes only after a
/// warm-up of up to five minutes. Every viewer must not spend that on a frozen frame.
#[tokio::test(flavor = "multi_thread")]
async fn a_guide_camera_warming_up_to_disconnect_gives_the_view_back() {
    const NAME: &str = "Cooled Guide";
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.update(|s| s.guide_camera.cooler_enabled = true);
    install_camera(&state, CameraRole::Guide, "mock_1", NAME, CameraPhase::Guiding).await;
    view_guide(&state);

    let outcome = lifecycle::disconnect(&state, "mock_1", WarmupPolicy::WhenPossible).await.unwrap();
    assert!(matches!(outcome, lifecycle::DisconnectOutcome::WarmingUp { .. }), "{outcome:?}");
    assert_eq!(state.camera_phase(CameraRole::Guide), CameraPhase::WarmingUp);

    assert_eq!(state.viewed_camera.get(), CameraRole::Main, "the guide loop has stopped");
    assert!(
        CameraService::select_viewed_camera(&state, CameraRole::Guide).is_err(),
        "a camera on its way out produces no frames to show"
    );
}
