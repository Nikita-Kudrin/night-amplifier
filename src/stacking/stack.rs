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

use super::config::{FrameQuality, StackingConfig};
use super::incremental_pixel::IncrementalPixel;
use super::quality_baseline::QualityBaseline;
use super::rejection::{RejectionMethod, REJECTION_PLUGIN};

/// One row of variance cells from the `r` accumulator rows they cover.
///
/// `pixel_rows` is `r * width` pixels of one plane (fewer on the last stripe), row-major.
fn variance_row(field_row: &mut [f32], pixel_rows: &[IncrementalPixel], width: usize, r: usize) {
    let rows_here = pixel_rows.len() / width.max(1);
    // One allocation per block row rather than per block: at 3008x3008x3 the inner loop runs
    // 425k times.
    let mut samples = Vec::with_capacity(r * r);
    for (bx, cell) in field_row.iter_mut().enumerate() {
        samples.clear();
        let (x0, x1) = (bx * r, ((bx + 1) * r).min(width));
        for row in 0..rows_here {
            for pixel in &pixel_rows[row * width + x0..row * width + x1] {
                // Unmeasured cells are skipped rather than folded in at the floor, which
                // would read downstream as a perfectly clean pixel — see
                // `IncrementalPixel::has_measured_spread` for the two ways that happens.
                if pixel.has_measured_spread() {
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

/// One row of coverage cells, `median(offered) / frame_count`, from the same rows.
///
/// **Subs that reached a pixel, not subs it kept.** A sample the rejector clipped still
/// reached it, and counting kept samples split fully covered cells by one sub wherever
/// the clips per pixel crossed a Poisson median's boundary (169 of 256 at 70 subs under
/// sigma clipping): the map then moved thresholds across a complete stack and the kernels
/// left their plain loop everywhere. A median, so a pixel skipped as non-finite cannot
/// read as a thin border either.
fn coverage_row(cover_row: &mut [f32], pixel_rows: &[IncrementalPixel], width: usize, r: usize, frame_count: usize) {
    let rows_here = pixel_rows.len() / width.max(1);
    let frames = frame_count.max(1) as f32;
    let mut counts = Vec::with_capacity(r * r);
    for (bx, cover) in cover_row.iter_mut().enumerate() {
        counts.clear();
        let (x0, x1) = (bx * r, ((bx + 1) * r).min(width));
        for row in 0..rows_here {
            counts.extend(pixel_rows[row * width + x0..row * width + x1].iter().map(|p| p.offered as f32));
        }
        *cover = if counts.is_empty() {
            0.0
        } else {
            crate::statistics::select_median(&mut counts) / frames
        };
    }
}

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

                    // `offered` is what the coverage map reads. The scale is kept too,
                    // though nothing here clips against it and the render never reads
                    // it: rejection switched on mid-session inherits it warm, and
                    // `noise_field` measures from it on demand. It costs nothing
                    // measurable — the loop is memory-bound and the pixel is read and
                    // written either way (117.2 vs 117.5 ms per eight 3008x3008x3 frames,
                    // x86). Taken before the mean moves, or the deviation is measured
                    // against a mean already holding this sample; outliers are dropped by
                    // the observer itself — see `observe_scale_guarded`.
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

    /// [`Self::compute`], plus the stack's coverage map, from one read of the accumulator.
    ///
    /// The per-frame path: this is what the render task carries to the filters, which
    /// read coverage and nothing else. The full per-pixel variance ([`Self::noise_field`])
    /// is measured on demand instead — reducing its three planes here cost as much again
    /// as the display copy itself, on the thread that drops camera frames when it falls
    /// behind, for a quantity no filter reads. Coverage comes from the first plane only:
    /// the border a sub leaves is the same in every channel. It counts subs that reached
    /// each place, not subs kept there — see `coverage_row`.
    pub fn compute_with_coverage(&self) -> Result<(Frame, NoiseField)> {
        if self.frame_count == 0 {
            return Err(StackError::EmptyStack);
        }

        let (w, h, c) = (self.width, self.height, self.channels);
        let r = crate::frame::NOISE_REDUCTION;
        let (fw, fh) = (w.div_ceil(r).max(1), h.div_ceil(r).max(1));
        let frames = self.frame_count;

        let mut mean = vec![0.0f32; w * h * c];
        let mut coverage = vec![0.0f32; fw * fh];

        let _span = info_span!("compute_pixels", coverage = true).entered();

        let (first_mean, other_mean) = mean.split_at_mut(w * h);
        let (first_pixels, other_pixels) = self.pixels.split_at(w * h);
        rayon::join(
            || {
                // The first plane's stripes yield its mean and its coverage from one read.
                first_mean
                    .par_chunks_mut(r * w)
                    .zip(coverage.par_chunks_mut(fw))
                    .zip(first_pixels.par_chunks(r * w))
                    .for_each(|((mean_rows, cover_row), pixel_rows)| {
                        for (out, pixel) in mean_rows.iter_mut().zip(pixel_rows.iter()) {
                            *out = pixel.mean;
                        }
                        coverage_row(cover_row, pixel_rows, w, r, frames);
                    });
            },
            || {
                other_mean
                    .par_iter_mut()
                    .zip(other_pixels.par_iter())
                    .for_each(|(out, pixel)| *out = pixel.mean);
            },
        );

        let frame = Frame::from_f32_vec(mean, w, h, c)?;
        let field = NoiseField::coverage_only(coverage, fw, fh, c, w, h)?;
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
        let mut coverage = vec![0.0f32; fw * fh];

        // Plane by plane, then 8-row stripes within each: a stripe over the whole buffer
        // straddles two planes whenever the height is not a multiple of 8.
        variance
            .par_chunks_mut(fw * fh)
            .zip(self.pixels.par_chunks(w * h))
            .for_each(|(field_plane, pixel_plane)| {
                field_plane
                    .par_chunks_mut(fw)
                    .zip(pixel_plane.par_chunks(r * w))
                    .for_each(|(field_row, pixel_rows)| variance_row(field_row, pixel_rows, w, r));
            });
        coverage
            .par_chunks_mut(fw)
            .zip(self.pixels[..w * h].par_chunks(r * w))
            .for_each(|(cover_row, pixel_rows)| coverage_row(cover_row, pixel_rows, w, r, self.frame_count));

        NoiseField::new(variance, fw, fh, c, w, h)
            .and_then(|field| field.with_coverage(coverage))
            .expect("noise field dimensions follow the stack's own")
    }

    pub fn config(&self) -> &StackingConfig {
        &self.config
    }

    /// Per pixel, the share of the stack each sample *kept*: `count / frame_count`.
    ///
    /// Not the coverage the noise map carries, which counts subs that *reached* a pixel
    /// (`coverage_row`): the two differ exactly by what the rejector clipped, and this one
    /// is what the rejection tests read as "frames kept".
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

/// Needs the accumulator's private pixels: without the rejection plugin nothing in
/// Community can make `count` and `offered` differ.
#[cfg(test)]
mod coverage_tests {
    use super::*;

    /// A clip is not a border. Every sub reached every pixel here and a scatter of samples
    /// was not kept — most of one cell among them — so the map must read the stack as
    /// complete, while `coverage_map` still reports what was kept.
    #[test]
    fn clipped_samples_still_count_as_coverage() {
        let config = StackingConfig::default().with_rejection(RejectionMethod::None);
        let mut stack = MasterStack::new(32, 32, 1, config).unwrap();
        for _ in 0..10 {
            stack.add_frame(&Frame::filled(32, 32, 1, 0.3).unwrap()).unwrap();
        }
        for (i, pixel) in stack.pixels.iter_mut().enumerate() {
            let (x, y) = (i % 32, i / 32);
            if (x < 8 && y < 8 && (x + y) % 3 != 0) || i % 7 == 0 {
                pixel.count -= 1;
            }
        }

        let (_, field) = stack.compute_with_coverage().unwrap();
        assert!(
            field.coverage().iter().all(|&c| c == 1.0),
            "clipped samples read as thin coverage: {:?}",
            &field.coverage()[..4]
        );
        assert!(!field.is_usable(), "a complete stack must carry no map");
        assert!((stack.coverage_map().data()[1] - 0.9).abs() < 1e-6, "kept share lost the clip");
    }
}
