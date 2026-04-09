//! Night Amplifier - Professional-grade EAA Live Stacking Engine
//!
//! A high-performance astronomy image stacking engine optimized for
//! Electronically Assisted Astronomy (EAA) on embedded platforms like Raspberry Pi 5.

#![allow(dead_code, unused_variables, unused_imports, unused_assignments)]

pub mod background;
pub mod calibration;
pub mod camera;
pub mod debayer;
pub mod detection;
pub mod disk_writer;
pub mod error;
pub mod ffi_safety;
pub mod fits;
pub mod frame;
pub mod logging;
pub mod planetary;
pub mod process;
pub mod push_to;
pub mod registration;
pub mod render;
pub mod ser;
pub mod stacking;
pub mod statistics;
pub mod telemetry;

pub mod app;
pub mod server;

pub use background::{
    subtract_background, subtract_background_with_config, BackgroundConfig, BackgroundExtractor,
    BackgroundModel,
};
pub use calibration::{
    create_master_dark, create_master_flat, Calibration, MasterDark, MasterFlat, FLAT_MIN_THRESHOLD,
};
pub use debayer::{
    debayer, debayer_auto, debayer_auto_with_algorithm, debayer_with_config, debayer_with_pattern,
    detect_cfa_pattern, CfaPattern, DebayerAlgorithm, DebayerConfig, Debayerer,
    PatternDetectionResult,
};
pub use detection::{
    detect_stars, detect_stars_adaptive, detect_stars_adaptive_thorough, detect_stars_sigma,
    BackgroundStats, DetectionConfig, Star, StarDetector,
};
pub use error::{Result, StackError};
pub use frame::{Frame, PixelFormat};
pub use process::{ChildGuard, ExternalProcess};
pub use registration::{
    register_frames, register_frames_adaptive, AdaptiveRegistration, AdaptiveRegistrationResult,
    AffineTransform, BrightnessVariation, FovType, ImageRegistration, RegistrationConfig,
    RegistrationHints, Triangle, TriangleMatcher,
};
pub use render::{
    apply_contrast_frame, apply_s_curve, apply_tone_mapping, asinh, asinh_stretch,
    asinh_stretch_frame, auto_stretch_default, auto_stretch_frame, calculate_black_point,
    calculate_black_points, compute_neutralization_multipliers, downsample, finalize_for_display,
    frame_to_rgb8, frame_to_rgb8_simple, frame_to_rgb8_with_contrast, neutralize_background,
    neutralize_background_auto, render_to_rgb8, render_with_auto_stretch, render_with_stretch,
    subtract_black_point, subtract_black_point_auto, AutoStretchConfig, AutoStretchResult,
    BlackPointConfig, ContrastConfig, OutputConfig, RenderPipeline, RenderPipelineConfig,
    ToneMappingAlgorithm,
};
pub use stacking::{
    warp_frame, warp_frame_into, FrameProcessingResult, MasterStack, PipelineConfig,
    RejectionMethod, Stacker, StackingConfig, StackingPipeline, StackingStats,
};
pub use statistics::{
    compute_image_stats, compute_image_stats_with_config, compute_luminance_stats, ChannelStats,
    ImageStats, StatsConfig,
};

// Camera support (re-export all public types)
pub use camera::{
    Camera, CameraEntry, CameraError, CameraInfo, CameraProvider, CameraRegistry, CameraResult,
    CameraStatus, CaptureConfig, GainPresets, ImageFormat, PlayerOneCamera, PlayerOneProvider,
    SensorType, ZwoCamera, ZwoProvider,
};

// Logging support
pub use logging::{init_default_logging, init_logging, LogConfig, LogGuard, LoggingError};

// FITS file support
pub use fits::{write_fits, write_fits_u16, FitsMetadata};

// Disk writer support
pub use disk_writer::{
    DiskWriter, DiskWriterConfig, DiskWriterError, DiskWriterHandle, FrameType, WriteRequest,
    QUEUE_WARNING_THRESHOLD,
};

// FFI safety utilities
pub use ffi_safety::{
    catch_ffi_panic, catch_ffi_panic_result, validate_buffer_size, validate_dimensions,
    FfiCleanupGuard, FfiError, FfiResult,
};

// Planetary stacking support
pub use planetary::{
    compute_alignment, compute_quality, stack_planetary, AlignmentRoi, PlanetaryConfig,
    PlanetaryStackMethod, PlanetaryStackStats, PlanetaryStacker, QualityMetric, ScoredFrame,
};

// SER video file support
pub use ser::{write_ser, SerColorId, SerHeader, SerReader, SerWriter};

// Push-To navigation support
pub use push_to::{PushToError, PushToResult, PUSH_TO_PLUGIN};

// Telemetry support (OpenTelemetry)
pub use telemetry::{
    is_telemetry_available, is_telemetry_default_enabled, TelemetryConfig, TelemetryError,
    TelemetryGuard,
};
