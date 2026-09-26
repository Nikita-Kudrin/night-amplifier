//! The AI denoiser's boundary: its config as data, the trait it arrives through, and the
//! call the encoder makes. The network, its weights and every tuned number live in the Pro
//! repo's `plugins::ai_denoise`; without that plugin the config is always
//! [`AiDenoiseConfig::OFF`] and nothing here runs.
//!
//! It runs **display-referred**, where the classic filters run in linear light: after the
//! stretch, before saturation, the S-curve and the floor, at stream resolution. The model
//! was trained on stretched, background-compensated RGB, so linear light is a distribution
//! it never saw; placed after the S-curve it softened stars more (M27 excess at r=5 px
//! +43 % against +26 %). So with it on, the S-curve stays out of the fused scale LUT
//! (`server::capture::pipeline`) and the encoder stages the stretch before calling it
//! (`server::encoding::fused`).

use std::sync::OnceLock;

use super::DenoiseScratch;
use crate::server::state::DenoiseSettings;

/// The network as the encoders see it. A data carrier: the plugin fills it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiDenoiseConfig {
    pub enabled: bool,
    /// How far each pixel moves toward the network's output, `0..=1`.
    pub strength: f32,
    /// Display luminance from which the pre-network pixels are blended back in, reaching
    /// all of them at full scale: the network softens star cores.
    pub highlight_floor: f32,
}

impl AiDenoiseConfig {
    pub const OFF: Self = Self {
        enabled: false,
        strength: 0.0,
        highlight_floor: 0.0,
    };

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.strength > 0.0
    }
}

impl Default for AiDenoiseConfig {
    fn default() -> Self {
        Self::OFF
    }
}

/// The network, and what the observer's settings mean for it.
pub trait AiDenoisePlugin: Send + Sync {
    /// The tuned config. Only asked while the observer has the network switched on and
    /// nothing holds it off (`server::capture::stage_config`).
    fn config(&self, settings: &DenoiseSettings) -> AiDenoiseConfig;

    /// Denoise one staged interleaved RGB f32 image in place, at output resolution,
    /// **after the stretch** (display-referred, nominally `0..=1`). `buf` is
    /// `width * height * 3` samples.
    fn denoise_display_rgb(
        &self,
        buf: &mut [f32],
        width: usize,
        height: usize,
        config: &AiDenoiseConfig,
        scratch: &mut DenoiseScratch,
    );
}

/// Global registry for the AI denoise plugin.
pub static AI_DENOISE_PLUGIN: OnceLock<Box<dyn AiDenoisePlugin>> = OnceLock::new();

/// The network's config: [`AiDenoiseConfig::OFF`] unless the observer asked for it and
/// the plugin is here to answer.
pub fn config_for(settings: &DenoiseSettings) -> AiDenoiseConfig {
    if !settings.ai {
        return AiDenoiseConfig::OFF;
    }
    crate::license::pro_plugin(&AI_DENOISE_PLUGIN)
        .map(|plugin| plugin.config(settings))
        .unwrap_or(AiDenoiseConfig::OFF)
}

/// Run the network over a staged, already stretched image, in place. A no-op without
/// the plugin or with a config that asks for nothing.
pub fn denoise_display_rgb_with(
    buf: &mut [f32],
    width: usize,
    height: usize,
    config: &AiDenoiseConfig,
    scratch: &mut DenoiseScratch,
) {
    let pixels = width * height;
    if pixels == 0 || buf.len() < pixels * 3 || !config.is_enabled() {
        return;
    }
    let Some(plugin) = crate::license::pro_plugin(&AI_DENOISE_PLUGIN) else {
        return;
    };

    let _span = tracing::info_span!("ai_denoise", width, height).entered();
    // Narrowed for the same reason as the linear filters' call: the plugin indexes its
    // planes from this buffer.
    plugin.denoise_display_rgb(&mut buf[..pixels * 3], width, height, config, scratch);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_off() {
        assert_eq!(AiDenoiseConfig::default(), AiDenoiseConfig::OFF);
        assert!(!AiDenoiseConfig::OFF.is_enabled());
        let zero_strength = AiDenoiseConfig {
            enabled: true,
            ..AiDenoiseConfig::OFF
        };
        assert!(!zero_strength.is_enabled(), "a zero strength asks for nothing");
    }

    /// Asked for, but no plugin to answer: the settings alone never produce a config.
    #[test]
    fn without_the_plugin_the_config_is_off_whatever_the_settings_say() {
        for ai in [false, true] {
            let settings = DenoiseSettings {
                ai,
                ..Default::default()
            };
            assert_eq!(config_for(&settings), AiDenoiseConfig::OFF);
        }
    }

    /// The config gate is not what is under test here — an enabled config is — so the
    /// early return exercised is the plugin lookup.
    #[test]
    fn without_the_plugin_the_image_is_untouched() {
        let before: Vec<f32> = (0..16 * 16 * 3).map(|i| (i % 89) as f32 / 89.0).collect();
        let mut buf = before.clone();
        let config = AiDenoiseConfig {
            enabled: true,
            strength: 1.0,
            highlight_floor: 0.2,
        };
        assert!(config.is_enabled());

        denoise_display_rgb_with(&mut buf, 16, 16, &config, &mut Default::default());
        assert_eq!(buf, before);
    }
}
