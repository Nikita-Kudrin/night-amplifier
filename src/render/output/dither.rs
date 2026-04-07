use rayon::prelude::*;

/// Apply ordered (Bayer) dithering to reduce banding in gradients
///
/// Uses a 4x4 Bayer matrix for threshold modulation. This helps smooth
/// out visible banding in low-contrast gradients like sky backgrounds.
pub(crate) fn apply_ordered_dither(mut data: Vec<u8>, width: usize, _height: usize) -> Vec<u8> {
    const BAYER_4X4: [[i8; 4]; 4] = [
        [-8, 0, -6, 2],
        [4, -4, 6, -2],
        [-5, 3, -7, 1],
        [7, -1, 5, -3],
    ];

    data.par_chunks_mut(width * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let y_mod = y % 4;
            for x in 0..width {
                let x_mod = x % 4;
                let threshold = BAYER_4X4[y_mod][x_mod];

                for c in 0..3 {
                    let idx = x * 3 + c;
                    let val = row[idx] as i16 + threshold as i16;
                    row[idx] = val.clamp(0, 255) as u8;
                }
            }
        });

    data
}
