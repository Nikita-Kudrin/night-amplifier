use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;

/// Downsample a frame by a factor (simple box filter)
pub fn downsample(frame: &Frame, factor: usize) -> Result<Frame> {
    if factor == 0 {
        return Err(StackError::InvalidConfiguration(
            "Downsample factor must be > 0".into(),
        ));
    }

    if factor == 1 {
        return Ok(frame.clone());
    }

    let src_width = frame.width();
    let src_height = frame.height();
    let channels = frame.channels();

    let dst_width = src_width / factor;
    let dst_height = src_height / factor;

    if dst_width == 0 || dst_height == 0 {
        return Err(StackError::InvalidConfiguration(
            "Downsample factor too large for image dimensions".into(),
        ));
    }

    let inv_area = 1.0 / (factor * factor) as f32;

    let mut output = vec![0.0f32; dst_width * dst_height * channels];

    output
        .par_chunks_mut(dst_width * channels)
        .enumerate()
        .for_each(|(dst_y, row)| {
            let src_y_start = dst_y * factor;

            for dst_x in 0..dst_width {
                let src_x_start = dst_x * factor;

                for c in 0..channels {
                    let mut sum = 0.0f32;

                    for sy in 0..factor {
                        for sx in 0..factor {
                            sum += frame.get_pixel(src_x_start + sx, src_y_start + sy, c);
                        }
                    }

                    row[dst_x * channels + c] = sum * inv_area;
                }
            }
        });

    Frame::from_f32_vec(output, dst_width, dst_height, channels)
}
