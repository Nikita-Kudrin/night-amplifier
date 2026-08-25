//! Planetary alignment resampling.
//!
//! `apply_offset` is the per-frame cost of planetary stacking: every selected frame is
//! resampled by its sub-pixel offset before it enters the accumulator. It had no
//! coverage here, which is how it kept a `Vec` allocation per output pixel — 4.17
//! million per stacked frame at IMX464 resolution, single-threaded — through the planar
//! migration.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use night_amplifier::frame::Frame;
use night_amplifier::planetary::{PlanetaryConfig, PlanetaryStacker, QualityMetric};
use std::hint::black_box;
use std::time::Duration;

/// A limb-darkened disc with surface texture, offset by `(shift_x, shift_y)` pixels.
///
/// The shift is what makes this bench measure anything: `apply_offset` short-circuits to
/// a plain clone when the computed offset is under 0.001 px, so a pair of identical
/// frames would skip the resample entirely.
fn planet_frame(
    width: usize,
    height: usize,
    channels: usize,
    shift_x: f32,
    shift_y: f32,
) -> Frame {
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

fn bench_apply_offset(c: &mut Criterion) {
    let mut group = c.benchmark_group("planetary_align");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));

    // 1354x768 rather than a full sensor frame: the stacker holds every added frame in
    // memory and this bench has to stay inside the ~30 s CI budget.
    let (w, h) = (1354, 768);

    for (label, channels) in [("resample_mono", 1), ("resample_rgb", 3)] {
        let reference = planet_frame(w, h, channels, 0.0, 0.0);
        let shifted = planet_frame(w, h, channels, 3.5, -2.5);

        // Setup builds the stack (reference + one offset frame, so alignment is computed
        // once outside the measurement); the routine is the resample-and-accumulate.
        group.bench_function(label, |b| {
            b.iter_batched(
                || {
                    let mut stacker = PlanetaryStacker::new(
                        PlanetaryConfig::default().with_quality_metric(QualityMetric::Laplacian),
                    );
                    stacker.add_frame(&reference).unwrap();
                    stacker.add_frame(&shifted).unwrap();
                    stacker
                },
                |stacker| black_box(stacker.stack().unwrap()),
                BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

criterion_group!(benches, bench_apply_offset);
criterion_main!(benches);
