#[cfg(feature = "telemetry")]
use opentelemetry::metrics::MeterProvider;
#[cfg(feature = "telemetry")]
use opentelemetry::KeyValue;

/// Record the memory usage of a master stack in bytes.
#[cfg(feature = "telemetry")]
pub fn record_master_stack_memory(bytes: u64, stack_id: &str) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.stacking");
        let gauge = meter
            .u64_gauge("master_stack.memory_bytes")
            .with_description("Memory usage of master stack storage in bytes")
            .with_unit("By")
            .build();
        gauge.record(bytes, &[KeyValue::new("stack_id", stack_id.to_string())]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_master_stack_memory(_bytes: u64, _stack_id: &str) {}

/// Record the number of frames in a master stack.
#[cfg(feature = "telemetry")]
pub fn record_master_stack_frame_count(count: u64, stack_id: &str) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.stacking");
        let gauge = meter
            .u64_gauge("master_stack.frame_count")
            .with_description("Number of frames accumulated in master stack")
            .with_unit("{frames}")
            .build();
        gauge.record(count, &[KeyValue::new("stack_id", stack_id.to_string())]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_master_stack_frame_count(_count: u64, _stack_id: &str) {}

/// Record the number of frame quality entries in a master stack.
#[cfg(feature = "telemetry")]
pub fn record_master_stack_qualities_count(count: u64, stack_id: &str) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.stacking");
        let gauge = meter
            .u64_gauge("master_stack.frame_qualities_count")
            .with_description("Number of frame quality entries stored")
            .with_unit("{entries}")
            .build();
        gauge.record(count, &[KeyValue::new("stack_id", stack_id.to_string())]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_master_stack_qualities_count(_count: u64, _stack_id: &str) {}

/// Record the pixel count of a master stack.
#[cfg(feature = "telemetry")]
pub fn record_master_stack_pixel_count(pixel_count: u64, stack_id: &str) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.stacking");
        let pixel_gauge = meter
            .u64_gauge("master_stack.pixel_count")
            .with_description("Number of pixels in master stack")
            .with_unit("{pixels}")
            .build();
        pixel_gauge.record(
            pixel_count,
            &[KeyValue::new("stack_id", stack_id.to_string())],
        );
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_master_stack_pixel_count(_pixel_count: u64, _stack_id: &str) {}

/// Record the disk writer queue depth.
#[cfg(feature = "telemetry")]
pub fn record_disk_writer_queue_depth(depth: u64) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.disk");
        let gauge = meter
            .u64_gauge("disk_writer.queue_depth")
            .with_description("Current number of frames queued for writing")
            .with_unit("{frames}")
            .build();
        gauge.record(depth, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_disk_writer_queue_depth(_depth: u64) {}

/// Record the disk writer queue capacity.
#[cfg(feature = "telemetry")]
pub fn record_disk_writer_queue_capacity(capacity: u64) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.disk");
        let gauge = meter
            .u64_gauge("disk_writer.queue_capacity")
            .with_description("Maximum queue size for disk writer")
            .with_unit("{frames}")
            .build();
        gauge.record(capacity, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_disk_writer_queue_capacity(_capacity: u64) {}

/// Record the catalog entry count and index sizes.
#[cfg(feature = "telemetry")]
pub fn record_catalog_stats(
    entries_count: u64,
    designation_index_size: u64,
    messier_index_size: u64,
    alias_index_size: u64,
) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.catalog");

        let entries_gauge = meter
            .u64_gauge("catalog.entries_count")
            .with_description("Number of catalog entries loaded")
            .with_unit("{entries}")
            .build();
        entries_gauge.record(entries_count, &[]);

        let index_gauge = meter
            .u64_gauge("catalog.index_size")
            .with_description("Number of entries in catalog index")
            .with_unit("{entries}")
            .build();
        index_gauge.record(
            designation_index_size,
            &[KeyValue::new("index", "designation")],
        );
        index_gauge.record(messier_index_size, &[KeyValue::new("index", "messier")]);
        index_gauge.record(alias_index_size, &[KeyValue::new("index", "alias")]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_catalog_stats(
    _entries_count: u64,
    _designation_index_size: u64,
    _messier_index_size: u64,
    _alias_index_size: u64,
) {
}

/// Record the number of connected cameras.
#[cfg(feature = "telemetry")]
pub fn record_cameras_count(count: u64) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.server");
        let gauge = meter
            .u64_gauge("server.cameras_count")
            .with_description("Number of connected cameras")
            .with_unit("{cameras}")
            .build();
        gauge.record(count, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_cameras_count(_count: u64) {}

/// Record the number of event subscribers.
#[cfg(feature = "telemetry")]
pub fn record_event_subscribers(count: u64) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.server");
        let gauge = meter
            .u64_gauge("server.event_subscribers")
            .with_description("Number of active event subscribers")
            .with_unit("{subscribers}")
            .build();
        gauge.record(count, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_event_subscribers(_count: u64) {}

/// Record the latest frame size in bytes.
#[cfg(feature = "telemetry")]
pub fn record_latest_frame_size(bytes: u64) {
    if let Some(provider) = super::METER_PROVIDER.get() {
        let meter = provider.meter("night_amplifier.server");
        let gauge = meter
            .u64_gauge("server.latest_frame_size")
            .with_description("Size of latest rendered frame in bytes")
            .with_unit("By")
            .build();
        gauge.record(bytes, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_latest_frame_size(_bytes: u64) {}

// ============================================================================
// Per-frame pipeline instrumentation
// ============================================================================
//
// The instruments above are recorded at most a few times per session, so they
// rebuild their meter and instrument on every call. The ones below fire once
// per frame, so they cache the instrument in a `OnceLock` and the per-call cost
// is a single atomic add. Keep this section to *pipeline stages* — anything
// finer (per row, per pixel, per tile) would distort what it measures.

/// A high-level pipeline stage, timed once per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameStage {
    /// Blocking `Camera::capture` call. Dominated by exposure time on real hardware.
    Capture,
    /// Bayer → RGB conversion, wherever it runs (camera provider or simulator).
    Debayer,
    /// One full iteration of the stacking task.
    Stack,
    /// `process_preview_frame` — the preview render pipeline only.
    Render,
    /// LZ4 encode for the lossless eyepiece stream.
    EncodeLz4,
}

impl FrameStage {
    const fn metric_name(self) -> &'static str {
        match self {
            Self::Capture => "frame.capture_ms",
            Self::Debayer => "frame.debayer_ms",
            Self::Stack => "frame.stack_ms",
            Self::Render => "frame.render_ms",
            Self::EncodeLz4 => "frame.encode_lz4_ms",
        }
    }
}

/// Times a pipeline stage and records it on drop.
///
/// Costs one `Instant::now()` pair when telemetry is compiled in, and nothing
/// at all when it is not.
#[must_use = "the stage is recorded when this guard is dropped"]
pub struct StageTimer {
    #[cfg(feature = "telemetry")]
    stage: FrameStage,
    #[cfg(feature = "telemetry")]
    started: std::time::Instant,
    #[cfg(not(feature = "telemetry"))]
    _phantom: std::marker::PhantomData<()>,
}

/// Start timing a pipeline stage. The measurement is recorded when the returned
/// guard is dropped.
#[cfg(feature = "telemetry")]
pub fn time_stage(stage: FrameStage) -> StageTimer {
    StageTimer {
        stage,
        started: std::time::Instant::now(),
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn time_stage(_stage: FrameStage) -> StageTimer {
    StageTimer {
        _phantom: std::marker::PhantomData,
    }
}

#[cfg(feature = "telemetry")]
impl Drop for StageTimer {
    fn drop(&mut self) {
        if let Some(histogram) = pipeline_histogram(self.stage) {
            histogram.record(self.started.elapsed().as_secs_f64() * 1000.0, &[]);
        }
    }
}

/// Cached duration histogram for a stage.
///
/// Nothing is cached until the meter provider exists, so recording before
/// `init_telemetry` cannot poison the slot with a dead instrument.
#[cfg(feature = "telemetry")]
fn pipeline_histogram(
    stage: FrameStage,
) -> Option<&'static opentelemetry::metrics::Histogram<f64>> {
    use opentelemetry::metrics::Histogram;
    use std::sync::OnceLock;

    static CAPTURE: OnceLock<Histogram<f64>> = OnceLock::new();
    static DEBAYER: OnceLock<Histogram<f64>> = OnceLock::new();
    static STACK: OnceLock<Histogram<f64>> = OnceLock::new();
    static RENDER: OnceLock<Histogram<f64>> = OnceLock::new();
    static ENCODE_LZ4: OnceLock<Histogram<f64>> = OnceLock::new();

    let cell = match stage {
        FrameStage::Capture => &CAPTURE,
        FrameStage::Debayer => &DEBAYER,
        FrameStage::Stack => &STACK,
        FrameStage::Render => &RENDER,
        FrameStage::EncodeLz4 => &ENCODE_LZ4,
    };

    if let Some(histogram) = cell.get() {
        return Some(histogram);
    }

    let provider = super::METER_PROVIDER.get()?;
    let histogram = provider
        .meter("night_amplifier.pipeline")
        .f64_histogram(stage.metric_name())
        .with_description("Wall time spent in a live-view pipeline stage")
        .with_unit("ms")
        .build();
    Some(cell.get_or_init(|| histogram))
}

/// Record the JPEG encode time for one resolution tier.
///
/// Separate from [`StageTimer`] because it carries a `tier` attribute — the
/// render task encodes one payload per tier that has clients.
#[cfg(feature = "telemetry")]
pub fn record_jpeg_encode_ms(tier: &'static str, millis: f64) {
    use opentelemetry::metrics::Histogram;
    use std::sync::OnceLock;

    static ENCODE_JPEG: OnceLock<Histogram<f64>> = OnceLock::new();

    let histogram = match ENCODE_JPEG.get() {
        Some(histogram) => histogram,
        None => {
            let Some(provider) = super::METER_PROVIDER.get() else {
                return;
            };
            let built = provider
                .meter("night_amplifier.pipeline")
                .f64_histogram("frame.encode_jpeg_ms")
                .with_description("Wall time spent encoding one JPEG resolution tier")
                .with_unit("ms")
                .build();
            ENCODE_JPEG.get_or_init(|| built)
        }
    };
    histogram.record(millis, &[KeyValue::new("tier", tier)]);
}

#[cfg(not(feature = "telemetry"))]
pub fn record_jpeg_encode_ms(_tier: &'static str, _millis: f64) {}

/// Count a frame published to stream clients. The rate of this counter is the
/// delivered live-view frame rate.
#[cfg(feature = "telemetry")]
pub fn record_frame_published() {
    use opentelemetry::metrics::Counter;
    use std::sync::OnceLock;
    static PUBLISHED: OnceLock<Counter<u64>> = OnceLock::new();

    if let Some(counter) = frame_counter(
        &PUBLISHED,
        "frame.published",
        "Frames published to stream clients",
    ) {
        counter.add(1, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_frame_published() {}

/// Count a frame dropped by pipeline back-pressure.
#[cfg(feature = "telemetry")]
pub fn record_frame_dropped() {
    use opentelemetry::metrics::Counter;
    use std::sync::OnceLock;
    static DROPPED: OnceLock<Counter<u64>> = OnceLock::new();

    if let Some(counter) = frame_counter(
        &DROPPED,
        "frame.dropped",
        "Frames dropped because a pipeline stage was busy",
    ) {
        counter.add(1, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_frame_dropped() {}

/// Count frames the render task discarded to catch up to the newest one.
///
/// A non-zero rate here means the render stage is behind the rest of the
/// pipeline — the cheapest signal that the queues are backing up.
#[cfg(feature = "telemetry")]
pub fn record_frames_skipped_to_latest(count: u64) {
    use opentelemetry::metrics::Counter;
    use std::sync::OnceLock;
    static SKIPPED: OnceLock<Counter<u64>> = OnceLock::new();

    if count == 0 {
        return;
    }
    if let Some(counter) = frame_counter(
        &SKIPPED,
        "frame.render_skipped",
        "Stale frames discarded by the render task to reach the newest one",
    ) {
        counter.add(count, &[]);
    }
}

#[cfg(not(feature = "telemetry"))]
pub fn record_frames_skipped_to_latest(_count: u64) {}

/// Cached counter, following the same "don't cache before the provider exists"
/// rule as [`pipeline_histogram`].
#[cfg(feature = "telemetry")]
fn frame_counter(
    cell: &'static std::sync::OnceLock<opentelemetry::metrics::Counter<u64>>,
    name: &'static str,
    description: &'static str,
) -> Option<&'static opentelemetry::metrics::Counter<u64>> {
    if let Some(counter) = cell.get() {
        return Some(counter);
    }
    let provider = super::METER_PROVIDER.get()?;
    let counter = provider
        .meter("night_amplifier.pipeline")
        .u64_counter(name)
        .with_description(description)
        .build();
    Some(cell.get_or_init(|| counter))
}
