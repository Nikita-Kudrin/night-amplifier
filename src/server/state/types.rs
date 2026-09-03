use crate::stacking::{StackingType, WeightingPreset};

/// Capture session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureState {
    /// No capture in progress
    #[default]
    Idle,
    /// Capture is starting up
    Starting,
    /// Actively capturing frames
    Capturing,
    /// Capture is stopping
    Stopping,
    /// Capture encountered an error
    Error,
}

/// Lifecycle phase of a connected camera handle.
///
/// Orthogonal to `CaptureState`: a camera can be `WarmingUp` while the
/// capture session is `Idle` (the user disconnected but TEC must ramp down
/// before the USB handle is closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraPhase {
    /// No handle open; camera not connected.
    #[default]
    Disconnected,
    /// Handle open, cooler off or not supported — ambient.
    Idle,
    /// Handle open, cooler driving toward `target_temp_c`.
    Precooling,
    /// Handle currently owned by the capture thread.
    Capturing,
    /// Handle open, cooler ramping off before release.
    WarmingUp,
}

#[derive(Clone)]
pub struct StretchResult {
    pub black_point: f32,
    pub scale_lut: std::sync::Arc<Vec<f32>>,
    pub color_intensity: f32,
    /// The shadow floor the row tail still has to apply, resolved against this
    /// solve's sky level and already resampled onto its table. `None` in the common
    /// case, since the floor was fused into `scale_lut` with contrast — set only
    /// when saturation boost kept contrast out of that table and the floor had to
    /// follow (it always goes after contrast). One field, not a floor plus a
    /// `fused` flag: nothing downstream needs the floor that *was* fused, it's
    /// already in the table. Shared, not rebuilt per payload — same reason
    /// `render_task::ConversionCache` exists: a session with lossless + two JPEG
    /// tiers would otherwise resample and allocate the same curve four times a frame.
    pub deferred_shadow_floor: Option<std::sync::Arc<crate::render::ShadowFloorTable>>,
}

/// A frame ready to be rendered and encoded.
/// Replaces the old fully-stretched `Arc<Frame>` in the pipeline,
/// allowing the stretch to be fused into the downsampling loop.
#[derive(Clone)]
pub struct RenderReadyFrame {
    pub linear_frame: std::sync::Arc<crate::frame::Frame>,
    pub pipeline_config: crate::render::RenderPipelineConfig,
    pub stretch_result: Option<StretchResult>,
}
