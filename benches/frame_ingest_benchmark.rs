//! Benchmark for `Frame::from_raw` — the camera ingest path.
//!
//! Run with: cargo bench --bench frame_ingest_benchmark
//!
//! Every frame a camera hands over goes through `from_raw` before anything else in the
//! pipeline touches it, so its cost is paid once per frame, unconditionally. It has no
//! other benchmark today.
//!
//! `from_raw` dispatches on `PixelFormat` to one of two decode arms inside
//! `scatter_to_planes` (see `src/frame/factory.rs`): a single-channel arm for raw Bayer
//! captures, and a multi-channel arm for already-interleaved sources. Each case below
//! exercises one arm; the `*Be` and `Bayer8` formats were left out because they run the
//! same arm as `Bayer16`/`Rgb8` and differ only in the per-sample decode closure — a case
//! that would only re-measure a sibling's kernel, per the community `AGENTS.md`.
//!
//! `from_raw` is pure (`&[u8]` in, fresh `Frame` out), so each case repeats it `REPS`
//! times inside `b.iter` and declares `Throughput::Elements(REPS * pixels)`.
//! **The reported `time:` is for `REPS` invocations, not one.**
//!
//! Both cases are IMX464 resolution (2712x1538) — the sensor the rest of the suite is
//! sized against.

use criterion::{criterion_group, criterion_main, Criterion, SamplingMode, Throughput};
use night_amplifier::{Frame, PixelFormat};
use std::hint::black_box;
use std::time::Duration;

const WIDTH: usize = 2712;
const HEIGHT: usize = 1538;

/// A raw 16-bit Bayer buffer, one channel, little-endian — what a cooled mono/OSC camera
/// hands to `from_raw` before debayering. Filled with a non-constant pattern so the
/// decode isn't a memset in disguise.
fn generate_bayer16_raw(width: usize, height: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 2];
    for (i, chunk) in data.chunks_exact_mut(2).enumerate() {
        let v = ((i * 37) % 65536) as u16;
        chunk.copy_from_slice(&v.to_le_bytes());
    }
    data
}

/// A raw interleaved 8-bit RGB buffer — an already-decoded 8-bit source (e.g. a
/// simulated-camera image load) rather than a sensor's raw Bayer output.
fn generate_rgb8_raw(width: usize, height: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 3];
    for (i, v) in data.iter_mut().enumerate() {
        *v = ((i * 37) % 256) as u8;
    }
    data
}

/// A single `from_raw` call on a 2712x1538 `Bayer16` buffer is ~1.15 ms — repeating it
/// `REPS` times clears the ~100 ms floor with headroom for run-to-run noise.
const BAYER16_REPS: usize = 150;

/// The interleaved-scatter arm `Rgb8` exercises is markedly more expensive than the
/// single-channel arm above — ~14.4 ms per call, an order of magnitude more than
/// `Bayer16` at the same resolution — so it needs far fewer repeats to clear the floor.
const RGB8_REPS: usize = 10;

fn frame_ingest_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_ingest");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(1));

    let bayer16 = generate_bayer16_raw(WIDTH, HEIGHT);
    group.throughput(Throughput::Elements((BAYER16_REPS * WIDTH * HEIGHT) as u64));
    group.bench_function(format!("from_raw_bayer16_mono_x{}", BAYER16_REPS), |b| {
        b.iter(|| {
            for _ in 0..BAYER16_REPS {
                black_box(
                    Frame::from_raw(
                        black_box(&bayer16),
                        WIDTH,
                        HEIGHT,
                        1,
                        PixelFormat::Bayer16,
                    )
                    .expect("from_raw failed"),
                );
            }
        })
    });

    let rgb8 = generate_rgb8_raw(WIDTH, HEIGHT);
    group.throughput(Throughput::Elements((RGB8_REPS * WIDTH * HEIGHT) as u64));
    group.bench_function(format!("from_raw_rgb8_x{}", RGB8_REPS), |b| {
        b.iter(|| {
            for _ in 0..RGB8_REPS {
                black_box(
                    Frame::from_raw(black_box(&rgb8), WIDTH, HEIGHT, 3, PixelFormat::Rgb8)
                        .expect("from_raw failed"),
                );
            }
        })
    });

    group.finish();
}

criterion_group!(benches, frame_ingest_benchmark);
criterion_main!(benches);
