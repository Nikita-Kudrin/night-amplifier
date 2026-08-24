//! Subtractive Chromatic Noise Reduction (SCNR)
//!
//! This module provides functions for removing green chromatic noise
//! while perfectly preserving the original luminance of the pixel.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;
use tracing::instrument;

/// Apply Subtractive Chromatic Noise Reduction (SCNR) to a frame in-place.
///
/// Uses the Maximum Neutral protection method (G_limit = max(R, B)) combined with
/// Luminance Preservation to ensure OIII regions and overall brightness are not destroyed.
#[instrument(skip(frame), fields(
    resolution = %format!("{}x{}", frame.width(), frame.height()),
    amount = amount
))]
pub fn apply_scnr(frame: &mut Frame, amount: f32) -> Result<()> {
    if frame.channels() != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "SCNR requires 3 channels, got {}",
            frame.channels()
        )));
    }

    if amount <= 0.0 {
        return Ok(());
    }

    let amount = amount.clamp(0.0, 1.0);
    let row_len = frame.width() * 3;
    let data = frame.data_mut();

    data.par_chunks_mut(row_len).for_each(|row| {
        for idx in (0..row.len()).step_by(3) {
            let r = row[idx];
            let g = row[idx + 1];
            let b = row[idx + 2];

            let g_limit = r.max(b);

            if g > g_limit {
                // 1. Calculate original luminance (Rec. 709)
                let l_old = (r * 0.2126) + (g * 0.7152) + (b * 0.0722);

                // 2. Apply Maximum Neutral SCNR to Green
                let g_new = g_limit * amount + g * (1.0 - amount);

                // 3. Calculate new luminance
                let l_new = (r * 0.2126) + (g_new * 0.7152) + (b * 0.0722);

                // 4. Restore luminance with a division-by-zero safeguard
                let ratio = if l_new > 1e-8 { l_old / l_new } else { 1.0 };

                row[idx] = (r * ratio).clamp(0.0, 1.0);
                row[idx + 1] = (g_new * ratio).clamp(0.0, 1.0);
                row[idx + 2] = (b * ratio).clamp(0.0, 1.0);
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scnr_basic() {
        let mut frame = Frame::filled(2, 2, 3, 0.0).unwrap();

        let mut data = vec![0.0f32; 2 * 2 * 3];
        data[0] = 0.1; // R
        data[1] = 0.5; // G (spike)
        data[2] = 0.1; // B

        frame.data_mut().copy_from_slice(&data);

        let l_old = (0.1 * 0.2126) + (0.5 * 0.7152) + (0.1 * 0.0722);

        apply_scnr(&mut frame, 1.0).unwrap();

        let r = frame.get_pixel(0, 0, 0);
        let g = frame.get_pixel(0, 0, 1);
        let b = frame.get_pixel(0, 0, 2);

        let l_new = (r * 0.2126) + (g * 0.7152) + (b * 0.0722);

        // Luminance should be preserved exactly
        assert!((l_old - l_new).abs() < 1e-4);

        // Green should have been reduced (but ratio'd back up due to luminance preservation)
        // Since g_limit = 0.1, it starts low, but luminance preservation scales all channels up.
        assert!(g < 0.5);
        assert!(r > 0.1);
        assert!(b > 0.1);
    }
}
