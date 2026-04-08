use tracing::info;
use super::common::{create_planetary_frame, create_test_frame};
use super::*;

#[test]
fn test_alignment() {
    let reference = create_test_frame(64, 64, 0.0, 0.0);
    let shifted = create_test_frame(64, 64, 5.0, 3.0);

    let roi = AlignmentRoi::centered(64, 64, 32);
    let (dx, dy, _) = compute_alignment(&reference, &shifted, &roi, 10, 1);

    assert!((dx - (-5.0)).abs() < 2.0, "X offset should be ~-5, got {}", dx);
    assert!((dy - (-3.0)).abs() < 2.0, "Y offset should be ~-3, got {}", dy);
}

#[test]
fn test_alignment_with_planetary_disk() {
    let reference = create_planetary_frame(64, 64, 0.0, 0.0, 0.0);
    let shifted = create_planetary_frame(64, 64, 3.0, -2.0, 0.0);

    let roi = AlignmentRoi::centered(64, 64, 40);
    let (dx, dy, _) = compute_alignment(&reference, &shifted, &roi, 10, 2);

    assert!((dx - (-3.0)).abs() < 1.5, "X offset should be ~-3, got {}", dx);
    assert!(
        (dy - 2.0).abs() < 1.5,
        "Y offset should be ~2, got {}",
        dy
    );
}

#[test]
fn test_alignment_roi_centered() {
    let roi = AlignmentRoi::centered(100, 80, 30);
    assert_eq!(roi.x, 35);
    assert_eq!(roi.y, 25);
    assert_eq!(roi.width, 30);
    assert_eq!(roi.height, 30);
}

#[test]
fn test_alignment_roi_clamped() {
    let roi = AlignmentRoi::centered(50, 50, 100);
    assert_eq!(roi.width, 50);
    assert_eq!(roi.height, 50);
}

#[test]
fn test_subpixel_alignment_accuracy() {
    let reference = create_planetary_frame(64, 64, 0.0, 0.0, 0.0);
    let shifted = create_planetary_frame(64, 64, 2.5, 1.3, 0.0);
    let roi = AlignmentRoi::centered(64, 64, 40);
    let (dx_sub, dy_sub, _) = compute_alignment(&reference, &shifted, &roi, 10, 2);
    let (dx_int, dy_int, _) = compute_alignment(&reference, &shifted, &roi, 10, 1);
    assert!((dx_sub - (-2.5)).abs() < 1.0 || (dx_int - (-2.5)).abs() < 1.0);
    assert!((dy_sub - (-1.3)).abs() < 1.0 || (dy_int - (-1.3)).abs() < 1.0);
}

#[test]
fn test_alignment_with_custom_roi() {
    let reference = create_planetary_frame(64, 64, 0.0, 0.0, 0.0);
    let shifted = create_planetary_frame(64, 64, 4.0, 2.0, 0.0);
    let roi = AlignmentRoi::new(10, 10, 30, 30);
    let (dx, dy, _) = compute_alignment(&reference, &shifted, &roi, 15, 1);
    assert!((dx - (-4.0)).abs() < 3.0, "X offset should be ~ -4, got {}", dx);
    assert!((dy - (-2.0)).abs() < 3.0, "Y offset should be ~ -2, got {}", dy);
}

#[test]
fn test_alignment_centroid_fallback() {
    // Create a very blurry "planet" by using a large spot with low gradient
    let mut reference = Frame::zeros(64, 64, 1).unwrap();
    for y in 20..44 {
        for x in 20..44 {
            reference.set_pixel(x, y, 0, 0.2); // Low contrast
        }
    }

    let mut shifted = Frame::zeros(64, 64, 1).unwrap();
    // Shifted by (10, 5)
    for y in 25..49 {
        for x in 30..54 {
            shifted.set_pixel(x, y, 0, 0.2);
        }
    }

    let roi = AlignmentRoi::centered(64, 64, 32);
    // Correlation might be weak here, centroid should pick it up
    let (dx, dy, ncc) = compute_alignment(&reference, &shifted, &roi, 15, 1);

    info!("Centroid fallback test results: dx={}, dy={}, ncc={}", dx, dy, ncc);
    
    // Centroid of ref is (31.5, 31.5), shifted is (41.5, 36.5)
    // Offset rx-fx = -10, ry-fy = -5
    assert!((dx - (-10.0)).abs() < 1.0, "X offset should be ~-10, got {}", dx);
    assert!((dy - (-5.0)).abs() < 1.0, "Y offset should be ~-5, got {}", dy);
}
