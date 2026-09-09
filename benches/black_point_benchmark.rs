//! Sky-level estimation, the input the black point is derived from.
//!
//! Separate binary rather than a fifth group in `render_benchmark`, which already sits
//! at ~28 s of the ~30 s budget — the same reason `scale_lut_benchmark` was split out.
//!
//! `estimate_background_mode` costs what it costs almost regardless of frame size: it
//! strides the frame for a fixed ~50 000 luminance samples, then bins them twice — once
//! coarsely to pick the sky's region, once finely to locate the peak inside it.
//!
//! The two cases differ in how tightly the sky clusters, which is what the refinement is
//! sensitive to. A deep stack (sigma ~2e-5 against a 2.4e-4 coarse bin) puts nearly every
//! sample inside the refinement window, so its window test predicts perfectly; a single
//! sub spreads over ~5 bins and roughly half the samples fall outside, which costs branch
//! mispredictions — measured 0.51 ms against 0.61 ms per call. Keep both: the deeper case
//! is the one production spends its time in, and the shallower one is where a future
//! change to the window test would show up first.
//!
//! Guards a real regression. Refining by sorting the window and taking its half-sample
//! mode — the first fix for the black point snapping a whole coarse bin as a stack
//! deepened — measured 1.40 ms per call against 0.39 ms for the unrefined original. The
//! sub-histogram that replaced it keeps the resolution for 0.51 ms.
//!
//! Production shape throughout: 3008x3008x3 is the IMX533 this was measured on, and it
//! matters beyond pixel count — at that size the sampling stride is 181, so the 50 000
//! reads are scattered across a 108 MB frame and miss cache on nearly every one.

use criterion::{criterion_group, criterion_main, Criterion, SamplingMode};
use night_amplifier::frame::Frame;
use night_amplifier::render::estimate_background_mode;
use std::hint::black_box;
use std::time::Duration;

const WIDTH: usize = 3008;
const HEIGHT: usize = 3008;

/// Sky level a background-subtracted deep-sky stack actually lands on, in normalised
/// units: ~151 of 65535 ADU, measured off the M27 session this bench was written for.
const SKY: f32 = 0.0023;

/// Per-pixel sky sigma at ~70 frames (2.0e-5 = 1.3 ADU) and at one frame (5.0e-5 =
/// 3.3 ADU). Both measured on that session; the first is the case that stresses the
/// refinement hardest.
const SIGMA_DEEP: f32 = 2.0e-5;
const SIGMA_SINGLE: f32 = 5.0e-5;

/// Standard normal deviates, drawn once and reused.
///
/// Generating 27 million sums-of-twelve-uniforms per frame put this binary at 90 s
/// wall clock against a ~30 s budget, nearly all of it setup that never reaches the
/// reported figure. A table walked at a coprime stride gives every pixel an
/// uncorrelated draw for one multiply and one mask, and the histogram cannot tell the
/// difference — it only ever sees ~50 000 of these values.
const NOISE_LEN: usize = 1 << 20;
const NOISE_STRIDE: usize = 7919;

fn noise_table() -> Vec<f32> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut unit = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 16_777_216.0
    };
    (0..NOISE_LEN)
        .map(|_| (0..12).map(|_| unit()).sum::<f32>() - 6.0)
        .collect()
}

/// Deterministic sky: Gaussian-ish noise on a pedestal, plus a star field so the
/// histogram carries the bright tail the peak search has to walk past.
fn sky_frame(noise: &[f32], sigma: f32) -> Frame {
    let mut data = vec![0.0f32; WIDTH * HEIGHT * 3];
    let mut idx = 0usize;
    for v in data.iter_mut() {
        *v = (SKY + noise[idx] * sigma).max(0.0);
        idx = (idx + NOISE_STRIDE) & (NOISE_LEN - 1);
    }

    let mut frame = Frame::from_f32_vec(data, WIDTH, HEIGHT, 3).unwrap();
    // ~4000 stars, the order this field detects, spread over the upper range so the
    // histogram's search limit and the peak search both do real work.
    let mut star_state = 0x9E37_79B9_7F4A_7C15u64;
    for i in 0..4000 {
        star_state ^= star_state >> 12;
        star_state ^= star_state << 25;
        star_state ^= star_state >> 27;
        let x = (star_state as usize >> 8) % (WIDTH - 4) + 2;
        let y = (star_state as usize >> 32) % (HEIGHT - 4) + 2;
        let peak = 0.02 + (i % 50) as f32 * 0.019;
        for dy in 0..3 {
            for dx in 0..3 {
                let falloff = if dx == 1 && dy == 1 { 1.0 } else { 0.35 };
                for c in 0..3 {
                    frame.set_pixel(x + dx - 1, y + dy - 1, c, (peak * falloff).min(1.0));
                }
            }
        }
    }
    frame
}

fn bench_background_mode(c: &mut Criterion) {
    let noise = noise_table();
    let deep = sky_frame(&noise, SIGMA_DEEP);
    let single = sky_frame(&noise, SIGMA_SINGLE);

    let mut group = c.benchmark_group("background_mode");
    // Criterion's default linear scheme wants 1+2+..+10 = 55 iterations per case, which
    // does not fit the budget once a case clears 100 ms.
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_millis(1500));

    // One call is ~1.3 ms, so `time:` needs ~80 repeats to clear the ~100 ms floor;
    // below that criterion overhead and thermal throttling move the figure more than a
    // real regression would. 96 leaves the cheaper case clear of it too.
    const REPS: usize = 96;

    group.bench_function(format!("deep_stack_x{REPS}"), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                black_box(estimate_background_mode(black_box(&deep)));
            }
        })
    });

    group.bench_function(format!("single_sub_x{REPS}"), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                black_box(estimate_background_mode(black_box(&single)));
            }
        })
    });

    group.finish();
}

criterion_group!(benches, bench_background_mode);
criterion_main!(benches);
