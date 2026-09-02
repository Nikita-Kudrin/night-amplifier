//! Rayon work partitioning shared by the render kernels and the Pro plugins.

/// Elements per rayon task for a flat pass over a contiguous run.
///
/// Targets many chunks per worker so work stealing still balances, with a floor so a
/// short run does not fan out into more tasks than the work is worth.
///
/// # Why twelve chunks per worker and not four
///
/// The divisor is sized for asymmetric cores, not for x86. RK3588 pairs 4xA76 at
/// 2.4 GHz with 4xA55 at 1.8 GHz — roughly a 3x per-core spread once IPC is counted —
/// and the Pi 5's A76 cluster is uniform only by comparison. Four chunks per worker
/// hands each core a near-equal share and leaves the scheduler almost nothing to steal,
/// so the critical path ends on an A55 finishing a chunk the A76s could have taken. At
/// twelve the tail is small enough to migrate. On a symmetric machine the extra chunks
/// buy nothing but cost only their own bookkeeping, which the 8192 floor bounds.
///
/// Deliberately imposes **no divisibility constraint** on `len`. The version this
/// replaced searched upward one integer at a time for a divisor of the length, so that
/// callers could recover a channel index from a flat chunk index. That cost 0.115 ms per
/// call on a 2712x1538 plane and 4.9 ms on a 1999x1999 one — and in the awkward cases it
/// gave up and returned `len`, i.e. one chunk and no parallelism at all, which made
/// `subtract_black_point` 4.8x more expensive per pixel there than on a sensor-shaped
/// frame. Callers that need per-channel behaviour dispatch per plane instead, which is
/// both cheaper and simpler.
///
/// Callers that recover an absolute index as `block * chunk` are therefore correct for
/// any value this returns, and must stay that way: the result is a function of
/// `rayon::current_num_threads()`, so anything that depends on the exact chunk length
/// depends on the machine's core count. See
/// `render::denoise::tests::the_ycbcr_round_trip_is_invariant_to_thread_count`.
pub fn balanced_chunk_len(len: usize) -> usize {
    if len == 0 {
        return 1;
    }
    let blocks = (rayon::current_num_threads() * 12).max(1);
    (len / blocks).max(8192).min(len)
}

#[cfg(test)]
mod tests {
    use super::balanced_chunk_len;

    /// Pinned pool size, so the assertions describe the function rather than the
    /// machine CI happens to run on.
    fn with_threads<T: Send>(threads: usize, f: impl FnOnce() -> T + Send) -> T {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(f)
    }

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
        for len in [
            2712 * 1538,
            1021 * 1021,
            1999 * 1999,
            3008 * 3008,
            1289 * 1291,
        ] {
            let chunk = balanced_chunk_len(len);
            assert!(chunk <= len, "len {len}: chunk {chunk} exceeds the run");
            assert!(
                len / chunk > 1,
                "len {len} collapsed to a single chunk of {chunk}"
            );
        }
    }

    /// The property the divisor exists for: enough chunks that work stealing can move
    /// the tail off a slow core. On a 4+4 big.LITTLE part the A55s run roughly a third
    /// the speed of the A76s, so a split with only a few chunks per worker ends its
    /// critical path on a little core.
    #[test]
    fn a_sensor_plane_gives_the_scheduler_room_to_rebalance() {
        let len = 3008 * 3008;
        let threads = 4;
        let chunks = with_threads(threads, || len / balanced_chunk_len(len));
        assert!(
            chunks >= threads * 12,
            "{chunks} chunks across {threads} workers is too coarse to rebalance"
        );
    }

    /// The other edge of the same change: a higher divisor must not turn a short run
    /// into a swarm of tasks whose scheduling costs more than the arithmetic.
    #[test]
    fn the_floor_still_bounds_fan_out_on_short_runs() {
        assert_eq!(with_threads(16, || balanced_chunk_len(16_384)), 8192);
        assert_eq!(with_threads(16, || balanced_chunk_len(50_000)), 8192);
    }

    /// Chunk length has to track the pool it will be scheduled on — a fixed length
    /// would starve a wide machine and over-fragment a narrow one.
    #[test]
    fn chunk_length_shrinks_as_workers_multiply() {
        let len = 3008 * 3008;
        let narrow = with_threads(1, || balanced_chunk_len(len));
        let wide = with_threads(8, || balanced_chunk_len(len));
        assert!(
            wide < narrow,
            "8 workers got {wide}, 1 worker got {narrow} — the split ignores the pool"
        );
        assert!(narrow <= len);
    }
}
