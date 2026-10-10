use crate::frame::Frame;

/// Creates a test frame with a simple Gaussian dot
pub fn create_test_frame(width: usize, height: usize, offset_x: f32, offset_y: f32) -> Frame {
    // `set_pixel` rather than hand-computed offsets: this fixture feeds
    // `frame_to_luminance`, and an interleaved fixture read by a planar consumer
    // (or the reverse) hides the bug instead of catching it.
    let mut frame = Frame::filled(width, height, 3, 0.1).unwrap();

    let cx = width as f32 / 2.0 + offset_x;
    let cy = height as f32 / 2.0 + offset_y;

    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist_sq = dx * dx + dy * dy;
            let intensity = (-dist_sq / 100.0).exp();

            for c in 0..3 {
                let current = frame.get_pixel(x, y, c);
                frame.set_pixel(x, y, c, current + intensity);
            }
        }
    }

    frame
}

/// Creates a simulated planetary disk with surface features
pub fn create_planetary_frame(
    width: usize,
    height: usize,
    offset_x: f32,
    offset_y: f32,
    blur_factor: f32,
) -> Frame {
    let mut frame = Frame::filled(width, height, 3, 0.05).unwrap();

    let cx = width as f32 / 2.0 + offset_x;
    let cy = height as f32 / 2.0 + offset_y;
    let radius = (width.min(height) as f32 / 4.0).max(10.0);

    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < radius {
                let limb_darkening = 1.0 - (dist / radius).powi(2) * 0.3;
                let feature1 = (dx * 0.3 + dy * 0.1).sin() * 0.1;
                let feature2 = (-dist * 0.2).exp() * 0.2;
                let sharpness = 1.0 / (1.0 + blur_factor * 0.1);

                let base_intensity = 0.6 * limb_darkening + feature1 + feature2;
                let intensity = base_intensity * sharpness;

                frame.set_pixel(x, y, 0, (intensity * 1.1).clamp(0.0, 1.0));
                frame.set_pixel(x, y, 1, intensity.clamp(0.0, 1.0));
                frame.set_pixel(x, y, 2, (intensity * 0.9).clamp(0.0, 1.0));
            }
        }
    }

    frame
}
