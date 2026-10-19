//! How frames are shown: the preview and stream resolutions, and the eyepiece view.

use night_amplifier_core::render::denoise::settings::{finite_or, nullable_f32};

/// The longest edge any stream is sent at, `Native` included. A 6248x4176 sensor streamed
/// whole was a 26 MP canvas: past the 4096 px texture limit of many mobile GPUs (a silently
/// black view) and iOS's 16.7 MP canvas cap, with ~940 MB of denoise scratch on the server.
/// 3840² fits all three; the browser mirrors it as `SERVER_MAX_STREAM_EDGE`.
pub const MAX_STREAM_EDGE: u32 = 3840;

/// A resolution the observer picks, as a bounding box: the frame is fitted inside it with
/// its aspect ratio intact and never upscaled. Serves three settings:
///
/// - **Processing Resolution** (`preview_resolution`): how far the preview pipeline may
///   bin before it runs. Preview stages walk every sample, so binning first runs them on a
///   quarter of the data. A setting rather than derived from clients: re-deriving it per
///   frame moved the tone curve under every viewer (**25.7% shadow lift at the 1% input
///   point**) when someone else's tab joined.
/// - **Streaming Resolution** (`streaming_resolution`): the one JPEG size every `/` and
///   `/eyepiece` client receives.
/// - **Eyepiece Streaming Resolution** ([`EyepieceStreamResolution`]): the one RGB8+LZ4 size
///   every `/eyepiece_quality` client receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// No bounding box: every stage sees the full frame, and every stream up to
    /// [`MAX_STREAM_EDGE`].
    Native,
    Uhd2160,
    Qhd1440,
    Hd1080,
}

impl Resolution {
    /// The bounding box, or `None` for [`Self::Native`].
    pub fn target_box(self) -> Option<(u32, u32)> {
        match self {
            Self::Native => None,
            Self::Uhd2160 => Some((3840, 2160)),
            Self::Qhd1440 => Some((2560, 1440)),
            Self::Hd1080 => Some((1920, 1080)),
        }
    }

    /// The box a stream is encoded into: [`Self::target_box`], with `Native` bounded only by
    /// [`MAX_STREAM_EDGE`]. Processing and the saved/snapshot images stay truly native.
    pub fn bounding_box(self) -> (u32, u32) {
        self.target_box().unwrap_or((MAX_STREAM_EDGE, MAX_STREAM_EDGE))
    }

    /// The name the settings UI shows.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::Uhd2160 => "4K",
            Self::Qhd1440 => "1440p",
            Self::Hd1080 => "1080p",
        }
    }
}

/// The Processing Resolution default: never bin, because the alternative silently costs
/// resolution no part of the UI asked for.
pub const DEFAULT_PREVIEW_RESOLUTION: Resolution = Resolution::Native;

/// The Streaming Resolution default for `/` and `/eyepiece`: every processed pixel, as
/// Processing Resolution, so a fresh install loses no detail it did not choose to.
pub const DEFAULT_STREAMING_RESOLUTION: Resolution = Resolution::Native;

pub fn default_preview_resolution() -> Resolution {
    DEFAULT_PREVIEW_RESOLUTION
}

pub fn default_streaming_resolution() -> Resolution {
    DEFAULT_STREAMING_RESOLUTION
}

/// The subset of [`Resolution`] the eyepiece quality stream offers. No 1080p: the eyepiece
/// screens this view exists for are at least 1440 px on their short edge, and a variant
/// that cannot be stored cannot be selected by a stale client either — `serde` rejects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum EyepieceStreamResolution {
    #[default]
    Native,
    Uhd2160,
    Qhd1440,
}

impl EyepieceStreamResolution {
    pub const fn resolution(self) -> Resolution {
        match self {
            Self::Native => Resolution::Native,
            Self::Uhd2160 => Resolution::Uhd2160,
            Self::Qhd1440 => Resolution::Qhd1440,
        }
    }
}

/// Settings specifically for the eyepiece view feature
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct EyepieceSettings {
    /// Enable Binoview
    pub binoview: bool,
    /// Screen width
    #[serde(deserialize_with = "nullable_f32")]
    pub screen_width: f32,
    /// Screen height
    #[serde(deserialize_with = "nullable_f32")]
    pub screen_height: f32,
    /// Measurement unit (e.g. "mm", "inches")
    pub screen_measurement: String,
    /// Screen resolution X
    pub screen_resolution_x: u32,
    /// Screen resolution Y
    pub screen_resolution_y: u32,
    /// Enable Circular view
    #[serde(default = "default_circular_view")]
    pub circular_view: bool,
    /// Dark background enhancement intensity (0.0 to 1.0)
    #[serde(default = "default_intensity", deserialize_with = "nullable_f32")]
    pub intensity: f32,
    /// Where black sits, as a signed fraction of full scale, in `[-0.09, 0.5]`.
    /// **Positive** lifts the output floor — an OLED shows a zero pixel fully off, and
    /// the autostretch black point clamps a few percent of sky pixels to zero, reading
    /// as black speckle at the eyepiece. **Negative** pushes the sky toward black
    /// instead, anchored to the sky rather than full scale: `-0.052` puts the floor at
    /// sky level whether the sky is nominal or bright, so one setting behaves the same
    /// on every target (sky otherwise sits at 14-17 output levels, a visible grey —
    /// dimming via the stretch would dim the target too; see [`intensity`](Self::intensity)).
    #[serde(default = "default_black_floor", deserialize_with = "nullable_f32")]
    pub black_floor: f32,

    /// Let the darkening half of `black_floor` clip to true black instead of
    /// scaling the sky down (`render::output::sky_shadow`).
    ///
    /// Sky noise is as wide as the sky level, so a hard floor puts around 40 %
    /// of all samples on exactly zero. That buys the deepest possible sky and a
    /// little more separation between target and background, and costs the black
    /// speckle the positive half of `black_floor` exists to remove. Off by
    /// default; no effect while `black_floor` is positive.
    #[serde(default)]
    pub darker_sky: bool,
    /// Blue-noise dithering at the 8-bit conversion, to keep smooth gradients from
    /// banding once denoising removes the noise that currently masks the steps.
    #[serde(default = "default_dither")]
    pub dither: bool,
    /// The RGB8+LZ4 size every `/eyepiece_quality` client receives.
    #[serde(default)]
    pub stream_resolution: EyepieceStreamResolution,
}

fn default_intensity() -> f32 {
    0.3
}

fn default_circular_view() -> bool {
    true
}

fn default_black_floor() -> f32 {
    0.04
}

fn default_dither() -> bool {
    true
}

impl EyepieceSettings {
    /// These settings with every non-finite number replaced by its default, applied where
    /// the block enters or leaves, as [`DenoiseSettings::sanitized`] is and for the same
    /// `null` reason. Ranges stay the renderer's business — `stage_config` clamps
    /// `intensity` and `black_floor` where it reads them — so only a value that could
    /// not be saved is replaced here.
    pub fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        self.screen_width = finite_or(self.screen_width, defaults.screen_width);
        self.screen_height = finite_or(self.screen_height, defaults.screen_height);
        self.intensity = finite_or(self.intensity, defaults.intensity);
        self.black_floor = finite_or(self.black_floor, defaults.black_floor);
        self
    }
}

impl Default for EyepieceSettings {
    fn default() -> Self {
        Self {
            binoview: true,
            screen_width: 140.0,
            screen_height: 67.0,
            screen_measurement: "mm".to_string(),
            screen_resolution_x: 2880,
            screen_resolution_y: 1440,
            circular_view: true,
            intensity: 0.3,
            black_floor: default_black_floor(),
            darker_sky: false,
            dither: default_dither(),
            stream_resolution: EyepieceStreamResolution::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use night_amplifier_core::render::display::output_dimensions;

    fn streamed(resolution: Resolution, width: usize, height: usize) -> (usize, usize) {
        let (max_w, max_h) = resolution.bounding_box();
        output_dimensions(width, height, max_w, max_h)
    }

    /// Native streams a sensor whole up to 3840 px a side, and only shrinks one past it.
    #[test]
    fn native_streams_are_capped_at_3840_a_side() {
        assert_eq!(streamed(Resolution::Native, 3008, 3008), (3008, 3008));
        assert_eq!(streamed(Resolution::Native, 3840, 2160), (3840, 2160));
        assert_eq!(streamed(Resolution::Native, 6248, 4176), (3840, 2566));
        assert_eq!(streamed(Resolution::Native, 4176, 6248), (2566, 3840));
        assert_eq!(streamed(Resolution::Native, 9576, 9576), (3840, 3840));
    }

    /// The cap is not the 4K option: that box is 3840x2160, so it stays the smaller size.
    #[test]
    fn native_stays_larger_than_4k_on_a_large_sensor() {
        assert_eq!(streamed(Resolution::Uhd2160, 6248, 4176), (3231, 2160));
        assert_eq!(streamed(Resolution::Uhd2160, 3008, 3008), (2160, 2160));
        assert_eq!(streamed(Resolution::Native, 3008, 3008), (3008, 3008));
    }

    /// Processing reads `target_box`, which the stream cap must not reach.
    #[test]
    fn processing_at_native_is_never_bounded() {
        assert_eq!(Resolution::Native.target_box(), None);
    }
}
