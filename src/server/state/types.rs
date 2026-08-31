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
    /// The shadow floor the row tail still has to apply, resolved against the
    /// sky level this solve produced and already resampled onto its table.
    ///
    /// `None` in the common case, because the floor was fused into `scale_lut`
    /// along with contrast. It is only set when saturation boost kept contrast
    /// out of that table and the floor had to follow it out, since the floor
    /// goes after contrast either way.
    ///
    /// One field rather than a floor plus a `fused` flag: two fields that must
    /// agree is one more thing to get wrong than a field that means what its
    /// name says. Nothing downstream needs to know the floor that *was* fused —
    /// it is already in the table.
    ///
    /// Shared rather than rebuilt per payload, for the reason
    /// `render_task::ConversionCache` exists: the table depends only on this
    /// solve, so a session with the lossless stream and two JPEG tiers would
    /// otherwise resample the same curve four times a frame and allocate for it
    /// each time.
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
