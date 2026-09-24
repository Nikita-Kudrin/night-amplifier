//! Spatial denoising: the boundary, not the filters.
//!
//! The filters themselves are a Pro feature and live in `night-amplifier-pro`'s
//! `plugins::denoise`. What stays here is what Community has to own either way: the
//! config the encoders read, the buffer pool the render thread lends the filters, the
//! trait they arrive through, and the gates that decide whether they run at all.
//! Without the plugin `DenoiseConfig` is always [`DenoiseConfig::OFF`] and the encoders
//! take their fused per-row path, which is byte-identical to the pre-denoise output
//! rather than merely equivalent.
//!
//! The filters run at **stream resolution** inside the encoders, not in the render
//! pipeline: the pipeline's frame is sensor resolution (9MP on IMX533) against a 1440²
//! eyepiece, so denoising and then discarding 3/4 of it is 4.5x the DRAM traffic for
//! nothing (576MB/frame vs ~128MB at display size), and the encoder's downsample is
//! itself a 2x noise reduction that eases their job. They sit between downsample and
//! tone curve (staged by `server::encoding::fused`), in linear light because the
//! stretch has not run yet — post-tone-curve the same noise spans wildly different
//! amplitudes by brightness and one threshold could not describe it.
//!
//! The configs below carry numbers somebody tuned by looking at a picture. **Community
//! never fills them in**: every `Default` here is off, and the values come from the
//! plugin along with the code that earned them.

use std::sync::OnceLock;

/// Wavelet levels a [`LumaDenoiseConfig`] carries per-level values for.
///
/// The one filter constant Community keeps, because it is an array size rather than a
/// tuning decision.
pub const MAX_LEVELS: usize = 6;

/// À trous wavelet denoising of the luminance plane.
///
/// A data carrier: what `k` and `gain` mean is defined by the plugin that fills them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LumaDenoiseConfig {
    /// Per-level gain applied to what survives the threshold, finest first.
    pub gain: [f32; MAX_LEVELS],
    pub enabled: bool,
    /// Per-level threshold in sigmas of that level's noise, finest first.
    pub k: [f32; MAX_LEVELS],
    /// Overall amount, scaling every threshold.
    pub strength: f32,
}

impl LumaDenoiseConfig {
    pub const OFF: Self = Self {
        gain: [1.0; MAX_LEVELS],
        enabled: false,
        k: [0.0; MAX_LEVELS],
        strength: 0.0,
    };

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.strength > 0.0
    }
}

impl Default for LumaDenoiseConfig {
    fn default() -> Self {
        Self::OFF
    }
}

/// Guided-filter smoothing of the chroma planes.
///
/// A data carrier, like [`LumaDenoiseConfig`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromaDenoiseConfig {
    pub enabled: bool,
    /// Filter radius in **display** pixels.
    ///
    /// Display, not sensor: this stage runs after the encoder's downsample, so a radius
    /// carried over from full resolution would cover a quarter of the intended area.
    pub radius: usize,
    /// Edge threshold, in multiples of the guide's noise sigma on the subsampled grid.
    pub noise_k: f32,
    /// Resolution divisor for the coefficient solve.
    pub subsample: usize,
    /// Blend between the original and filtered chroma, `0..=1`.
    pub strength: f32,
}

impl ChromaDenoiseConfig {
    pub const OFF: Self = Self {
        enabled: false,
        radius: 0,
        noise_k: 0.0,
        subsample: 1,
        strength: 0.0,
    };

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.strength > 0.0 && self.radius > 0
    }
}

impl Default for ChromaDenoiseConfig {
    fn default() -> Self {
        Self::OFF
    }
}

/// Both spatial denoisers, as the encoders see them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DenoiseConfig {
    /// À trous wavelet denoising of the luminance plane.
    pub luma: LumaDenoiseConfig,
    /// Guided-filter smoothing of the two chroma planes.
    pub chroma: ChromaDenoiseConfig,
}

impl Default for DenoiseConfig {
    fn default() -> Self {
        Self::OFF
    }
}

impl DenoiseConfig {
    /// Neither filter runs — and, without the plugin, the only config there is.
    ///
    /// The encoders take their original fused path for this, so it is byte-identical to
    /// the pre-denoise output rather than merely equivalent.
    pub const OFF: Self = Self {
        luma: LumaDenoiseConfig::OFF,
        chroma: ChromaDenoiseConfig::OFF,
    };

    /// Whether anything at all would happen, which is what decides between the encoders'
    /// fused and staged traversals.
    pub fn is_enabled(&self) -> bool {
        self.luma.is_enabled() || self.chroma.is_enabled()
    }
}

/// The spatial denoisers, and the mapping from the observer's controls onto them.
///
/// Both halves are here because both are tuning: which thresholds a dial position means
/// is no more a Community decision than what the filter does with them.
pub trait DenoisePlugin: Send + Sync {
    /// Map the observer's controls onto the config the encoders read.
    ///
    /// `aggressiveness` is the stretch profile, which changes what the luma filter is
    /// being asked for — a field of stars has no nebulosity to protect. The Planetary
    /// gate is applied by the caller before this is reached, so no plugin can get it
    /// wrong.
    fn config(
        &self,
        settings: &crate::server::state::DenoiseSettings,
        aggressiveness: crate::render::StretchAggressiveness,
    ) -> DenoiseConfig;

    /// The share of the stack's depth the tone curve spends on a calmer sky, for this
    /// dial position. See [`crate::render::AutoStretchConfig::grain_split`].
    ///
    /// Without a plugin Community pins `DEFAULT_GRAIN_SPLIT`, which is what the dial's
    /// middle resolves to — so the tone curve renders identically either way and the
    /// whole difference is the filters. Only asked while the Denoise switch is on; off,
    /// Community pins the same value ([`grain_split_for`]).
    fn grain_split(&self, settings: &crate::server::state::DenoiseSettings) -> f32;

    /// Denoise one staged interleaved RGB f32 image in place, at output resolution, in
    /// linear light. `buf` is `width * height * 3` samples.
    ///
    /// `noise` is the stack's coverage — the share of its subs that reached each place —
    /// already resampled onto *this image's* grid by the encoder that owns the resample
    /// kernel; see [`crate::frame::NoiseField`]. It carries no variance plane on this path.
    /// `None` whenever no accumulator stands behind the frame (live view, the guide camera,
    /// planetary, comet) or every sub covered all of it, which is the common case: the
    /// filters' own global estimates have to stay first-class, not a degraded mode.
    fn denoise_rgb_interleaved(
        &self,
        buf: &mut [f32],
        width: usize,
        height: usize,
        config: &DenoiseConfig,
        noise: Option<&crate::frame::NoiseField>,
        scratch: &mut DenoiseScratch,
    );
}

/// Global registry for the denoise plugin.
pub static DENOISE_PLUGIN: OnceLock<Box<dyn DenoisePlugin>> = OnceLock::new();

/// Reusable working buffers for one denoise pass. At 1440² a pass touches ~75MB
/// (staged interleaved RGB, three planar channels, three more for the transform),
/// freshly zero-initialised and dropped per payload per frame — measured 13ms of the
/// 20ms denoising adds to an encode (page faults, not arithmetic).
///
/// Owned by the render task's thread and passed down, not thread-local: the inline
/// encode priming a newly-connected client runs on a pooled tokio blocking thread, where
/// a thread-local would strand 75MB per thread the pool ever grows to.
///
/// Memory ownership rather than logic, so it stays in Community even though only the
/// plugin reads most of it. The `planes` + `aux` shape is part of the trait contract.
#[derive(Default)]
pub struct DenoiseScratch {
    /// Interleaved RGB at output resolution, between the resample and the tone curve.
    /// Filled by the encoder, not by the filters, so it is taken out rather than
    /// borrowed.
    pub(crate) staged: Vec<f32>,
    /// Three full-size planes: the filters' working colour space.
    pub planes: [Vec<f32>; 3],
    /// Three more, for whatever the luma filter needs to ping-pong through.
    pub aux: [Vec<f32>; 3],
}

/// Grow `buf` to hold at least `len` samples and hand back exactly `len`.
///
/// Grow-only, and deliberately not re-zeroed between passes: every consumer fills what
/// it takes before reading it, so clearing a 25 MB buffer each frame would be a memset
/// with no reader.
pub fn take(buf: &mut Vec<f32>, len: usize) -> &mut [f32] {
    if buf.len() < len {
        buf.resize(len, 0.0);
    }
    &mut buf[..len]
}

/// Denoise an interleaved RGB f32 image in place, at the resolution it will be
/// displayed at. Allocates its own buffers; see [`denoise_rgb_interleaved_with`].
pub fn denoise_rgb_interleaved(
    buf: &mut [f32],
    width: usize,
    height: usize,
    config: &DenoiseConfig,
) {
    denoise_rgb_interleaved_with(
        buf,
        width,
        height,
        config,
        None,
        &mut DenoiseScratch::default(),
    );
}

/// [`denoise_rgb_interleaved`], reusing a caller-owned set of buffers and carrying the
/// frame's noise map.
///
/// A no-op without the plugin, which is also why the encoders check
/// [`DenoiseConfig::is_enabled`] first: `OFF` is the only config Community can produce,
/// so the staged traversal is never entered and the fused path stays byte-identical to
/// what it was before denoising existed.
pub fn denoise_rgb_interleaved_with(
    buf: &mut [f32],
    width: usize,
    height: usize,
    config: &DenoiseConfig,
    noise: Option<&crate::frame::NoiseField>,
    scratch: &mut DenoiseScratch,
) {
    let pixels = width * height;
    if pixels == 0 || buf.len() < pixels * 3 || !config.is_enabled() {
        return;
    }
    let Some(plugin) = crate::license::pro_plugin(&DENOISE_PLUGIN) else {
        return;
    };

    let _span = tracing::info_span!("denoise", width, height).entered();

    // Narrowed rather than passed whole: the filters index their planes from this
    // buffer, so a caller-supplied slice longer than the image would run off the end.
    plugin.denoise_rgb_interleaved(&mut buf[..pixels * 3], width, height, config, noise, scratch);
}

/// The denoise config for these settings, or [`DenoiseConfig::OFF`] without the plugin.
///
/// The Planetary gate lives at the call site in `server::capture::stage_config`, not
/// here: it is a product rule rather than tuning, and it sits beside the same asymmetry
/// `cfa::fpn`, superpixel debayering and the black floor each state at their own site.
pub fn config_for(
    settings: &crate::server::state::DenoiseSettings,
    aggressiveness: crate::render::StretchAggressiveness,
) -> DenoiseConfig {
    crate::license::pro_plugin(&DENOISE_PLUGIN)
        .map(|plugin| plugin.config(settings, aggressiveness))
        .unwrap_or(DenoiseConfig::OFF)
}

/// The tone curve's grain split for these settings.
///
/// [`crate::render::DEFAULT_GRAIN_SPLIT`] — the value the dial's own middle resolves to —
/// without the plugin **and with the Denoise switch off**. So Community's tone curve is
/// exactly the one a Pro observer gets at the default dial position, and switching Denoise
/// off gives exactly Community's picture. The UI greys the Background Grain dial out with
/// the filters; a dial that still moved the curve would be a control nobody can reach
/// (measured before this gate: 1/12 at 0 %, 1/8 at the middle, switch off).
pub fn grain_split_for(settings: &crate::server::state::DenoiseSettings) -> f32 {
    if !settings.enabled {
        return crate::render::DEFAULT_GRAIN_SPLIT;
    }
    crate::license::pro_plugin(&DENOISE_PLUGIN)
        .map(|plugin| plugin.grain_split(settings))
        .unwrap_or(crate::render::DEFAULT_GRAIN_SPLIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_community_default_is_off() {
        assert!(!DenoiseConfig::default().is_enabled());
        assert!(!LumaDenoiseConfig::default().is_enabled());
        assert!(!ChromaDenoiseConfig::default().is_enabled());
        assert_eq!(DenoiseConfig::default(), DenoiseConfig::OFF);
    }

    /// No plugin, no filter — and, crucially, no panic and no partial pass: the buffer
    /// has to come back exactly as it went in.
    #[test]
    fn without_the_plugin_the_image_is_untouched() {
        let before: Vec<f32> = (0..16 * 16 * 3).map(|i| (i % 97) as f32 / 97.0).collect();
        let mut buf = before.clone();

        // Ask for something, not `OFF`, so the early return being tested is the plugin
        // lookup rather than the config gate.
        let config = DenoiseConfig {
            luma: LumaDenoiseConfig {
                enabled: true,
                strength: 1.0,
                ..LumaDenoiseConfig::OFF
            },
            chroma: ChromaDenoiseConfig::OFF,
        };
        assert!(config.is_enabled());

        denoise_rgb_interleaved_with(&mut buf, 16, 16, &config, None, &mut Default::default());
        assert_eq!(buf, before);
    }

    #[test]
    fn without_the_plugin_the_config_is_off_and_the_split_is_the_dials_middle() {
        let settings = crate::server::state::DenoiseSettings::default();
        assert_eq!(
            config_for(&settings, crate::render::StretchAggressiveness::Medium),
            DenoiseConfig::OFF
        );
        assert_eq!(
            grain_split_for(&settings),
            crate::render::DEFAULT_GRAIN_SPLIT
        );
    }

    #[test]
    fn take_grows_but_does_not_shrink() {
        let mut buf = Vec::new();
        assert_eq!(take(&mut buf, 8).len(), 8);
        assert_eq!(take(&mut buf, 4).len(), 4);
        assert!(buf.len() >= 8, "the pool must not give memory back per pass");
    }
}
