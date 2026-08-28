//! Planetary stacking: resample plus accumulate.
//!
//! `PlanetaryStacker::stack` is the per-stack cost: every selected frame is resampled by
//! its sub-pixel offset (`apply_offset`) and then folded into the accumulator. It had no
//! coverage here, which is how `apply_offset` kept a `Vec` allocation per output pixel —
//! 4.17 million per stacked frame at IMX464 resolution, single-threaded — through the
//! planar migration.
//!
//! `stack` takes `&self`, so the stacker is built once and shared. It used to be
//! rebuilt inside `iter_batched`'s setup, and `add_frame` runs quality scoring plus a
//! search-radius-50 cross-correlation: criterion excludes setup from the *measurement*
//! but still has to run it, so the binary took 78 s wall clock to report two numbers and
//! criterion asked for a 37.8 s target time. Both AGENTS.md files cap a bench binary at
//! ~30 s.

use criterion::{criterion_group, criterion_main, Criterion, SamplingMode};
use night_amplifier::frame::Frame;
use night_amplifier::planetary::{PlanetaryConfig, PlanetaryStacker, QualityMetric};
use std::hint::black_box;
use std::time::Duration;

/// A limb-darkened disc with surface texture, offset by `(shift_x, shift_y)` pixels.
///
/// The shift is what makes this bench measure anything: `apply_offset` short-circuits to
/// a plain clone when the computed offset is under 0.001 px, so a pair of identical
/// frames would skip the resample entirely.
fn planet_frame(width: usize, height: usize, channels: usize, shift_x: f32, shift_y: f32) -> Frame {
    let mut frame = Frame::zeros(width, height, channels).unwrap();
    let cx = width as f32 / 2.0 + shift_x;
    let cy = height as f32 / 2.0 + shift_y;
    let radius = (width.min(height) as f32 / 3.0).max(8.0);

    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let dist = (dx * dx + dy * dy).sqrt();
            let value = if dist < radius {
                let limb = 1.0 - (dist / radius).powi(2) * 0.3;
                (0.6 * limb + (dx * 0.3 + dy * 0.1).sin() * 0.1).clamp(0.0, 1.0)
            } else {
                0.02
            };
            for c in 0..channels {
                frame.set_pixel(x, y, c, value);
            }
        }
    }
    frame
}

/// Stacks per measured iteration. See the note on the mono case below.
///
/// Mono is ~4.3 ms per stack; 25 repeats clears the ~100 ms floor.
const REPS: usize = 25;

fn bench_planetary_stack(c: &mut Criterion) {
    let mut group = c.benchmark_group("planetary_align");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // 1354x768 rather than a full sensor frame: the stacker holds every added frame in
    // memory and this bench has to stay inside the ~30 s CI budget.
    let (w, h) = (1354, 768);

    for (label, channels) in [("resample_mono", 1), ("resample_rgb", 3)] {
        let reference = planet_frame(w, h, channels, 0.0, 0.0);
        let shifted = planet_frame(w, h, channels, 3.5, -2.5);

        // Built once: `add_frame` scores quality and cross-correlates for alignment, and
        // `stack` only reads what it stored. Rebuilding it per iteration measured the
        // same thing and cost ~3.8 s of setup for every 5 ms of it.
        let mut stacker = PlanetaryStacker::new(
            PlanetaryConfig::default().with_quality_metric(QualityMetric::Laplacian),
        );
        stacker.add_frame(&reference).unwrap();
        stacker.add_frame(&shifted).unwrap();

        // `resample_mono` is ~5.5 ms on its own, below the point where the figure is
        // stable run to run; `stack` takes `&self`, so repeating it measures the same
        // work. **The reported `time:` is for `REPS` stacks, not one.**
        group.bench_function(format!("{}_x{}", label, REPS), |b| {
            b.iter(|| {
                for _ in 0..REPS {
                    black_box(stacker.stack().unwrap());
                }
            })
        });
    }

    group.finish();
}

criterion_group!(benches, bench_planetary_stack);
criterion_main!(benches);
