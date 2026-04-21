use criterion::{criterion_group, criterion_main, Criterion};
use night_amplifier::disk_writer::WritingSessionType;
use night_amplifier::frame::Frame;
use night_amplifier::{DiskWriter, DiskWriterConfig, FitsMetadata};
use std::hint::black_box;
use std::time::Duration;

/// Benchmark: queue N raw FITS frames through the disk writer and wait for all
/// writes to complete. Measures end-to-end throughput including serialisation
/// and file I/O.
fn bench_disk_writer_fits_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("disk_writer");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let frame = Frame::filled(1280, 960, 1, 0.42).unwrap();
    let metadata = FitsMetadata::new();
    let num_frames: u64 = 20;

    group.bench_function("write_20_fits_raw_1280x960", |b| {
        b.iter(|| {
            let temp_dir = tempfile::tempdir().unwrap();
            let config = DiskWriterConfig::new(temp_dir.path()).with_max_queue_size(30);
            let (writer, handle) = DiskWriter::new(config);

            handle
                .start_session(WritingSessionType::IndividualFrames)
                .unwrap();

            let writer_task = std::thread::spawn(move || writer.run());

            for i in 0..num_frames {
                let _ = handle.queue_raw_frame(black_box(frame.clone()), i, metadata.clone());
            }

            // Wait for queue to drain
            while handle.queue_depth() > 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            handle.end_session();
            drop(handle);
            writer_task.join().ok();
        });
    });

    group.finish();
}

/// Benchmark: measure how much disk writing impacts concurrent CPU-bound work.
fn bench_disk_writer_cpu_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("disk_writer_contention");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let frame = Frame::filled(1280, 960, 1, 0.42).unwrap();
    let metadata = FitsMetadata::new();

    fn cpu_work(frame: &Frame) -> f32 {
        let data = frame.data();
        let mut sum = 0.0f32;
        for &v in data.iter() {
            sum += v;
        }
        sum
    }

    group.bench_function("cpu_work_with_disk_io", |b| {
        b.iter(|| {
            let temp_dir = tempfile::tempdir().unwrap();
            let config = DiskWriterConfig::new(temp_dir.path()).with_max_queue_size(30);
            let (writer, handle) = DiskWriter::new(config);

            handle
                .start_session(WritingSessionType::IndividualFrames)
                .unwrap();

            let writer_task = std::thread::spawn(move || writer.run());

            for i in 0..10u64 {
                let _ = handle.queue_raw_frame(frame.clone(), i, metadata.clone());
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
        });
    });

    group.bench_function("cpu_work_alone", |b| {
        b.iter(|| {
            let mut total = 0.0f32;
            for _ in 0..20 {
                total += black_box(cpu_work(&frame));
            }
            black_box(total);
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
