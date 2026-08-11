use rayon::prelude::*;
use wide::f32x4;

/// SIMD-optimized min/max computation
#[inline]
pub(crate) fn min_max_simd(values: &[f32]) -> (f32, f32) {
    if values.is_empty() {
        return (0.0, 0.0);
    }

    if values.len() < 8 {
        // Scalar for small arrays
        let mut min_val = values[0];
        let mut max_val = values[0];
        for &v in &values[1..] {
            min_val = min_val.min(v);
            max_val = max_val.max(v);
        }
        return (min_val, max_val);
    }

    // SIMD for larger arrays
    let chunks = values.chunks_exact(4);
    let remainder = chunks.remainder();

    let mut min_vec = f32x4::splat(f32::MAX);
    let mut max_vec = f32x4::splat(f32::MIN);

    for chunk in chunks {
        let v = f32x4::new([chunk[0], chunk[1], chunk[2], chunk[3]]);
        min_vec = min_vec.min(v);
        max_vec = max_vec.max(v);
    }

    // Reduce to scalars
    let min_arr = min_vec.to_array();
    let max_arr = max_vec.to_array();

    let mut min_val = min_arr[0].min(min_arr[1]).min(min_arr[2]).min(min_arr[3]);
    let mut max_val = max_arr[0].max(max_arr[1]).max(max_arr[2]).max(max_arr[3]);

    // Handle remainder
    for &v in remainder {
        min_val = min_val.min(v);
        max_val = max_val.max(v);
    }

    (min_val, max_val)
}

/// Compute absolute deviations in-place using SIMD
///
/// Transforms values[i] = |values[i] - median|
#[inline]
pub(crate) fn compute_mad_in_place_simd(values: &mut [f32], median: f32) {
    let len = values.len();

    if len < 4096 {
        // Sequential for small arrays
        if len < 8 {
            for v in values.iter_mut() {
                *v = (*v - median).abs();
            }
            return;
        }

        let median_vec = f32x4::splat(median);
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f32x4::new([
                values[idx],
                values[idx + 1],
                values[idx + 2],
                values[idx + 3],
            ]);
            let diff = v - median_vec;
            let abs_diff = diff.abs();
            let result = abs_diff.to_array();
            values[idx] = result[0];
            values[idx + 1] = result[1];
            values[idx + 2] = result[2];
            values[idx + 3] = result[3];
        }

        for v in values[chunks * 4..].iter_mut() {
            *v = (*v - median).abs();
        }
        return;
    }

    // Process in parallel chunks using SIMD
    let chunk_size = 4096;
    values.par_chunks_mut(chunk_size).for_each(|chunk| {
        let chunk_len = chunk.len();
        if chunk_len < 8 {
            for v in chunk.iter_mut() {
                *v = (*v - median).abs();
            }
            return;
        }

        let median_vec = f32x4::splat(median);
        let chunks = chunk_len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f32x4::new([
                chunk[idx],
                chunk[idx + 1],
                chunk[idx + 2],
                chunk[idx + 3],
            ]);
            let diff = v - median_vec;
            let abs_diff = diff.abs();
            let result = abs_diff.to_array();
            chunk[idx] = result[0];
            chunk[idx + 1] = result[1];
            chunk[idx + 2] = result[2];
            chunk[idx + 3] = result[3];
        }

        for v in chunk[chunks * 4..].iter_mut() {
            *v = (*v - median).abs();
        }
    });
}

/// Fast median computation using partial sort or parallel full sort
///
/// Uses `select_nth_unstable` for small arrays and Rayon's `par_sort_unstable_by` 
/// for large arrays to utilize multiple cores.
#[inline]
pub fn fast_median(values: &mut [f32]) -> f32 {
    let len = values.len();
    if len == 0 {
        return 0.0;
    }
    if len == 1 {
        return values[0];
    }

    let mid = len / 2;
    let compare = |a: &f32, b: &f32| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Greater);

    if len < 4096 {
        if len % 2 == 1 {
            // Odd length: return middle element
            values.select_nth_unstable_by(mid, compare);
            values[mid]
        } else {
            // Even length: return average of two middle elements
            values.select_nth_unstable_by(mid, compare);
            let upper = values[mid];
            let lower = values[..mid].iter().copied().fold(f32::MIN, f32::max);
            (lower + upper) / 2.0
        }
    } else {
        // Parallel full sort for large arrays
        values.par_sort_unstable_by(compare);
        if len % 2 == 1 {
            values[mid]
        } else {
            (values[mid - 1] + values[mid]) / 2.0
        }
    }
}
