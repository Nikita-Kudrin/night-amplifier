//! Doubled star images: a mount bump or a trail splits every star of a sub the same way.
//!
//! A bump mid-exposure leaves each star as two images a fixed vector apart; a trail splits
//! into several detections along one direction. Each image is as sharp as a clean star, so
//! FWHM cannot see either. What gives them away is that the companion offsets *agree*: a
//! real double star, or a chance neighbour in a crowded field, points anywhere.

use super::Star;

/// Brightest stars judged: bright enough that their companions are detected too.
const TOP_STARS: usize = 30;

/// Stars searched for companions. Wider than [`TOP_STARS`], since a bump's second image
/// is fainter than the first and falls down the flux ranking.
const CANDIDATES: usize = 120;

/// Companion separation, in pixels. Below 3 px a split image is one detection; above
/// 20 px the sub no longer registers and is a different verdict anyway.
const SEPARATION: std::ops::Range<f32> = 3.0..20.0;

/// How unequal a pair may be and still be one star twice: a bump spends anywhere from a
/// fifth to all of the exposure in its second position.
const FLUX_RATIO: f32 = 5.0;

/// Offsets this close (px) are the same displacement, within centroid noise on both ends.
const VECTOR_TOLERANCE: f32 = 1.5;

/// Other stars that must share an offset before it counts as the sub's, not a chance
/// pairing. At 2, sparse sessions (the 130 mm ring) score 0–13 % and the visibly bumped
/// dumbbell subs 60–100 %, but a dense field (the 250 mm Orion) scores 43–63 % on every
/// sub — hence the gate reads the share against the session's own baseline.
const MIN_SHARERS: usize = 2;

/// Share of the brightest stars (0..=1) whose companion offset at least [`MIN_SHARERS`]
/// other bright stars repeat.
///
/// Absolute values depend on the field — a crowded one scores ~50 % on every sub — so
/// read it against the same session's other subs, not against a constant.
pub fn doubled_star_share(stars: &[Star]) -> f32 {
    let top = stars.len().min(TOP_STARS);
    if top == 0 {
        return 0.0;
    }
    let offsets: Vec<Vec<(f32, f32)>> = stars[..top]
        .iter()
        .map(|star| companion_offsets(star, &stars[..stars.len().min(CANDIDATES)]))
        .collect();

    let shared = (0..top)
        .filter(|&i| {
            offsets[i].iter().any(|v| {
                let sharers = (0..top)
                    .filter(|&j| j != i)
                    .filter(|&j| offsets[j].iter().any(|w| (v.0 - w.0).hypot(v.1 - w.1) < VECTOR_TOLERANCE))
                    .count();
                sharers >= MIN_SHARERS
            })
        })
        .count();
    shared as f32 / top as f32
}

/// The offset most bright stars repeat between themselves and a companion — a bump's
/// displacement — or `None` when none is shared by more than [`MIN_SHARERS`] stars.
pub fn dominant_companion_offset(stars: &[Star]) -> Option<(f32, f32)> {
    let top = stars.len().min(TOP_STARS);
    let offsets: Vec<(f32, f32)> = stars[..top]
        .iter()
        .flat_map(|star| companion_offsets(star, &stars[..stars.len().min(CANDIDATES)]))
        .collect();
    let agreeing = |v: &(f32, f32)| {
        offsets.iter().filter(|w| (v.0 - w.0).hypot(v.1 - w.1) < VECTOR_TOLERANCE).count()
    };
    // Each pair among the top stars is found from both ends, hence the doubled bar.
    offsets
        .iter()
        .map(|v| (*v, agreeing(v)))
        .max_by_key(|&(_, n)| n)
        .filter(|&(_, n)| n > 2 * MIN_SHARERS)
        .map(|(v, _)| v)
}

/// `stars` with the fainter image of every pair `offset` apart dropped: the field as a
/// single, unbumped exposure would have shown it. Order is kept.
pub fn collapse_doubles(stars: &[Star], offset: (f32, f32)) -> Vec<Star> {
    let mut dropped = vec![false; stars.len()];
    for (i, star) in stars.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        for sign in [1.0, -1.0] {
            let (x, y) = (star.x + sign * offset.0, star.y + sign * offset.1);
            let twin = stars.iter().enumerate().find(|&(j, other)| {
                j != i && !dropped[j] && (other.x - x).hypot(other.y - y) < VECTOR_TOLERANCE
            });
            if let Some((j, other)) = twin {
                let fainter = if other.flux <= star.flux { j } else { i };
                dropped[fainter] = true;
            }
        }
    }
    stars.iter().zip(&dropped).filter(|(_, &d)| !d).map(|(s, _)| *s).collect()
}

/// Offsets to `star`'s plausible second images, folded onto one half-plane so a pair
/// reads the same from either end.
fn companion_offsets(star: &Star, candidates: &[Star]) -> Vec<(f32, f32)> {
    candidates
        .iter()
        .filter(|other| {
            SEPARATION.contains(&star.distance_to(other))
                && other.flux > star.flux / FLUX_RATIO
                && other.flux < star.flux * FLUX_RATIO
        })
        .map(|other| {
            let (dx, dy) = (other.x - star.x, other.y - star.y);
            if dx < 0.0 || (dx == 0.0 && dy < 0.0) {
                (-dx, -dy)
            } else {
                (dx, dy)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `count` stars scattered over `size`², brightest first, from a fixed seed. Fluxes span
    /// three decades log-uniformly, as a real field's steep brightness distribution does.
    fn field(count: usize, size: f32, seed: u64) -> Vec<Star> {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as f32 / (1u64 << 31) as f32
        };
        let mut stars: Vec<Star> = (0..count)
            .map(|_| Star::new(next() * size, next() * size, (7.0 * next()).exp(), 0.5, 20.0))
            .collect();
        stars.sort_by(|a, b| b.flux.total_cmp(&a.flux));
        stars
    }

    /// Every star again, `(dx, dy)` away at `ratio` of its flux, re-sorted as detection would.
    fn bumped(stars: &[Star], dx: f32, dy: f32, ratio: f32) -> Vec<Star> {
        let mut out: Vec<Star> = stars
            .iter()
            .flat_map(|s| [*s, Star::new(s.x + dx, s.y + dy, s.flux * ratio, 0.5, 20.0)])
            .collect();
        out.sort_by(|a, b| b.flux.total_cmp(&a.flux));
        out
    }

    #[test]
    fn a_clean_field_has_no_doubled_stars() {
        assert!(doubled_star_share(&field(150, 3000.0, 7)) < 0.05);
    }

    #[test]
    fn a_bumped_sub_doubles_most_of_its_stars() {
        let share = doubled_star_share(&bumped(&field(150, 3000.0, 7), 6.0, -5.0, 0.6));
        assert!(share > 0.8, "{share}");
    }

    /// A crowded field has a chance neighbour beside most bright stars, at random
    /// offsets. The score must stay far below a bumped sub's, though above a sparse
    /// field's — which is why the gate reads it against the session's own baseline.
    #[test]
    fn a_crowded_field_is_not_a_doubled_one() {
        let crowded = field(400, 800.0, 11);
        let share = doubled_star_share(&crowded);
        let bumped = doubled_star_share(&bumped(&crowded, 6.0, -5.0, 0.6));
        assert!(share < 0.4, "{share}");
        assert!(bumped > share + 0.5, "crowded {share}, bumped {bumped}");
    }

    #[test]
    fn a_bump_is_found_and_undone() {
        let clean = field(150, 3000.0, 7);
        let bumped = bumped(&clean, 6.0, -5.0, 0.6);

        let (dx, dy) = dominant_companion_offset(&bumped).expect("a shared offset");
        // Folded onto one half-plane: the bump reads as (6, -5) from either image.
        assert!((dx - 6.0).abs() < 0.5 && (dy + 5.0).abs() < 0.5, "({dx}, {dy})");

        let single = collapse_doubles(&bumped, (dx, dy));
        assert_eq!(single.len(), clean.len());
        assert!(single.iter().all(|s| clean.contains(s)), "the brighter image of each pair stays");
    }

    #[test]
    fn a_clean_field_has_no_dominant_offset() {
        assert_eq!(dominant_companion_offset(&field(150, 3000.0, 7)), None);
    }

    /// `bumped`, with each second image's centroid off by up to ±`noise` px on x, as a
    /// real detection's would be.
    fn bumped_noisy(stars: &[Star], dx: f32, dy: f32, ratio: f32, noise: f32) -> Vec<Star> {
        let mut out: Vec<Star> = stars
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                let jitter = (((i * 7919) % 101) as f32 / 50.0 - 1.0) * noise;
                [*s, Star::new(s.x + dx + jitter, s.y + dy, s.flux * ratio, 0.5, 20.0)]
            })
            .collect();
        out.sort_by(|a, b| b.flux.total_cmp(&a.flux));
        out
    }

    /// A bump along one image axis — a Dec bump with the camera square to the mount, the
    /// commonest kind — puts the offsets' x across zero, where the half-plane fold flips
    /// them. Noisy centroids must still read as one shared offset.
    #[test]
    fn a_bump_along_the_y_axis_is_still_one_offset() {
        let clean = field(150, 3000.0, 7);
        let diagonal = doubled_star_share(&bumped_noisy(&clean, 6.0, -5.0, 0.6, 0.4));
        let vertical = bumped_noisy(&clean, 0.0, 6.0, 0.6, 0.4);
        let share = doubled_star_share(&vertical);

        assert!(diagonal > 0.8, "control: {diagonal}");
        assert!(share > 0.8, "vertical {share} against diagonal {diagonal}");
        let offset = dominant_companion_offset(&vertical).expect("a shared offset");
        assert_eq!(collapse_doubles(&vertical, offset).len(), clean.len(), "{offset:?}");
    }

    #[test]
    fn no_stars_is_no_doubling() {
        assert_eq!(doubled_star_share(&[]), 0.0);
    }
}
