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

/// A deterministic stream in `[0, 1)` (64-bit LCG, top 24 bits).
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407))
    }

    pub fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Lunar-like texture: Gaussian "craters" 2-25 px across, bright and dark, over a grey
/// floor, with the whole field moved by `shift` (exactly, since each blob is evaluated at
/// its shifted centre). One seed is one surface.
pub fn textured_frame(width: usize, height: usize, shift: (f32, f32), seed: u64) -> Frame {
    let mut plane = vec![0.4f32; width * height];
    let mut rng = Lcg::new(seed);
    let margin = 80.0;
    let blobs = width * height / 1600;
    for _ in 0..blobs {
        let cx = rng.next() * (width as f32 + 2.0 * margin) - margin + shift.0;
        let cy = rng.next() * (height as f32 + 2.0 * margin) - margin + shift.1;
        let sigma = 2.0 + rng.next() * 23.0;
        let amplitude = (rng.next() - 0.5) * 0.4;
        let reach = (4.0 * sigma) as i64;
        let (x0, x1) = ((cx as i64 - reach).max(0), (cx as i64 + reach).min(width as i64 - 1));
        let (y0, y1) = ((cy as i64 - reach).max(0), (cy as i64 + reach).min(height as i64 - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                plane[y as usize * width + x as usize] += amplitude * (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
            }
        }
    }
    Frame::from_f32_vec(plane, width, height, 1).unwrap()
}

/// A 40 px disc with banding and a soft limb on a dark sky, centred and moved by `shift`.
pub fn small_planet_frame(width: usize, height: usize, shift: (f32, f32)) -> Frame {
    let mut frame = Frame::filled(width, height, 1, 0.02).unwrap();
    let (cx, cy) = (width as f32 / 2.0 + shift.0, height as f32 / 2.0 + shift.1);
    let radius = 20.0f32;
    for y in (cy - 24.0) as usize..=(cy + 24.0) as usize {
        for x in (cx - 24.0) as usize..=(cx + 24.0) as usize {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let dist = (dx * dx + dy * dy).sqrt();
            let edge = ((radius - dist) / 1.5 + 0.5).clamp(0.0, 1.0);
            let bands = 0.15 * (dy * 0.6).sin() + 0.05 * (dx * 0.25 + dy * 0.1).cos();
            let surface = 0.6 * (1.0 - 0.3 * (dist / radius).powi(2)) + bands;
            frame.set_pixel(x, y, 0, 0.02 + edge * (surface - 0.02));
        }
    }
    frame
}

/// `frame` plus Gaussian noise of `sigma` (Box-Muller over [`Lcg`]).
pub fn with_noise(frame: &Frame, sigma: f32, seed: u64) -> Frame {
    let mut rng = Lcg::new(seed);
    let mut noisy = frame.clone();
    for value in noisy.data_mut() {
        let (u1, u2) = (rng.next().max(1e-7), rng.next());
        *value += sigma * (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
    }
    noisy
}
