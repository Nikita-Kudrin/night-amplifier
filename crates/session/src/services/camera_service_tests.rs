//! A camera wedged inside its SDK while being enumerated — QHY and ZWO discovery open every
//! idle device — against the camera list. Here rather than in the server's endpoint tests:
//! they rely on the test-time `DISCOVERY_TIMEOUT`, which only this crate's tests compile.

use super::*;
use night_amplifier_core::camera::{
    CameraEntry, CameraError, CameraInfo, CameraResult, DeviceCatalog, DeviceIdentity,
    OpenedCamera,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

/// "Hung" blocks inside `list` until released; "Healthy" lists one camera.
struct HungCatalog {
    release: Mutex<Receiver<()>>,
    hung_calls: AtomicUsize,
}

impl DeviceCatalog for HungCatalog {
    fn provider_names(&self, _: bool) -> Vec<String> {
        vec!["Hung".to_string(), "Healthy".to_string()]
    }
    fn list(&self, provider: &str, _: bool) -> CameraResult<Vec<CameraEntry>> {
        if provider == "Healthy" {
            return Ok(vec![CameraEntry {
                provider: provider.to_string(),
                index: 0,
                info: CameraInfo {
                    name: "Healthy Cam".to_string(),
                    serial: Some("HEALTHY1".to_string()),
                    ..Default::default()
                },
            }]);
        }
        self.hung_calls.fetch_add(1, Ordering::SeqCst);
        // Returns once the test drops the sender.
        let _ = self.release.lock().unwrap().recv_timeout(Duration::from_secs(30));
        Ok(Vec::new())
    }
    fn identities(&self, provider: &str, _: bool) -> CameraResult<Vec<DeviceIdentity>> {
        Err(CameraError::ProviderNotFound(provider.to_string()))
    }
    fn open(&self, provider: &str, _: usize, _: bool) -> CameraResult<OpenedCamera> {
        Err(CameraError::ProviderNotFound(provider.to_string()))
    }
}

fn rig() -> (Arc<AppState>, Arc<HungCatalog>, Sender<()>) {
    let (release, released) = mpsc::channel();
    let catalog = Arc::new(HungCatalog {
        release: Mutex::new(released),
        hung_calls: AtomicUsize::new(0),
    });
    let (mut state, _disk_writer) = AppState::new_for_testing();
    state.device_catalog = Arc::clone(&catalog) as Arc<dyn DeviceCatalog>;
    (Arc::new(state), catalog, release)
}

#[tokio::test]
async fn a_hung_discovery_does_not_block_the_camera_list() {
    let (state, _catalog, release) = rig();
    let listed =
        tokio::time::timeout(Duration::from_secs(4), CameraService::list_cameras(&state)).await;
    drop(release);
    assert!(listed.is_ok(), "the camera list waited on a hung SDK enumeration");
}

#[tokio::test]
async fn a_hung_provider_does_not_hide_the_others() {
    let (state, _catalog, release) = rig();
    let listed = CameraService::list_cameras(&state).await;
    drop(release);
    let names: Vec<&str> = listed.iter().map(|camera| camera.name.as_str()).collect();
    assert_eq!(names, ["Healthy Cam"]);
}

/// Each refresh used to park one more blocking thread behind the stuck device.
#[tokio::test]
async fn refreshing_behind_a_hung_enumeration_starts_no_new_one() {
    let (state, catalog, release) = rig();
    CameraService::list_cameras(&state).await;
    CameraService::list_cameras(&state).await;
    let calls = catalog.hung_calls.load(Ordering::SeqCst);
    drop(release);
    assert_eq!(calls, 1);
}

fn indi_camera(id: i32, name: &str) -> CameraListItem {
    indi_list_item(&CameraInfo {
        id,
        name: name.to_string(),
        ..Default::default()
    })
}

/// Only a camera the last discovery did not find is announced again, and one the server no
/// longer lists leaves the set.
#[test]
fn a_discovery_replaces_the_indi_cameras_and_names_the_new_ones() {
    let cameras = DiscoveredCameras::default();
    let ids = |items: Vec<CameraListItem>| items.into_iter().map(|camera| camera.id).collect::<Vec<_>>();

    assert_eq!(ids(cameras.replace_indi(vec![indi_camera(0, "CCD A"), indi_camera(1, "CCD B")])), ["indi_0", "indi_1"]);
    assert_eq!(ids(cameras.replace_indi(vec![indi_camera(1, "CCD B"), indi_camera(2, "CCD C")])), ["indi_2"]);
    assert_eq!(ids(cameras.indi()), ["indi_1", "indi_2"]);
}

/// A list built while discovery still runs carries what the last one found.
#[tokio::test]
async fn the_camera_list_carries_the_indi_cameras_last_discovered() {
    let (state, _catalog, release) = rig();
    drop(release);
    CameraService::list_cameras(&state).await;
    state.discovered_cameras.replace_indi(vec![indi_camera(0, "CCD A")]);

    let listed = CameraService::list_known_cameras(&state).await;
    let indi = listed.iter().find(|camera| camera.provider.as_deref() == Some("indi"));
    assert_eq!(indi.map(|camera| (camera.id.as_str(), camera.connected)), Some(("indi_0", false)));
}

/// An INDI server that no longer answers lists no cameras: its last ones stayed listed,
/// and connecting one could only fail.
#[tokio::test]
async fn an_indi_server_that_does_not_answer_lists_no_cameras() {
    let (state, _catalog, release) = rig();
    drop(release);
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    state.settings.update(|settings| {
        settings.indi_server_host = "127.0.0.1".to_string();
        settings.indi_server_port = closed_port;
    });
    state.discovered_cameras.replace_indi(vec![indi_camera(0, "CCD A")]);

    CameraService::list_cameras(&state).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !state.discovered_cameras.indi().is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "the unreachable server's cameras stayed listed");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A sync on every reconnect and lag enumerated the USB bus each time, for every page.
#[tokio::test]
async fn a_known_camera_list_reuses_the_last_enumeration() {
    let (state, catalog, release) = rig();
    drop(release);

    let first = CameraService::list_known_cameras(&state).await;
    assert_eq!(catalog.hung_calls.load(Ordering::SeqCst), 1, "nothing was enumerated yet");
    let again = CameraService::list_known_cameras(&state).await;

    assert_eq!(catalog.hung_calls.load(Ordering::SeqCst), 1);
    let ids = |items: &[CameraListItem]| items.iter().map(|camera| camera.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&again), ids(&first));
    CameraService::list_cameras(&state).await;
    assert_eq!(catalog.hung_calls.load(Ordering::SeqCst), 2, "a refresh still enumerates");
}

/// The simulator setting changes what there is to find, so a list made under the other one
/// is not reused.
#[tokio::test]
async fn a_known_camera_list_enumerates_again_once_the_simulator_setting_changed() {
    let (state, catalog, release) = rig();
    drop(release);
    CameraService::list_cameras(&state).await;

    let simulated = !state.settings.snapshot().use_simulated_camera;
    state.settings.update(|settings| settings.use_simulated_camera = simulated);
    CameraService::list_known_cameras(&state).await;

    assert_eq!(catalog.hung_calls.load(Ordering::SeqCst), 2);
}
