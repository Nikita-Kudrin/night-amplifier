//! How frames are shown: the preview and stream resolutions, and the eyepiece view.

use night_amplifier_core::render::denoise::settings::{finite_or, nullable_f32};

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
    /// No bounding box: every stage and stream sees the full frame.
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

    /// [`Self::target_box`] with `Native` as an unbounded box, for the encoders.
    pub fn bounding_box(self) -> (u32, u32) {
        self.target_box().unwrap_or((u32::MAX, u32::MAX))
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

/// The Streaming Resolution default for `/` and `/eyepiece`.
pub const DEFAULT_STREAMING_RESOLUTION: Resolution = Resolution::Qhd1440;

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
    Native,
    Uhd2160,
    #[default]
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
