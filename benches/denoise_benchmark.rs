//! Benchmarks for the two spatial denoisers, at the resolutions they actually run at.
//!
//! Run with: cargo bench --bench denoise_benchmark -- --noplot
//!
//! # Why these sizes and no sensor-sized case
//!
//! The whole architectural point of this stage is that it runs on the *streamed*
//! image, after the encoder's box downsample, not on the sensor frame. 1440² is the
//! eyepiece screen and 1920x1080 the Hd1080 tier. A 3008² case would measure a
//! configuration the code deliberately cannot reach — per the sizing rules, an input
//! size production never produces does not earn a row. A binoview eye (720²) is left
//! out for the opposite reason: it re-measures the 1440² kernel at a quarter the area.
//!
//! Both filters mutate the buffer in place, so the "repeat the kernel `reps` times"
//! trick is unavailable: iteration two would denoise iteration one's output, which is
//! a different and progressively easier workload. Each setup builds a `Vec` of `reps`
//! clones, as `cfa_benchmark` and `render_benchmark` do.
//!
//! **The reported `time:` covers `reps` invocations.** Per-call figures the current
//! `reps` were sized from (20-core x86 dev box, not a Pi 5):
//!
//! | Stage | 1440² | 1920x1080 |
//! |---|---|---|
//! | chroma guided filter | 6.7 ms | 6.9 ms |
//! | luma à trous wavelets, dial at or below the middle | 10.6 ms | 10.6 ms |
//! | luma à trous wavelets, dial at the top (levels 5-6) | 13.8 ms | 14.0 ms |
//! | both | 14.3 ms | 14.4 ms |
//!
//! The `both` row is well under the sum: the YCbCr split and merge are shared, and are
//! roughly 3.7 ms of the 1440² figure on their own.
//!
//! **Compare one group at a time** (`cargo bench --bench denoise_benchmark -- --noplot
//! 'denoise_both/1440'`) when measuring a change here, and never an A/B where one side
//! ran alone and the other in the whole binary. A group's figures depend on what ran
//! before it in the same process: measured, `denoise_both` reads 14.3 ms a call on its
//! own and 27.2 ms with the chroma group ahead of it, and `denoise_luma_wavelet` 10.6 ms
//! against 23.5. The chroma group is the one that poisons the rest — dropping it from the
//! filter is enough to restore the isolated figures — and how badly is not the same for
//! two builds, so a mixed A/B reversed the sign of a 20 % improvement before this note
//! existed. Resetting the buffers outside the timer instead of cloning them per iteration
//! was tried and changes none of it, so the cause is not the 274 MB of churn it removes.
//!
//! A single group is ~6 s; the whole binary is ~2 minutes for the same reason.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, SamplingMode, Throughput};
use night_amplifier::render::denoise::{
    denoise_rgb_interleaved, ChromaDenoiseConfig, DenoiseConfig, LumaDenoiseConfig,
};
use std::hint::black_box;
use std::time::Duration;

/// `(width, height, reps)` per group. `reps` is per *group*, not global: the chroma
/// filter is roughly half the cost of the wavelet, so one shared figure would leave it
/// under the 100 ms floor while padding the wavelet's live footprint. A 1440²
/// interleaved RGB f32 buffer is 24.9 MB, which is what caps these.
const CHROMA_CASES: [(usize, usize, usize); 2] = [(1440, 1440, 16), (1920, 1080, 16)];
const LUMA_CASES: [(usize, usize, usize); 2] = [(1440, 1440, 11), (1920, 1080, 11)];
const COARSE_CASES: [(usize, usize, usize); 2] = [(1440, 1440, 8), (1920, 1080, 8)];
const BOTH_CASES: [(usize, usize, usize); 2] = [(1440, 1440, 8), (1920, 1080, 8)];

/// Interleaved RGB f32 at stream resolution, with sky-level noise, a gradient and a few
/// stars — the state of a frame between the encoder's resample and its tone curve, which
/// is exactly where this stage sits.
fn staged_buffer(width: usize, height: usize) -> Vec<f32> {
    let mut data = vec![0.0f32; width * height * 3];
    let mut seed = 0x9E37_79B9u32;
    for y in 0..height {
        for x in 0..width {
            let gradient = 0.0005 * (x as f32 / width as f32 + y as f32 / height as f32);
            for c in 0..3 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                data[(y * width + x) * 3 + c] = 0.003 + gradient + noise * 0.004;
            }
        }
    }
    for i in (0..width * height).step_by(4099) {
        for c in 0..3 {
            data[i * 3 + c] = 0.6;
        }
    }
    data
}

fn bench_case(
    c: &mut Criterion,
    name: &str,
    cases: [(usize, usize, usize); 2],
    config: DenoiseConfig,
) {
    let mut group = c.benchmark_group(name);
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_millis(1500));

    for (width, height, reps) in cases {
        let buffer = staged_buffer(width, height);
        group.throughput(Throughput::Elements((reps * width * height) as u64));
        group.bench_function(format!("{width}x{height}_x{reps}"), |b| {
            b.iter_batched_ref(
                || vec![buffer.clone(); reps],
                |buffers| {
                    for buf in buffers.iter_mut() {
                        denoise_rgb_interleaved(black_box(buf), width, height, &config);
                    }
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench_chroma(c: &mut Criterion) {
    bench_case(
        c,
        "denoise_chroma_guided",
        CHROMA_CASES,
        DenoiseConfig {
            luma: LumaDenoiseConfig::OFF,
            chroma: ChromaDenoiseConfig::default(),
        },
    );
}

fn bench_luma(c: &mut Criterion) {
    bench_case(
        c,
        "denoise_luma_wavelet",
        LUMA_CASES,
        DenoiseConfig {
            luma: LumaDenoiseConfig::default(),
            chroma: ChromaDenoiseConfig::OFF,
        },
    );
}

/// The **upper half of the Background Grain dial**, which `bench_luma` cannot see.
///
/// `LumaDenoiseConfig::default()` leaves the coarse thresholds at zero, and
/// `denoise_luma_with` then breaks out of the transform after level 4 — so the default
/// case never runs levels 5-6, never measures their noise and never reads the mask off
/// their smoothed planes. That is the half the manual tells an observer to start at, and
/// at 1440² it is the difference between 10.5 ms and 12.8 ms a frame.
fn bench_luma_coarse(c: &mut Criterion) {
    bench_case(
        c,
        "denoise_luma_wavelet_coarse",
        COARSE_CASES,
        DenoiseConfig {
            luma: LumaDenoiseConfig {
                k: LumaDenoiseConfig::thresholds_for(0.0, 1.0),
                ..LumaDenoiseConfig::default()
            },
            chroma: ChromaDenoiseConfig::OFF,
        },
    );
}

fn bench_both(c: &mut Criterion) {
    bench_case(
        c,
        "denoise_both",
        BOTH_CASES,
        DenoiseConfig {
            luma: LumaDenoiseConfig::default(),
            chroma: ChromaDenoiseConfig::default(),
        },
    );
}

criterion_group!(benches, bench_chroma, bench_luma, bench_luma_coarse, bench_both);
criterion_main!(benches);
