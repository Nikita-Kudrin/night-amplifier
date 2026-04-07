//! Builder functions for creating master calibration frames

use crate::error::{Result, StackError};
use crate::frame::Frame;

use super::dark::MasterDark;
use super::flat::MasterFlat;

/// Creates a master dark from multiple dark frames by averaging
///
/// # Arguments
/// * `frames` - Vector of dark frames to average
///
/// # Returns
/// A MasterDark containing the averaged dark frame
pub fn create_master_dark(frames: Vec<Frame>) -> Result<MasterDark> {
    if frames.is_empty() {
        return Err(StackError::ArithmeticError {
            message: "Cannot create master dark from zero frames".to_string(),
        });
    }

    let averaged = average_frames(&frames)?;
    Ok(MasterDark::new(averaged))
}

/// Creates a master flat from multiple flat frames by averaging
///
/// # Arguments
/// * `frames` - Vector of flat frames to average
///
/// # Returns
/// A MasterFlat containing the normalized averaged flat frame
pub fn create_master_flat(frames: Vec<Frame>) -> Result<MasterFlat> {
    if frames.is_empty() {
        return Err(StackError::ArithmeticError {
            message: "Cannot create master flat from zero frames".to_string(),
        });
    }

    let averaged = average_frames(&frames)?;
    MasterFlat::new(averaged)
}

/// Averages multiple frames with matching dimensions
fn average_frames(frames: &[Frame]) -> Result<Frame> {
    let first = &frames[0];
    let width = first.width();
    let height = first.height();
    let channels = first.channels();
    let sample_count = first.sample_count();

    for frame in frames.iter().skip(1) {
        if !frame.dimensions_match(first) {
            return Err(StackError::CalibrationDimensionMismatch {
                frame_width: frame.width(),
                frame_height: frame.height(),
                cal_width: width,
                cal_height: height,
            });
        }
    }

    let mut sum = vec![0.0f64; sample_count];
    for frame in frames {
        for (s, &v) in sum.iter_mut().zip(frame.data().iter()) {
            *s += v as f64;
        }
    }

    let inv_count = 1.0 / frames.len() as f64;
    let averaged: Vec<f32> = sum.iter().map(|&s| (s * inv_count) as f32).collect();

    Frame::from_f32_vec(averaged, width, height, channels)
}
