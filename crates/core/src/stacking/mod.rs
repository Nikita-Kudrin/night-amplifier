mod comet_plugin;
mod config;
mod incremental_pixel;
mod pipeline;
mod quality_baseline;
mod rejection;
mod stack;
mod stacker;
mod warp;

#[cfg(test)]
mod stack_tests;

// Re-export public types
pub use comet_plugin::{CometCentroid, CometContext, CometPlugin, CometSettings};
pub use config::{
    FrameQuality, StackingConfig, StackingType, StackingTypeInfo, WeightingConfig, WeightingPreset,
};
pub use incremental_pixel::{
    mean_error_table, scale_alpha_table, IncrementalPixel, CLIPPED_SCALE_WINDOW,
    COLLAPSED_SCALE_MARGIN, MEAN_ERROR_TABLE_LEN, SCALE_FLOOR, SCALE_GUARD_MIN_OBSERVATIONS,
    SCALE_WINDOW, WARMUP_SIGMA_GUARD,
};
pub use pipeline::{FrameProcessingResult, PipelineConfig, StackingPipeline, StackingStats};
pub use rejection::RejectionMethod;
pub use stack::MasterStack;
pub use stacker::Stacker;
pub use warp::{warp_frame, warp_frame_into};

pub use rejection::RejectionPlugin;
