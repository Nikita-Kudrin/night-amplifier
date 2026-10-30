//! The sky around a planetary target, per channel, which the Moon measure (`super::target`)
//! takes off the target's colour: a sensor's pedestal times the blue gain once refused a Moon
//! at a fifth of full scale as haze, and a single black point biased its blue 1-2.5 %.
//!
//! Read on [`BLOCK`]-pixel blocks wholly below the target rather than on pixels: a block's mean
//! holds a sub's noise down 16x, so the spread between blocks is the sky's own unevenness (a
//! gradient, a glow towards the disc) and not read noise, which a pixel histogram showed as haze.

/// Side of a sky block, in the measured frame's pixels.
pub(super) const BLOCK: usize = 16;

/// Fewest whole sky blocks a sky is read from: under it (a frame-filling Moon) nothing is
/// taken off, as before the measure read a sky at all.
const MIN_BLOCKS: usize = 16;

/// The low quantile of the blocks' brightness the spread is read down to.
const LOW_QUANTILE: f32 = 0.05;

/// One block's sums over its pixels below the target.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Block {
    sums: [f32; 3],
    below: u32,
}

impl Block {
    pub(super) fn add(&mut self, rgb: [f32; 3]) {
        for (sum, v) in self.sums.iter_mut().zip(rgb) {
            *sum += v;
        }
        self.below += 1;
    }
}

/// The sky's level per channel and how uneven it is, both off the frame's whole sky blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Sky {
    /// Each channel's median over the blocks.
    pub level: [f32; 3],
    /// The blocks' median brightest channel.
    pub brightness: f32,
    /// That median over its [`LOW_QUANTILE`]: zero on a flat sky however noisy.
    pub spread: f32,
}

impl Sky {
    /// The sky of a `width` x `height` frame from its `blocks`, row-major, [`BLOCK`] pixels a
    /// side (the last row and column cut by the frame's edge); `None` under [`MIN_BLOCKS`].
    pub(super) fn of(blocks: &[Block], width: usize, height: usize) -> Option<Self> {
        let cols = width.div_ceil(BLOCK);
        let side = |i: usize, len: usize| BLOCK.min(len - i * BLOCK);
        let means: Vec<[f32; 3]> = blocks
            .iter()
            .enumerate()
            .filter(|(i, block)| block.below as usize == side(i % cols, width) * side(i / cols, height))
            .map(|(_, block)| block.sums.map(|s| s / block.below as f32))
            .collect();
        if means.len() < MIN_BLOCKS {
            return None;
        }
        let level = [0, 1, 2].map(|c| quantile(&mut means.iter().map(|m| m[c]).collect::<Vec<_>>(), 0.5));
        let mut brightness: Vec<f32> = means.iter().map(|m| m[0].max(m[1]).max(m[2])).collect();
        let median = quantile(&mut brightness, 0.5);
        let low = quantile(&mut brightness, LOW_QUANTILE);
        Some(Self { level, brightness: median, spread: median - low })
    }
}

fn quantile(values: &mut [f32], q: f32) -> f32 {
    let rank = ((values.len() - 1) as f32 * q).round() as usize;
    *values.select_nth_unstable_by(rank, f32::total_cmp).1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(width: usize, height: usize, level: impl Fn(usize) -> [f32; 3]) -> Vec<Block> {
        let cols = width.div_ceil(BLOCK);
        (0..cols * height.div_ceil(BLOCK))
            .map(|i| {
                let area = BLOCK.min(width - (i % cols) * BLOCK) * BLOCK.min(height - (i / cols) * BLOCK);
                let mut block = Block::default();
                (0..area).for_each(|_| block.add(level(i)));
                block
            })
            .collect()
    }

    #[test]
    fn a_flat_sky_reads_its_level_per_channel_and_no_spread() {
        let sky = Sky::of(&blocks(100, 90, |_| [0.006, 0.005, 0.0117]), 100, 90).unwrap();
        assert!(sky.level.iter().zip([0.006, 0.005, 0.0117]).all(|(a, b)| (a - b).abs() < 1e-6), "{sky:?}");
        assert!((sky.brightness - 0.0117).abs() < 1e-6 && sky.spread.abs() < 1e-6, "{sky:?}");
    }

    #[test]
    fn a_gradient_spreads_and_a_partly_covered_block_does_not_count() {
        let mut gradient = blocks(160, 160, |i| [0.01 + 0.001 * (i % 10) as f32; 3]);
        assert!(Sky::of(&gradient, 160, 160).unwrap().spread > 0.003);
        gradient.iter_mut().skip(MIN_BLOCKS - 1).for_each(|b| b.below -= 1);
        assert_eq!(Sky::of(&gradient, 160, 160).map(|s| s.level), None, "under the least whole blocks");
    }
}
