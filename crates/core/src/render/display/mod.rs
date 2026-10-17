//! What a rendered frame looks like on a screen: the fused f32 -> RGB8 kernels every
//! streamed and saved preview goes through, their area-averaging resampler, the sky
//! shadow, and PNG encoding of the result. The server frames these bytes for the wire
//! (`session::encoding`); the disk writer saves them — neither re-derives the pixels.

mod axis_taps;
mod fused;
mod png;
mod sky_shadow_rows;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::frame::{Frame, NoiseField};
use crate::render::{RenderPipelineConfig, ShadowFloorTable, SkyShadow};

pub use fused::{frame_to_rgb8_downsampled, frame_to_rgb8_downsampled_with, output_dimensions};
pub use png::{encode_rgb8_png, encode_rgba8_png, write_png};

#[derive(Clone)]
pub struct StretchResult {
    pub black_point: f32,
    pub scale_lut: Arc<Vec<f32>>,
    pub color_intensity: f32,
    /// The shadow floor the row tail still has to apply, resolved against this solve's
    /// sky level and already resampled onto its table. `None` in the common case,
    /// since the floor was fused into `scale_lut` with contrast — set only when
    /// saturation boost kept contrast out of that table, so the floor must follow
    /// (always after contrast). One field, not a floor plus a `fused` flag: nothing
    /// downstream needs a floor that's already in the table. Shared, not rebuilt per
    /// payload, for the same reason the server's `ConversionCache` exists:
    /// lossless + JPEG at different sizes would otherwise resample it twice a frame.
    pub deferred_shadow_floor: Option<Arc<ShadowFloorTable>>,
    /// The soft darkening, applied by the encoder after the row tail: it reads a
    /// 3x3 neighbourhood, so it can ride neither the LUT nor a per-row pass.
    pub sky_shadow: Option<SkyShadow>,
}

/// A frame ready to be rendered and encoded.
/// Replaces the old fully-stretched `Arc<Frame>` in the pipeline,
/// allowing the stretch to be fused into the downsampling loop.
#[derive(Clone)]
pub struct RenderReadyFrame {
    pub linear_frame: Arc<Frame>,
    pub pipeline_config: RenderPipelineConfig,
    pub stretch_result: Option<StretchResult>,
    /// How much of the stack reached each place of this frame, on a coarse grid — see
    /// [`NoiseField`]. The denoise plugin raises its thresholds where fewer
    /// subs reached, since the mean there is noisier by `sqrt(N / reached)`.
    ///
    /// `None` whenever no accumulator stands behind the frame — live view, the guide
    /// camera, planetary, comet — and whenever every sub covered the whole frame. That is
    /// the common case, and the denoiser's own global estimates have to stay first-class
    /// rather than becoming a degraded mode.
    pub noise: Option<Arc<NoiseField>>,
}

/// Ready frames for tests of the kernels and of what wraps them — the session crate's
/// encoder tests included, through `test-support`.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    use super::*;

    pub fn to_ready_frame(frame: &Frame) -> RenderReadyFrame {
        let mut config = RenderPipelineConfig::default();
        config.contrast = false;
        config.auto_stretch = false;
        config.saturation_boost = false;
        RenderReadyFrame {
            noise: None,
            linear_frame: Arc::new(frame.clone()),
            pipeline_config: config,
            stretch_result: None,
        }
    }

    /// Like `to_ready_frame`, but with `auto_stretch` actually enabled and a real
    /// `StretchResult` attached — every fused-kernel test up to this point runs with
    /// stretch/saturation/contrast all disabled, so the scale-LUT application branch in
    /// `expand_to_rgb8_fused`/`area_downsample_to_rgb8_fused` had no coverage at all.
    pub fn to_ready_frame_with_stretch(
        frame: &Frame,
        black_point: f32,
        scale_lut: Arc<Vec<f32>>,
    ) -> RenderReadyFrame {
        let mut config = RenderPipelineConfig::default();
        config.contrast = false;
        config.auto_stretch = true;
        config.saturation_boost = false;
        RenderReadyFrame {
            noise: None,
            linear_frame: Arc::new(frame.clone()),
            pipeline_config: config,
            stretch_result: Some(StretchResult {
                deferred_shadow_floor: None,
                sky_shadow: None,
                black_point,
                scale_lut,
                color_intensity: 1.0,
            }),
        }
    }
}
