use crate::frame::Frame;

/// Result of the autostretch solver
#[derive(Debug, Clone, Copy)]
pub struct AutoStretchResult {
    pub stretch_factor: f32,
    pub black_point: f32,
    pub original_median: f32,
    pub adjusted_median: f32,
    pub iterations: u32,
    pub converged: bool,
}

/// Estimate the fraction of pixels that likely contain signal (stars/nebulae)
pub fn estimate_signal_fraction(frame: &Frame, background_mode: f32, sigma: f32) -> f32 {
    let data = frame.data();
    let channels = frame.channels();
    let num_pixels = data.len() / channels;
    let step = (num_pixels / 20000).max(1);

    let threshold = background_mode + 2.0 * sigma;

    let mut signal_count = 0usize;
    let mut total_count = 0usize;

    if channels == 3 {
        for i in (0..num_pixels).step_by(step) {
            let idx = i * 3;
            if idx + 2 < data.len() {
                let lum = 0.2126 * data[idx] + 0.7152 * data[idx + 1] + 0.0722 * data[idx + 2];
                total_count += 1;
                if lum > threshold {
                    signal_count += 1;
                }
            }
        }
    } else {
        for i in (0..num_pixels).step_by(step) {
            let lum = data[i];
            total_count += 1;
            if lum > threshold {
                signal_count += 1;
            }
        }
    }

    if total_count == 0 {
        return 0.0;
    }

    signal_count as f32 / total_count as f32
}
