use super::*;

fn neptune() -> DeviceIdentity {
    DeviceIdentity::new("Neptune-C II", Some("NEP123".to_string()))
}

fn ares() -> DeviceIdentity {
    DeviceIdentity::new("Ares-C PRO", Some("ARE456".to_string()))
}

#[test]
fn serial_ids_round_trip() {
    let id = camera_id("PlayerOne", 3, Some("NEP123"));
    assert_eq!(id, "playerone_sn-NEP123");
    assert_eq!(
        parse_camera_id(&id),
        Ok(("playerone", CameraLocator::Serial("NEP123".to_string())))
    );
}

/// Anything outside `[A-Za-z0-9-]` would have to survive a URL path, so it is encoded.
#[test]
fn unsafe_serials_are_hex_encoded_and_round_trip() {
    let serial = "QHY268C/ab_12 x";
    let id = camera_id("qhy", 0, Some(serial));
    assert!(
        id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "id must be URL-path safe: {id}"
    );
    assert_eq!(
        parse_camera_id(&id),
        Ok(("qhy", CameraLocator::Serial(serial.to_string())))
    );
}

#[test]
fn index_ids_are_unchanged_and_still_parse() {
    assert_eq!(camera_id("ZWO", 1, None), "zwo_1");
    assert_eq!(
        parse_camera_id("playerone_0"),
        Ok(("playerone", CameraLocator::Index(0)))
    );
}

#[test]
fn malformed_ids_are_rejected() {
    assert_eq!(parse_camera_id("invalidformat"), Err(CameraIdError::Format));
    assert_eq!(parse_camera_id("_0"), Err(CameraIdError::Format));
    assert_eq!(
        parse_camera_id("provider_notanumber"),
        Err(CameraIdError::Locator)
    );
    assert_eq!(parse_camera_id("provider_sn-"), Err(CameraIdError::Locator));
    assert_eq!(parse_camera_id("provider_snx-4"), Err(CameraIdError::Locator));
    assert_eq!(parse_camera_id("provider_snx-zz"), Err(CameraIdError::Locator));
}

#[test]
fn placeholder_serials_are_not_identities() {
    assert_eq!(normalize_serial(""), None);
    assert_eq!(normalize_serial("\0\0\0"), None);
    assert_eq!(normalize_serial("0000000000"), None);
    assert_eq!(normalize_serial("  NEP123\0\0"), Some("NEP123".to_string()));
}

#[test]
fn a_serial_is_found_wherever_the_device_is_listed_now() {
    let before = [neptune(), ares()];
    let after = [ares(), neptune()];
    let locator = CameraLocator::Serial("NEP123".to_string());
    assert_eq!(resolve_index(&before, &locator), Some(0));
    assert_eq!(resolve_index(&after, &locator), Some(1));
}

/// A missing device is waited for. Falling back to its old index is exactly how the
/// imaging camera got installed as the guide camera.
#[test]
fn a_missing_serial_resolves_to_nothing_rather_than_an_index() {
    let listed = [ares()];
    assert_eq!(
        resolve_index(&listed, &CameraLocator::Serial("NEP123".to_string())),
        None
    );
    assert_eq!(resolve_index(&listed, &CameraLocator::Index(1)), None);
}

#[test]
fn identities_match_on_serial_when_both_have_one_and_on_name_otherwise() {
    let body_a = DeviceIdentity::new("ASI120MM", Some("A".to_string()));
    let body_b = DeviceIdentity::new("ASI120MM", Some("B".to_string()));
    let unknown = DeviceIdentity::new("ASI120MM", None);
    assert!(!body_a.matches(&body_b), "same model, different bodies");
    assert!(body_a.matches(&unknown));
    assert!(!neptune().matches(&DeviceIdentity::new("Ares-C PRO", None)));
}

/// The 2026-09-07 incident: the guide camera was last seen at index 0, and after the
/// dropout index 0 is the imaging camera.
#[test]
fn recovery_never_offers_the_other_camera_after_a_reorder() {
    let listed = [ares(), neptune()];
    assert_eq!(
        recovery_candidates(&listed, &neptune(), 0, Some(&ares())),
        vec![1]
    );
}

#[test]
fn recovery_offers_nothing_while_the_serial_is_missing() {
    let listed = [ares()];
    assert!(recovery_candidates(&listed, &neptune(), 0, Some(&ares())).is_empty());
}

/// Without serials only the model name is left: every body of that model is a
/// candidate, the last-seen index first, and a different model never is.
#[test]
fn without_serials_same_model_candidates_are_tried_last_seen_first() {
    let listed = [
        DeviceIdentity::new("ASI120MM", None),
        DeviceIdentity::new("ASI533MC", None),
        DeviceIdentity::new("ASI120MM", None),
    ];
    let expected = DeviceIdentity::new("ASI120MM", None);
    assert_eq!(recovery_candidates(&listed, &expected, 2, None), vec![2, 0]);
}

#[test]
fn a_serial_known_to_belong_to_the_other_role_is_never_a_candidate() {
    let guide = DeviceIdentity::new("ASI120MM", Some("GUIDE".to_string()));
    let main = DeviceIdentity::new("ASI120MM", Some("MAIN".to_string()));
    // The expected camera has no recorded serial, so candidates come from the name.
    let expected = DeviceIdentity::new("ASI120MM", None);
    let listed = [main.clone(), guide.clone()];
    assert_eq!(
        recovery_candidates(&listed, &expected, 0, Some(&main)),
        vec![1]
    );
}

/// Two bodies of one model and no serial: the SDK device id is all that tells the
/// guide camera's still-plugged device apart from the one being recovered.
#[test]
fn a_device_id_held_by_the_other_role_is_never_a_candidate() {
    let listed = [
        DeviceIdentity::new("ASI120MM", None).with_device_id(4),
        DeviceIdentity::new("ASI120MM", None).with_device_id(7),
    ];
    let expected = DeviceIdentity::new("ASI120MM", None);
    let other = DeviceIdentity::new("ASI120MM", None).with_device_id(4);
    assert_eq!(recovery_candidates(&listed, &expected, 0, Some(&other)), vec![1]);
}

#[test]
fn the_same_device_is_decided_by_serial_then_device_id_and_never_by_name_alone() {
    let body = |serial: Option<&str>, device_id: Option<i32>| {
        let identity = DeviceIdentity::new("ASI294MC", serial.map(str::to_string));
        match device_id {
            Some(id) => identity.with_device_id(id),
            None => identity,
        }
    };
    assert_eq!(body(Some("A"), Some(1)).is_same_device(&body(Some("A"), Some(2))), Some(true));
    assert_eq!(body(Some("A"), Some(1)).is_same_device(&body(Some("B"), Some(1))), Some(false));
    assert_eq!(body(None, Some(4)).is_same_device(&body(Some("A"), Some(4))), Some(true));
    assert_eq!(body(None, Some(4)).is_same_device(&body(None, Some(7))), Some(false));
    assert_eq!(
        body(None, None).is_same_device(&body(None, Some(7))),
        None,
        "two bodies of one model are indistinguishable by name"
    );
}
