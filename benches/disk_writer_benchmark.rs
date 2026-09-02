use criterion::{criterion_group, criterion_main, Criterion, SamplingMode};
use night_amplifier::camera::{BufferPool, ImageFormat, RawFrame, SensorType};
use night_amplifier::disk_writer::WritingSessionType;
use night_amplifier::{DiskWriter, DiskWriterConfig, FitsMetadata};
use std::hint::black_box;
use std::time::Duration;

/// Benchmark: queue N raw FITS frames through the disk writer and wait for all
/// writes to complete. Measures end-to-end throughput including serialisation
/// and file I/O.
///
/// One 20-frame session is ~33 ms — below the ~100 ms floor. Each session is
/// independent (its own temp dir, writer, and thread), so `REPS` sessions back to back
/// measures the same work each time, the same way a repeated pure call would.
/// **The reported `time:` is for `REPS` sessions, not one.**
fn bench_disk_writer_fits_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("disk_writer");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    const REPS: usize = 4;

    let pool = BufferPool::new();
    let buf = pool.get(1280 * 960 * 2);
    let frame = std::sync::Arc::new(RawFrame {
        data: buf,
        width: 1280,
        height: 960,
        format: ImageFormat::Raw16,
    });
    let metadata = FitsMetadata::new();
    let num_frames: u64 = 20;

    group.bench_function(format!("write_20_fits_raw_1280x960_x{}", REPS), |b| {
        b.iter(|| {
            for _ in 0..REPS {
                let temp_dir = tempfile::tempdir().unwrap();
                let config = DiskWriterConfig::new(temp_dir.path()).with_max_queue_size(30);
                let (writer, handle) = DiskWriter::new(config);

                handle
                    .start_session(WritingSessionType::IndividualFrames, "")
                    .unwrap();

                let writer_task = std::thread::spawn(move || writer.run());

                for i in 0..num_frames {
                    let _ = handle.queue_raw_frame(
                        black_box(std::sync::Arc::clone(&frame)),
                        i,
                        metadata.clone(),
                        SensorType::Mono,
                        None,
                    );
                }

                // Wait for queue to drain
                while handle.queue_depth() > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }

                handle.end_session();
                drop(handle);
                writer_task.join().ok();
            }
        });
    });

    group.finish();
}

/// Benchmark: measure how much disk writing impacts concurrent CPU-bound work.
///
/// `cpu_work_with_disk_io` is ~24 ms and `cpu_work_alone` is ~21 ms on their own — both
/// below the ~100 ms floor. Each iteration is an independent unit of work (its own
/// session for the I/O case, a fresh sum for the CPU-alone case), so `REPS` repeats
/// measures the same work each time.
/// **The reported `time:` is for `REPS` repeats, not one.**
fn bench_disk_writer_cpu_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("disk_writer_contention");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    const IO_REPS: usize = 5;
    const CPU_REPS: usize = 6;

    let pool = BufferPool::new();
    let buf = pool.get(1280 * 960 * 2);
    let frame = std::sync::Arc::new(RawFrame {
        data: buf,
        width: 1280,
        height: 960,
        format: ImageFormat::Raw16,
    });
    let metadata = FitsMetadata::new();

    fn cpu_work(frame: &RawFrame) -> f32 {
        let data = frame.data_slice();
        let mut sum = 0.0f32;
        for &v in data.iter() {
            sum += v as f32;
        }
        sum
    }

    group.bench_function(format!("cpu_work_with_disk_io_x{}", IO_REPS), |b| {
        b.iter(|| {
            for _ in 0..IO_REPS {
                let temp_dir = tempfile::tempdir().unwrap();
                let config = DiskWriterConfig::new(temp_dir.path()).with_max_queue_size(30);
                let (writer, handle) = DiskWriter::new(config);

                handle
                    .start_session(WritingSessionType::IndividualFrames, "")
                    .unwrap();

                let writer_task = std::thread::spawn(move || writer.run());

                for i in 0..10u64 {
                    let _ = handle.queue_raw_frame(
                        std::sync::Arc::clone(&frame),
                        i,
                        metadata.clone(),
                        SensorType::Mono,
                        None,
                    );
                }

                // CPU work on the main thread while disk I/O is happening
                let mut total = 0.0f32;
                for _ in 0..20 {
                    total += black_box(cpu_work(&frame));
                }
                black_box(total);

                while handle.queue_depth() > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }

                handle.end_session();
                drop(handle);
                writer_task.join().ok();
            }
        });
    });

    group.bench_function(format!("cpu_work_alone_x{}", CPU_REPS), |b| {
        b.iter(|| {
            for _ in 0..CPU_REPS {
                let mut total = 0.0f32;
                for _ in 0..20 {
                    total += black_box(cpu_work(&frame));
                }
                black_box(total);
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_disk_writer_fits_throughput,
    bench_disk_writer_cpu_contention
);
criterion_main!(benches);
