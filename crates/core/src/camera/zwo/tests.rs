use super::*;

#[test]
fn test_provider_available() {
    let provider = ZwoProvider::new();
    assert!(provider.is_available());
    assert_eq!(provider.name(), "ZWO");
}

#[test]
fn test_num_cameras_returns_zero_without_sdk() {
    // Without the actual SDK .so loaded, num_cameras should return 0
    let count = shim::num_cameras();
    assert_eq!(count, 0);
}

#[test]
fn test_get_camera_ids_without_hardware() {
    // Without hardware, get_camera_ids returns either None (SDK not installed)
    // or Some(empty map) (SDK installed, no cameras connected)
    let ids = shim::get_camera_ids();
    match ids {
        None => {} // SDK not installed - expected in CI
        Some(map) => assert!(map.is_empty(), "Expected no cameras without hardware"),
    }
}
