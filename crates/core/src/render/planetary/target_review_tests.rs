//! Review of 4a3778e: edge cases of the automatic Moon measure (`super::target`).

use super::{DaylightBalance, NotTheMoon, PlanetaryLevels, TargetMeasure, MOON_COLOUR};
use crate::frame::Frame;

/// The raw IMX533 Moon of 2026-10-24.
const RAW_MOON: [f32; 3] = [0.944, 1.0, 0.487];
/// The balance that renders `RAW_MOON` in `MOON_COLOUR`: this camera's true balance.
const TRUE: DaylightBalance = DaylightBalance { red: 1.10 / 0.944, green: 1.0, blue: 0.92 / 0.487 };

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32 - 0.5
    }
}

/// Maria: darker patches scaled to the disc.
fn maria(x: f32, y: f32, radius: f32) -> f32 {
    let (u, v) = (x / radius * 6.0, y / radius * 6.0);
    if u.sin() * (v * 1.3).cos() > 0.35 { 0.72 } else { 1.0 }
}

/// A sphere of `radius` on a `(w, h)` frame; `surface(nx, ny, nz, c)` is channel `c`'s
/// brightness at the unit normal, `None` where unlit. Each channel times `raw`, plus `pedestal`
/// and `noise`.
fn sphere(
    (w, h): (usize, usize),
    radius: f32,
    surface: impl Fn(f32, f32, f32, usize) -> Option<f32>,
    raw: [f32; 3],
    (pedestal, noise): (f32, f32),
) -> Frame {
    let mut frame = Frame::zeros(w, h, 3).unwrap();
    let mut rng = Lcg(11);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = ((x as f32 - cx) / radius, (y as f32 - cy) / radius);
            let rr = nx * nx + ny * ny;
            for (c, t) in raw.iter().enumerate() {
                let level = if rr < 1.0 { surface(nx, ny, (1.0 - rr).sqrt(), c).unwrap_or(0.0) } else { 0.0 };
                frame.set_pixel(x, y, c, (level * t + pedestal + noise * rng.next()).clamp(0.0, 1.0));
            }
        }
    }
    frame
}

/// The Moon at `phase` (radians, 0 full) under Lommel-Seeliger, `peak` at the subsolar point.
fn moon_at(phase: f32, peak: f32, radius: f32) -> impl Fn(f32, f32, f32, usize) -> Option<f32> {
    move |nx, ny, nz, _| {
        let cos_i = nx * phase.sin() + nz * phase.cos();
        (cos_i > 0.0).then(|| 2.0 * peak * cos_i / (cos_i + nz) * maria(nx * radius, ny * radius, radius))
    }
}

fn verdict(frame: &Frame, balance: &DaylightBalance) -> (TargetMeasure, Result<DaylightBalance, NotTheMoon>) {
    let mut shown = frame.clone();
    balance.apply(&mut shown);
    let m = TargetMeasure::of(&shown, &PlanetaryLevels::solve(&shown));
    (m, m.moon(balance))
}

fn report(name: &str, (m, v): &(TargetMeasure, Result<DaylightBalance, NotTheMoon>)) {
    println!(
        "{name:34} diam {:5.0} texture {:+.4} limb {:>5} haze {:.4} -> {v:?}",
        m.diameter,
        m.texture_excess(),
        m.limb.map_or("-".into(), |l| format!("{l:.2}")),
        m.haze
    );
}

/// H1: the Moon away from full (a quarter, a crescent) is still read as the Moon.
#[test]
fn h1_phases() {
    for (name, degrees) in [("full", 0.0f32), ("gibbous 45", 45.0), ("quarter 90", 90.0), ("crescent 120", 120.0), ("crescent 140", 140.0)] {
        let frame = sphere((900, 800), 300.0, moon_at(degrees.to_radians(), 0.45, 300.0), RAW_MOON, (0.004, 0.004));
        report(name, &verdict(&frame, &DaylightBalance::PRIOR));
    }
}

/// H2: a white-light Sun (linear limb darkening u = 0.5/0.6/0.75 in R/G/B) is refused.
#[test]
fn h2_white_light_sun() {
    let sunlight = [1.0 / 1.10, 1.0, 1.0 / 1.95];
    for (name, spots) in [("quiet Sun", false), ("Sun with spots", true)] {
        let frame = sphere(
            (900, 800),
            350.0,
            |nx, ny, nz, c| {
                let u = [0.5, 0.6, 0.75][c];
                let spot = spots && ((nx - 0.3).hypot(ny + 0.1) < 0.05 || (nx + 0.2).hypot(ny - 0.25) < 0.04);
                Some(0.6 * (1.0 - u * (1.0 - nz)) * if spot { 0.3 } else { 1.0 })
            },
            sunlight,
            (0.004, 0.004),
        );
        report(name, &verdict(&frame, &DaylightBalance::PRIOR));
    }
}

/// H4: a camera delivering balanced colour (SharpCap TIFF, an INDI RGB feed) starts at the
/// prior and must reach its own balance off the Moon.
#[test]
fn h4_pre_balanced_camera_never_calibrates_from_the_prior() {
    let frame = sphere((900, 800), 300.0, moon_at(0.0, 0.45, 300.0), MOON_COLOUR, (0.004, 0.004));
    let mut balance = DaylightBalance::PRIOR;
    for step in 0..5 {
        let result = verdict(&frame, &balance);
        report(&format!("pre-balanced, step {step}"), &result);
        match result.1 {
            Ok(next) => balance = next,
            Err(_) => break,
        }
    }
    println!("ends at {balance:?}");
}

/// H5: a large Mars (weak limb darkening, albedo marks) seen through a balance whose blue is
/// 16 % strong (a Moon calibrated at ~20 deg altitude) is refused; if not, the Moon after it is.
#[test]
fn h5_mars_latches_the_balance() {
    let mars_raw = [1.37, 1.0, 0.325];
    let mars = sphere(
        (900, 800),
        200.0,
        |nx, ny, nz, _| Some(0.35 * nz.powf(0.4) * maria(nx * 200.0 + 37.0, ny * 200.0, 200.0)),
        mars_raw,
        (0.004, 0.004),
    );
    for strong in [1.0f32, 1.10, 1.16, 1.22] {
        let in_force = DaylightBalance { blue: TRUE.blue * strong, ..TRUE }.sanitized();
        let result = verdict(&mars, &in_force);
        report(&format!("Mars, blue {strong:.2}x true"), &result);
        if let Ok(mars_balance) = result.1 {
            let moon = sphere((900, 800), 300.0, moon_at(0.0, 0.45, 300.0), RAW_MOON, (0.004, 0.004));
            report("  then the Moon through it", &verdict(&moon, &mars_balance));
        }
    }
}
