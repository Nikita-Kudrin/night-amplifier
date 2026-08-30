//! Frame weighting for quality-based incremental stacking.
//!
//! Scores each arriving frame against the frames already in the stack, so the
//! weight it is blended with reflects how it compares to the session's own
//! typical seeing rather than to an absolute idea of what a good frame is.

use super::config::{FrameQuality, WeightingConfig};

/// Accepted frames needed before scoring means anything. Below this the sample
/// is too small to say what typical looks like, so every frame weighs the same.
const WARMUP_FRAMES: usize = 5;

/// The session's yardstick for frame quality: the median FWHM and SNR of the
/// frames already blended in.
///
/// Medians, not extremes. Normalising against a running min/max — as this did
/// originally — makes a frame's weight depend on its arrival order rather than
/// its quality: whichever frame happens to be the current extreme scores 0.0 or
/// 1.0 however marginally it differs from the rest, and because incremental
/// stacking freezes a frame's weight the moment it is blended, those arbitrary
/// early weights are permanent. Ratios against a median are scale-free and
/// converge, so a frame of a given quality earns the same weight whenever it
/// happens to arrive.
///
/// The median is taken over the whole session, not a rolling window — unlike the
/// frame gate in `server::capture::frame_gate`, which wants to track conditions
/// as they drift. Here a moving yardstick would be the wrong thing: a frame's
/// weight is frozen when it is blended, so weights are only comparable to each
/// other if they were all measured against the same scale. Two frames of equal
/// quality an hour apart must contribute equally.
#[derive(Default)]
pub struct QualityBaseline {
    /// Kept sorted so the median is a lookup and insertion needs no allocation
    /// once capacity has settled.
    fwhms: Vec<f32>,
    snrs: Vec<f32>,
}

impl QualityBaseline {
    /// Folds a frame's metrics into the baseline.
    ///
    /// Call this *after* weighting the frame, never before: a frame that helps
    /// define the yardstick it is measured against is measuring itself.
    pub fn record(&mut self, quality: &FrameQuality) {
        if let Some(fwhm) = quality.fwhm {
            sorted_insert(&mut self.fwhms, fwhm);
        }
        if let Some(snr) = quality.snr {
            sorted_insert(&mut self.snrs, snr);
        }
    }

    /// Number of frames with a usable FWHM or SNR seen so far.
    fn samples(&self) -> usize {
        self.fwhms.len().max(self.snrs.len())
    }

    fn median_fwhm(&self) -> Option<f32> {
        median(&self.fwhms)
    }

    fn median_snr(&self) -> Option<f32> {
        median(&self.snrs)
    }

    /// Calculates the weight this frame should be blended with.
    ///
    /// Each metric scores as a ratio to the session median, capped at 1.0 — a
    /// frame better than typical is worth a full share, not more than one — and
    /// the enabled metrics are renormalised over whichever are actually
    /// measurable on this frame.
    pub fn calculate_weight(&self, quality: &FrameQuality, config: &WeightingConfig) -> f32 {
        if config.is_disabled() || self.samples() < WARMUP_FRAMES {
            return 1.0;
        }

        let mut weighted = 0.0;
        let mut available = 0.0;

        // FWHM is inverted: smaller stars are sharper, so the median goes on top.
        if let (Some(fwhm), Some(median)) = (quality.fwhm, self.median_fwhm()) {
            if fwhm > 0.0 {
                weighted += config.fwhm_weight * (median / fwhm).clamp(0.0, 1.0);
                available += config.fwhm_weight;
            }
        }

        if let (Some(snr), Some(median)) = (quality.snr, self.median_snr()) {
            if median > 0.0 {
                weighted += config.snr_weight * (snr / median).clamp(0.0, 1.0);
                available += config.snr_weight;
            }
        }

        if available <= 0.0 {
            return 1.0;
        }

        (weighted / available)
            .powf(config.power)
            .max(config.min_weight)
    }

    pub fn clear(&mut self) {
        self.fwhms.clear();
        self.snrs.clear();
    }
}

fn sorted_insert(values: &mut Vec<f32>, value: f32) {
    let at = values.partition_point(|&v| v < value);
    values.insert(at, value);
}

fn median(sorted: &[f32]) -> Option<f32> {
    if sorted.is_empty() {
        return None;
    }
    Some(sorted[sorted.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline_of(fwhms: &[f32]) -> QualityBaseline {
        let mut baseline = QualityBaseline::default();
        for &fwhm in fwhms {
            baseline.record(&FrameQuality::new(fwhm, 100.0));
        }
        baseline
    }

    #[test]
    fn warmup_weighs_every_frame_equally() {
        let config = WeightingConfig::balanced();
        let baseline = baseline_of(&[4.0, 8.0]);
        let sharp = baseline.calculate_weight(&FrameQuality::new(3.0, 200.0), &config);
        let soft = baseline.calculate_weight(&FrameQuality::new(9.0, 50.0), &config);
        assert_eq!(sharp, 1.0);
        assert_eq!(soft, 1.0);
    }

    #[test]
    fn a_frame_earns_the_same_weight_whatever_its_arrival_order() {
        let config = WeightingConfig::balanced();
        let subject = FrameQuality::new(4.5, 120.0);
        let others = [4.0, 5.0, 4.2, 6.0, 5.5, 4.8, 5.2];

        let early = baseline_of(&others).calculate_weight(&subject, &config);

        // Same population, opposite order.
        let mut reversed: Vec<f32> = others.to_vec();
        reversed.reverse();
        let late = baseline_of(&reversed).calculate_weight(&subject, &config);

        assert!(
            (early - late).abs() < 1e-6,
            "weight moved with arrival order: {early} vs {late}"
        );
    }

    #[test]
    fn sharper_frames_outweigh_softer_ones() {
        let config = WeightingConfig::fwhm_only();
        let baseline = baseline_of(&[5.0, 5.0, 5.0, 5.0, 5.0, 5.0]);

        let sharp = baseline.calculate_weight(&FrameQuality::new(4.0, 100.0), &config);
        let typical = baseline.calculate_weight(&FrameQuality::new(5.0, 100.0), &config);
        let bloated = baseline.calculate_weight(&FrameQuality::new(10.0, 100.0), &config);

        assert!(sharp >= typical, "{sharp} < {typical}");
        assert!(typical > bloated, "{typical} <= {bloated}");
        assert!(
            (typical - 1.0).abs() < 1e-6,
            "median frame should weigh 1.0"
        );
        assert!(
            (bloated - 0.5).abs() < 1e-6,
            "twice the median should halve"
        );
    }

    #[test]
    fn a_missing_metric_does_not_drag_the_weight_down() {
        let config = WeightingConfig::balanced();
        let baseline = baseline_of(&[5.0, 5.0, 5.0, 5.0, 5.0, 5.0]);

        let both = baseline.calculate_weight(&FrameQuality::new(5.0, 100.0), &config);
        let snr_only = baseline.calculate_weight(&FrameQuality::from_snr(100.0), &config);

        assert!((both - snr_only).abs() < 1e-6, "{both} vs {snr_only}");
    }

    #[test]
    fn disabled_weighting_returns_unity() {
        let baseline = baseline_of(&[4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        let weight =
            baseline.calculate_weight(&FrameQuality::new(20.0, 1.0), &WeightingConfig::disabled());
        assert_eq!(weight, 1.0);
    }
}
