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
