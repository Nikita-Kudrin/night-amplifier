use super::*;

#[test]
fn test_qhy_provider_init() {
    let provider = QhyProvider::new();
    assert_eq!(provider.name(), "QHY");
}

/// A device discovery must not open still lists at its scan position, with the serial its id
/// carries, so an id published from the list resolves against `identities`.
#[test]
fn a_device_described_from_its_id_keeps_its_position_and_serial() {
    let info = camera_info_from_id("QHY268M-1a2b3c4d", 2);
    assert_eq!(info.id, 2);
    assert_eq!(info.name, "QHY268M-1a2b3c4d");
    assert_eq!(info.serial.as_deref(), Some("QHY268M-1a2b3c4d"));
}
