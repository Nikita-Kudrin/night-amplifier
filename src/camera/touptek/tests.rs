use super::ffi_types::*;
use super::sdk::check_hresult;
use super::shim::parse_fourcc_bayer;
use crate::CfaPattern;

#[test]
fn test_check_hresult_success() {
    assert!(check_hresult(0, "test").is_ok()); // S_OK
    assert!(check_hresult(1, "test").is_ok()); // S_FALSE (also success)
    assert!(check_hresult(42, "test").is_ok()); // any positive
}

#[test]
fn test_check_hresult_failure() {
    let result = check_hresult(-1, "MyFunc");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("MyFunc"));
    assert!(msg.contains("HRESULT"));
}

#[test]
fn test_check_hresult_known_error_codes() {
    // E_FAIL = 0x80004005 as i32 is negative
    let e_fail: i32 = 0x80004005_u32 as i32;
    assert!(check_hresult(e_fail, "test").is_err());

    // E_INVALIDARG = 0x80070057 as i32 is negative
    let e_invalidarg: i32 = 0x80070057_u32 as i32;
    assert!(check_hresult(e_invalidarg, "test").is_err());
}

#[test]
fn test_parse_fourcc_rggb() {
    assert_eq!(parse_fourcc_bayer(FOURCC_RGGB), Some(CfaPattern::Rggb));
}

#[test]
fn test_parse_fourcc_bggr() {
    assert_eq!(parse_fourcc_bayer(FOURCC_BGGR), Some(CfaPattern::Bggr));
}

#[test]
fn test_parse_fourcc_grbg() {
    assert_eq!(parse_fourcc_bayer(FOURCC_GRBG), Some(CfaPattern::Grbg));
}

#[test]
fn test_parse_fourcc_gbrg() {
    assert_eq!(parse_fourcc_bayer(FOURCC_GBRG), Some(CfaPattern::Gbrg));
}

#[test]
fn test_parse_fourcc_mono() {
    assert_eq!(parse_fourcc_bayer(FOURCC_YYYY), None);
}

#[test]
fn test_parse_fourcc_unknown() {
    assert_eq!(parse_fourcc_bayer(0xDEADBEEF), None);
}

#[test]
fn test_make_fourcc_values() {
    // Verify our const fn produces the correct byte ordering
    assert_eq!(FOURCC_RGGB, u32::from_le_bytes([b'R', b'G', b'G', b'B']));
    assert_eq!(FOURCC_BGGR, u32::from_le_bytes([b'B', b'G', b'G', b'R']));
    assert_eq!(FOURCC_GRBG, u32::from_le_bytes([b'G', b'R', b'B', b'G']));
    assert_eq!(FOURCC_GBRG, u32::from_le_bytes([b'G', b'B', b'R', b'G']));
    assert_eq!(FOURCC_YYYY, u32::from_le_bytes([b'Y', b'Y', b'Y', b'Y']));
}

#[test]
fn test_flag_constants() {
    // Verify key flag values match the SDK header
    assert_eq!(TOUPCAM_FLAG_MONO, 0x10);
    assert_eq!(TOUPCAM_FLAG_TEC, 0x80);
    assert_eq!(TOUPCAM_FLAG_TEC_ONOFF, 0x20000);
    assert_eq!(TOUPCAM_FLAG_USB30, 0x40);
    assert_eq!(TOUPCAM_FLAG_RAW16, 0x8000);
    assert_eq!(TOUPCAM_FLAG_RAW8, 0x80000000);
}

#[test]
fn test_event_constants() {
    assert_eq!(TOUPCAM_EVENT_IMAGE, 0x0004);
    assert_eq!(TOUPCAM_EVENT_ERROR, 0x0080);
    assert_eq!(TOUPCAM_EVENT_DISCONNECTED, 0x0081);
}

#[test]
fn test_option_constants() {
    assert_eq!(TOUPCAM_OPTION_RAW, 0x04);
    assert_eq!(TOUPCAM_OPTION_BINNING, 0x17);
    assert_eq!(TOUPCAM_OPTION_BITDEPTH, 0x06);
}

#[test]
fn test_toupcam_max() {
    assert_eq!(TOUPCAM_MAX, 128);
}

#[test]
fn test_provider_not_available_without_sdk() {
    use super::TouptekProvider;
    use crate::camera::traits::CameraProvider;

    let provider = TouptekProvider::new();
    // Without the ToupTek shared library installed, provider should not be available
    // but should not crash
    let _available = provider.is_available();
    assert_eq!(provider.name(), "ToupTek");
}
