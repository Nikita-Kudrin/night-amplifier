//! Rayon work partitioning shared by the render kernels and the Pro plugins.

/// Elements per rayon task for a flat pass over a contiguous run.
///
/// Targets a few chunks per worker so work stealing still balances, with a floor so a
/// short run does not fan out into more tasks than the work is worth.
///
/// Deliberately imposes **no divisibility constraint** on `len`. The version this
/// replaced searched upward one integer at a time for a divisor of the length, so that
/// callers could recover a channel index from a flat chunk index. That cost 0.115 ms per
/// call on a 2712x1538 plane and 4.9 ms on a 1999x1999 one — and in the awkward cases it
/// gave up and returned `len`, i.e. one chunk and no parallelism at all, which made
/// `subtract_black_point` 4.8x more expensive per pixel there than on a sensor-shaped
/// frame. Callers that need per-channel behaviour dispatch per plane instead, which is
/// both cheaper and simpler.
pub fn balanced_chunk_len(len: usize) -> usize {
    if len == 0 {
        return 1;
    }
    let blocks = (rayon::current_num_threads() * 4).max(1);
    (len / blocks).max(8192).min(len)
}

#[cfg(test)]
mod tests {
    use super::balanced_chunk_len;

    #[test]
    fn short_runs_stay_in_one_chunk() {
        assert_eq!(balanced_chunk_len(0), 1);
        assert_eq!(balanced_chunk_len(1), 1);
        assert_eq!(balanced_chunk_len(4096), 4096);
    }

    /// The regression this function exists to prevent.
    ///
    /// `1021 * 1021` and `1999 * 1999` are prime squares, so their only divisors are 1,
    /// the root and the square itself. The divisor-searching predecessor returned the
    /// square — one chunk for the whole plane — after walking a million-plus modulo
    /// operations to get there.
    #[test]
    fn awkward_lengths_still_parallelise() {
        for len in [2712 * 1538, 1021 * 1021, 1999 * 1999, 3008 * 3008, 1289 * 1291] {
            let chunk = balanced_chunk_len(len);
            assert!(chunk <= len, "len {len}: chunk {chunk} exceeds the run");
            assert!(
                len / chunk > 1,
                "len {len} collapsed to a single chunk of {chunk}"
            );
        }
    }
}
