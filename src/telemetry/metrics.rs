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
