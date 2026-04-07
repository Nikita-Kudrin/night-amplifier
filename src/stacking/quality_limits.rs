//! Frame weighting utilities for quality-based incremental stacking.
//!
//! Computes weights from frame quality metrics (FWHM, SNR) dynamically
//! as new frames arrive, for use in Welford's online variance algorithm.

use super::config::{FrameQuality, WeightingConfig};

/// Global running limits for quality metrics to enable dynamic scoring.
#[derive(Clone, Copy)]
pub struct QualityLimits {
    pub min_fwhm: f32,
    pub max_fwhm: f32,
    pub min_snr: f32,
    pub max_snr: f32,
}

impl Default for QualityLimits {
    fn default() -> Self {
        Self {
            min_fwhm: f32::MAX,
            max_fwhm: f32::MIN,
            min_snr: f32::MAX,
            max_snr: f32::MIN,
        }
    }
}

impl QualityLimits {
    /// Updates the running min/max limits with a new frame's quality.
    pub fn update(&mut self, quality: &FrameQuality) {
        if let Some(fwhm) = quality.fwhm {
            self.min_fwhm = self.min_fwhm.min(fwhm);
            self.max_fwhm = self.max_fwhm.max(fwhm);
        }
        if let Some(snr) = quality.snr {
            self.min_snr = self.min_snr.min(snr);
            self.max_snr = self.max_snr.max(snr);
        }
    }

    /// Computes a normalized score [0, 1] for a single value against running limits.
    fn compute_score(val: Option<f32>, min_val: f32, max_val: f32, invert: bool) -> f32 {
        match val {
            Some(v) => {
                let range = max_val - min_val;
                if range < 1e-10 {
                    1.0
                } else {
                    let normalized = (v - min_val) / range;
                    if invert {
                        1.0 - normalized
                    } else {
                        normalized
                    }
                }
            }
            None => 0.5,
        }
    }

    /// Calculates the dynamic weight of a single frame.
    pub fn calculate_weight(&self, quality: &FrameQuality, config: &WeightingConfig) -> f32 {
        if config.is_disabled() {
            return 1.0;
        }

        // FWHM is inverted: lower FWHM = higher score
        let fwhm_score = Self::compute_score(quality.fwhm, self.min_fwhm, self.max_fwhm, true);

        // SNR is direct: higher SNR = higher score
        let snr_score = Self::compute_score(quality.snr, self.min_snr, self.max_snr, false);

        let combined = config.fwhm_weight * fwhm_score + config.snr_weight * snr_score;
        let scaled = combined.powf(config.power);

        scaled.max(config.min_weight)
    }
}
