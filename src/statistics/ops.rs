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

    if len < 8 {
        // Scalar for small arrays
        for v in values.iter_mut() {
            *v = (*v - median).abs();
        }
        return;
    }

    // Process in chunks of 4 using SIMD
    let median_vec = f32x4::splat(median);

    // Safe SIMD processing of complete chunks
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

    // Handle remainder
    for v in values[chunks * 4..].iter_mut() {
        *v = (*v - median).abs();
    }
}

/// Fast median computation using partial sort
///
/// For median, we only need the middle element(s), not full sort.
/// Uses `select_nth_unstable` which is O(n) average case vs O(n log n) for full sort.
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

    // Handle NaN values by treating them as large values
    let compare = |a: &f32, b: &f32| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Greater);

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
}
