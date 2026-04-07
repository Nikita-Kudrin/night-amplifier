use super::*;
use crate::CfaPattern;

#[test]
fn test_provider_available() {
    let provider = ZwoProvider::new();
    assert!(provider.is_available());
    assert_eq!(provider.name(), "ZWO");
}

#[test]
fn test_parse_props_display_color() {
    let props_str = r#"Camera ASI294MC Pro
	ID: 0 UUID:
	Detector: 4144 x 2822
	Color: true, Shutter: false, Cooler: true, USB3: true, Trigger: false
	Bayer Pattern: Some(BayerRG)
	Bins: [1, 2, 3, 4]
	Pixel Size: 4.63 um, e/ADU: 0.37, Bit Depth: 14
            "#;

    let parsed = parse_props_display(props_str);

    assert!(parsed.is_color);
    assert_eq!(parsed.bayer_pattern, Some(CfaPattern::Rggb));
    assert!(parsed.has_cooler);
    assert!(!parsed.has_shutter);
    assert!(parsed.is_usb3);
    assert_eq!(parsed.bit_depth, 14);
    assert_eq!(parsed.supported_bins, vec![1, 2, 3, 4]);
}

#[test]
fn test_parse_props_display_mono() {
    let props_str = r#"Camera ASI183MM
	ID: 1 UUID:
	Detector: 5496 x 3672
	Color: false, Shutter: false, Cooler: false, USB3: true, Trigger: false
	Bayer Pattern: None
	Bins: [1, 2]
	Pixel Size: 2.4 um, e/ADU: 0.3, Bit Depth: 12
            "#;

    let parsed = parse_props_display(props_str);

    assert!(!parsed.is_color);
    assert!(parsed.bayer_pattern.is_none());
    assert!(!parsed.has_cooler);
    assert_eq!(parsed.bit_depth, 12);
    assert_eq!(parsed.supported_bins, vec![1, 2]);
}
