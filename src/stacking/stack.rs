//! Master stack accumulator for live stacking.
//!
//! Accumulates frames using true O(1) incremental stacking.
//! Frame history is completely discarded in favor of running statistics,
//! providing instantaneous compute times and a flat memory footprint.

use crate::error::{Result, StackError};
use crate::frame::{Frame, NoiseField};
use crate::telemetry::metrics as telemetry_metrics;
use rayon::prelude::*;
use tracing::{info_span, warn};

/// One row of [`NoiseField`] cells, from the `r` accumulator rows they cover.
///
/// `pixel_rows` is `r * width` pixels of one plane (fewer on the last stripe), laid out
/// row-major. Shared by [`MasterStack::noise_field`] and
/// [`MasterStack::compute_with_noise`], which differ only in whether they also copy the
/// mean out of the same read.
fn noise_row(field_row: &mut [f32], pixel_rows: &[IncrementalPixel], width: usize, r: usize) {
    let rows_here = pixel_rows.len() / width.max(1);
    // One allocation per block row rather than per block: at 3008x3008x3 the inner loop
    // runs 425k times a frame.
    let mut samples = Vec::with_capacity(r * r);
    for (bx, cell) in field_row.iter_mut().enumerate() {
        samples.clear();
        let (x0, x1) = (bx * r, ((bx + 1) * r).min(width));
        for row in 0..rows_here {
            for pixel in &pixel_rows[row * width + x0..row * width + x1] {
                // `observe_scale` ignores the first offered sample, so below two there
                // is no spread to report. Skipped rather than folded in as zero, which
                // would read downstream as a perfectly clean pixel.
                if pixel.count >= 2 {
                    samples.push(pixel.variance() / pixel.count as f32);
                }
            }
        }
        *cell = if samples.is_empty() {
            f32::NAN
        } else {
            crate::statistics::select_median(&mut samples)
        };
    }
}

use super::config::{FrameQuality, StackingConfig};
use super::incremental_pixel::IncrementalPixel;
use super::quality_baseline::QualityBaseline;
use super::rejection::{RejectionMethod, REJECTION_PLUGIN};

pub struct MasterStack {
    width: usize,
    height: usize,
    channels: usize,
    config: StackingConfig,
    frame_count: usize,
    pixels: Vec<IncrementalPixel>,
    quality_baseline: QualityBaseline,
    frame_qualities: Vec<FrameQuality>,
}

impl MasterStack {
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        config: StackingConfig,
    ) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        if matches!(
            config.rejection,
            RejectionMethod::SigmaClip
                | RejectionMethod::WinsorizedSigmaClip
                | RejectionMethod::MinMax
        ) && crate::license::pro_plugin(&REJECTION_PLUGIN).is_none()
        {
            return Err(StackError::InvalidConfiguration(
                    "Advanced outlier rejection (Sigma Clipping, MinMax) is only available in Night Amplifier Pro.\n\
                     Please consider upgrading to unlock this feature.".into(),
                ));
        }

        let pixel_count = width * height * channels;
        Ok(Self {
            width,
            height,
            channels,
            config,
            frame_count: 0,
            pixels: vec![IncrementalPixel::new(); pixel_count],
            quality_baseline: QualityBaseline::default(),
            frame_qualities: Vec::new(),
        })
    }

    pub fn with_defaults(width: usize, height: usize, channels: usize) -> Result<Self> {
        Self::new(width, height, channels, StackingConfig::default())
    }

    pub fn add_frame(&mut self, frame: &Frame) -> Result<()> {
        self.add_frame_with_quality(frame, FrameQuality::default())
    }

    pub fn add_frame_with_quality(&mut self, frame: &Frame, quality: FrameQuality) -> Result<()> {
        self.add_frame_with_border_and_quality(frame, 0.0, 1e-6, quality)
    }

    pub fn add_frame_with_border(
        &mut self,
        frame: &Frame,
        border_value: f32,
        border_tolerance: f32,
    ) -> Result<()> {
        self.add_frame_with_border_and_quality(
            frame,
            border_value,
            border_tolerance,
            FrameQuality::default(),
        )
    }

    pub fn add_frame_with_border_and_quality(
        &mut self,
        frame: &Frame,
        border_value: f32,
        border_tolerance: f32,
        quality: FrameQuality,
    ) -> Result<()> {
        if frame.width() != self.width
            || frame.height() != self.height
            || frame.channels() != self.channels
        {
            return Err(StackError::CalibrationDimensionMismatch {
                frame_width: frame.width(),
                frame_height: frame.height(),
                cal_width: self.width,
                cal_height: self.height,
            });
        }

        let data = frame.data();

        // 1. Weigh this frame against the frames already stacked, then fold it
        //    into the baseline. Recording first would let the frame help define
        //    the yardstick it is measured against.
        let weight = self
            .quality_baseline
            .calculate_weight(&quality, &self.config.weighting);
        self.quality_baseline.record(&quality);

        let needs_rejection = matches!(
            self.config.rejection,
            RejectionMethod::SigmaClip | RejectionMethod::WinsorizedSigmaClip
        );

        if needs_rejection {
            if let Some(plugin) = crate::license::pro_plugin(&REJECTION_PLUGIN) {
                plugin.blend_incremental(
                    &mut self.pixels,
                    data,
                    border_value,
                    border_tolerance,
                    weight,
                    &self.config,
                )?;
            } else {
                return Err(StackError::InvalidConfiguration(
                    "Advanced outlier rejection is a Pro feature.".into(),
                ));
            }
        } else {
            let _store_span =
                info_span!("blend_pixels", frame_count = self.frame_count + 1).entered();

            // Built once per frame, not once per pixel: the divide this hoists runs 27
            // million times otherwise. Same reason as the rejection kernel's copy.
            let alphas = super::incremental_pixel::scale_alpha_table();

            // 2. Blend the frame directly into the Master result in O(1) memory (No rejection)
            self.pixels
                .par_iter_mut()
                .zip(data.par_iter())
                .for_each(|(pixel, &val)| {
                    // Skip borders, and non-finite samples: a NaN in a running mean
                    // never leaves again.
                    if !val.is_finite() || (val - border_value).abs() < border_tolerance {
                        return;
                    }

                    // The scale is maintained here too, though nothing on this path
                    // clips against it: `m2` is the render's per-pixel noise estimate
                    // (`MasterStack::noise_field`), and leaving it at zero would make
                    // that field read "perfectly clean" for every Community session and
                    // every test that stacks without the rejection plugin. Measured
                    // before the mean moves, or the deviation is taken against a mean
                    // that already contains this sample. Winsorised by the observer
                    // itself, since there is no rejection threshold here to clamp
                    // against — see `observe_scale_guarded`.
                    pixel.offered = pixel.offered.saturating_add(1);
                    pixel.observe_scale_guarded(val - pixel.mean, &alphas);

                    // Blend into running average
                    pixel.blend(val, weight);
                });

            drop(_store_span);
        }

        self.frame_count += 1;
        self.frame_qualities.push(quality);
        Ok(())
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frame_qualities(&self) -> &[FrameQuality] {
        &self.frame_qualities
    }

    pub fn compute(&self) -> Result<Frame> {
        if self.frame_count == 0 {
            return Err(StackError::EmptyStack);
        }

        let pixel_count = self.width * self.height * self.channels;
        let mut result = vec![0.0f32; pixel_count];

        let _span = info_span!("compute_pixels").entered();

        // Just extract the running mean. Zero math required!
        result
            .par_iter_mut()
            .zip(self.pixels.par_iter())
            .for_each(|(res, p)| {
                *res = p.mean;
            });

        Frame::from_f32_vec(result, self.width, self.height, self.channels)
    }

    /// [`Self::compute`] and [`Self::noise_field`] from one traversal of the accumulator.
    ///
    /// The two passes read the same 434 MB on a 3008x3008 colour stack, on the thread
    /// that is dropping camera frames when it falls behind, so the display copy and the
    /// noise map are taken together: block-row stripes, each pixel read once. Callers
    /// wanting only the mean keep using `compute`, whose flat zip is the cheapest form
    /// of that on its own.
    pub fn compute_with_noise(&self) -> Result<(Frame, NoiseField)> {
        if self.frame_count == 0 {
            return Err(StackError::EmptyStack);
        }

        let (w, h, c) = (self.width, self.height, self.channels);
        let r = crate::frame::NOISE_REDUCTION;
        let (fw, fh) = (w.div_ceil(r).max(1), h.div_ceil(r).max(1));

        let mut mean = vec![0.0f32; w * h * c];
        let mut variance = vec![f32::NAN; fw * fh * c];

        let _span = info_span!("compute_pixels", noise_field = true).entered();

        mean.par_chunks_mut(w * h)
            .zip(variance.par_chunks_mut(fw * fh))
            .zip(self.pixels.par_chunks(w * h))
            .for_each(|((mean_plane, field_plane), pixel_plane)| {
                mean_plane
                    .par_chunks_mut(r * w)
                    .zip(field_plane.par_chunks_mut(fw))
                    .zip(pixel_plane.par_chunks(r * w))
                    .for_each(|((mean_rows, field_row), pixel_rows)| {
                        for (out, pixel) in mean_rows.iter_mut().zip(pixel_rows.iter()) {
                            *out = pixel.mean;
                        }

                        noise_row(field_row, pixel_rows, w, r);
                    });
            });

        let frame = Frame::from_f32_vec(mean, w, h, c)?;
        let field = NoiseField::new(variance, fw, fh, c, w, h)?;
        Ok((frame, field))
    }

    /// Per-pixel variance of the stacked mean, block-reduced — see [`NoiseField`].
    ///
    /// `m2` is an exponentially weighted mean of squared deviation over *offered*
    /// samples, i.e. the current single-sub variance; dividing by `count` turns it into
    /// the error of the mean this stack reports. Three approximations ride on that and
    /// each is small enough to name rather than correct:
    ///
    /// - The mean is weighted, so exactly `SE^2 = sigma^2 * sum(w^2)/sum(w)^2`. The
    ///   accumulator does not keep `sum(w^2)` and must not grow to; with near-equal
    ///   quality weights that reduces to the form used here.
    /// - Clipped samples enter `m2` winsorised at the threshold, so a pixel that clips
    ///   often reads slightly low. That is the right bias: a cosmic ray is not noise the
    ///   denoiser has to survive.
    /// - `m2` remembers about `SCALE_WINDOW` samples while `count` remembers all of
    ///   them, so after a real change in sky brightness the ratio is wrong for roughly
    ///   that many frames.
    ///
    /// A block **median**, not a mean: a star's shot noise and its registration jitter
    /// are real per-pixel variance but not what a sky threshold is asking about, and a
    /// mean over 64 samples lets one of them set the block.
    pub fn noise_field(&self) -> NoiseField {
        let (w, h, c) = (self.width, self.height, self.channels);
        let r = crate::frame::NOISE_REDUCTION;
        let (fw, fh) = (w.div_ceil(r).max(1), h.div_ceil(r).max(1));
        let mut variance = vec![f32::NAN; fw * fh * c];

        variance
            .par_chunks_mut(fw * fh)
            .zip(self.pixels.par_chunks(w * h))
            .for_each(|(field_plane, pixel_plane)| {
                field_plane
                    .par_chunks_mut(fw)
                    .zip(pixel_plane.par_chunks(r * w))
                    .for_each(|(field_row, pixel_rows)| {
                        noise_row(field_row, pixel_rows, w, r);
                    });
            });

        NoiseField::new(variance, fw, fh, c, w, h)
            .expect("noise field dimensions follow the stack's own")
    }

    pub fn config(&self) -> &StackingConfig {
        &self.config
    }

    pub fn coverage_map(&self) -> Frame {
        let max_count = self.frame_count as f32;
        let data: Vec<f32> = self
            .pixels
            .iter()
            .map(|p| p.count as f32 / max_count.max(1.0))
            .collect();

        Frame::from_f32_vec(data, self.width, self.height, self.channels)
            .expect("Coverage map creation should not fail")
    }

    pub fn clear(&mut self) {
        self.frame_count = 0;
        self.pixels.par_iter_mut().for_each(|p| p.reset());
        self.quality_baseline.clear();
        self.frame_qualities.clear();
    }

    /// Update the stacking configuration dynamically.
    ///
    /// This allows changing rejection methods or sigma thresholds mid-stack.
    /// Subsequent frames will use the new configuration.
    pub fn update_config(&mut self, mut config: StackingConfig) {
        // Enforce Pro gating during dynamic updates
        if matches!(
            config.rejection,
            RejectionMethod::SigmaClip
                | RejectionMethod::WinsorizedSigmaClip
                | RejectionMethod::MinMax
        ) && crate::license::pro_plugin(&REJECTION_PLUGIN).is_none()
        {
            warn!("Ignoring request for advanced rejection method - Night Amplifier Pro required.");
            config.rejection = RejectionMethod::None;
        }
        self.config = config;
    }

    pub fn memory_usage(&self) -> usize {
        self.pixels.len() * std::mem::size_of::<IncrementalPixel>()
    }

    pub fn record_metrics(&self, stack_id: &str) {
        let pixel_count = (self.width * self.height * self.channels) as u64;

        telemetry_metrics::record_master_stack_memory(self.memory_usage() as u64, stack_id);
        telemetry_metrics::record_master_stack_frame_count(self.frame_count as u64, stack_id);
        telemetry_metrics::record_master_stack_qualities_count(
            self.frame_qualities.len() as u64,
            stack_id,
        );
        telemetry_metrics::record_master_stack_pixel_count(pixel_count, stack_id);
    }
}
