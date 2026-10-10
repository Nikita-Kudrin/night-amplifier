//! The ladder's last rung: the shift every star pair agrees on, at a few small turns.
//!
//! Triangle matching needs its 30 brightest detections to be stars; a thin sub of a
//! bright object may hold a dozen. Voting does not care what the rest are: every
//! reference/target pair proposes its offset, pairs that are the same star propose the
//! same one, and anything else spreads over the whole search area.
//!
//! A shift alone stops agreeing once an alt-az field has turned ~0.3° (8 px at a 3008²
//! corner, the vote window's width), and near transit the Cat's Eye turns ~0.7° a
//! minute against the session's fixed reference. So the vote runs at a few turns around
//! the session's last one; the ladder keeps handling large turns.

use crate::detection::Star;

use super::transform::AffineTransform;

/// How far (px) a sub may have moved from the reference and still be found. A
/// drifting or nudged Dob wanders hundreds of pixels over a session; beyond this the
/// two frames share too little sky for a dozen stars to agree.
const SEARCH_PX: f32 = 512.0;

/// Vote bin size (px): several times the centroid noise of a faint star, so one
/// star's votes do not split across many bins.
const BIN_PX: f32 = 4.0;

/// Votes below which a peak is no shift at all. The fit is judged on its support
/// afterwards; this only stops a lone pair from being refined into a transform.
const MIN_VOTES: usize = 3;

const BINS: usize = (2.0 * SEARCH_PX / BIN_PX) as usize;

/// Spacing of the turns tried: 0.15°, so the nearest one is at most 0.075° off, 2.8 px
/// at a 3008² corner, and its votes still fall in one 2x2 window (8 px). Two neighbouring
/// turns often tie on votes, so the vote can be a step off (~4 px at the corners); the
/// refit after it settles the turn.
const TURN_STEP_RAD: f32 = 0.15 * std::f32::consts::PI / 180.0;

/// Turns tried either side of the expected one: ±0.6°, a minute of the fastest
/// rotation a session sees between two stacked subs, with room to spare.
const TURN_STEPS: i32 = 4;

/// The voting rung's fit.
pub(super) struct Vote {
    /// Target -> reference.
    pub transform: AffineTransform,
    /// Offsets voted over every turn tried: the hypotheses the peak was picked from,
    /// which its support has to beat (see `Support::searched`).
    pub trials: usize,
}

/// The turn and shift (target -> reference) most star pairs agree on, trying turns
/// around `expected_rotation` (radians), or `None` when no offset gathered
/// [`MIN_VOTES`].
pub(super) fn vote_translation(reference: &[Star], target: &[Star], expected_rotation: f32) -> Option<Vote> {
    let centre = centroid(target)?;
    let mut votes = vec![0u16; BINS * BINS];
    let mut trials = 0;
    let mut best: Option<(usize, f32, usize, usize)> = None;
    // Nearest turn first, so a tie keeps the one closest to the expected turn.
    for step in (0..=2 * TURN_STEPS).map(|i| if i % 2 == 0 { -i / 2 } else { (i + 1) / 2 }) {
        let rotation = expected_rotation + step as f32 * TURN_STEP_RAD;
        let turned = turn(target, rotation, centre);
        votes.fill(0);
        trials += for_each_offset(reference, &turned, |dx, dy| {
            let (bx, by) = (bin(dx), bin(dy));
            votes[by * BINS + bx] = votes[by * BINS + bx].saturating_add(1);
        });
        let (peak, bx, by) = peak_window(&votes);
        if best.is_none_or(|(held, ..)| peak > held) {
            best = Some((peak, rotation, bx, by));
        }
    }
    let (peak, rotation, best_x, best_y) = best?;
    if peak < MIN_VOTES {
        return None;
    }

    // The bins only locate the peak; its position is the mean of the offsets in it,
    // which lands within the refit's starting radius instead of a bin's width away.
    let turned = turn(target, rotation, centre);
    let (mut sum_x, mut sum_y, mut count) = (0.0f64, 0.0f64, 0usize);
    for_each_offset(reference, &turned, |dx, dy| {
        let (bx, by) = (bin(dx), bin(dy));
        if (best_x..=best_x + 1).contains(&bx) && (best_y..=best_y + 1).contains(&by) {
            sum_x += f64::from(dx);
            sum_y += f64::from(dy);
            count += 1;
        }
    });
    let (dx, dy) = ((sum_x / count as f64) as f32, (sum_y / count as f64) as f32);

    // turned = R (p - c) + c, so the whole map is R p + (c - R c + d).
    let (sin, cos) = rotation.sin_cos();
    let transform = AffineTransform::new(
        rotation,
        1.0,
        centre.0 - (cos * centre.0 - sin * centre.1) + dx,
        centre.1 - (sin * centre.0 + cos * centre.1) + dy,
    );
    Some(Vote { transform, trials })
}

/// The busiest 2x2 window of bins and its corner, so a shift that falls on a bin edge
/// is not split in half.
fn peak_window(votes: &[u16]) -> (usize, usize, usize) {
    let window = |bx: usize, by: usize| {
        usize::from(votes[by * BINS + bx])
            + usize::from(votes[by * BINS + bx + 1])
            + usize::from(votes[(by + 1) * BINS + bx])
            + usize::from(votes[(by + 1) * BINS + bx + 1])
    };
    let mut best = (0, 0, 0);
    for by in 0..BINS - 1 {
        for bx in 0..BINS - 1 {
            let count = window(bx, by);
            if count > best.0 {
                best = (count, bx, by);
            }
        }
    }
    best
}

fn centroid(stars: &[Star]) -> Option<(f32, f32)> {
    if stars.is_empty() {
        return None;
    }
    let n = stars.len() as f32;
    Some((stars.iter().map(|s| s.x).sum::<f32>() / n, stars.iter().map(|s| s.y).sum::<f32>() / n))
}

/// `stars` turned by `rotation` about `centre`.
fn turn(stars: &[Star], rotation: f32, (cx, cy): (f32, f32)) -> Vec<Star> {
    let (sin, cos) = rotation.sin_cos();
    stars
        .iter()
        .map(|s| {
            let (dx, dy) = (s.x - cx, s.y - cy);
            Star { x: cx + cos * dx - sin * dy, y: cy + sin * dx + cos * dy, ..*s }
        })
        .collect()
}

/// Calls `vote` with every reference-minus-target offset inside the search area, and
/// returns how many there were.
fn for_each_offset(reference: &[Star], target: &[Star], mut vote: impl FnMut(f32, f32)) -> usize {
    let mut offered = 0;
    for t in target {
        for r in reference {
            let (dx, dy) = (r.x - t.x, r.y - t.y);
            if dx.abs() < SEARCH_PX && dy.abs() < SEARCH_PX {
                vote(dx, dy);
                offered += 1;
            }
        }
    }
    offered
}

fn bin(offset: f32) -> usize {
    (((offset + SEARCH_PX) / BIN_PX) as usize).min(BINS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scatter(count: usize, size: f32, seed: u64) -> Vec<Star> {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as f32 / (1u64 << 31) as f32
        };
        (0..count).map(|_| Star::new(next() * size, next() * size, 1.0, 0.5, 30.0)).collect()
    }

    #[test]
    fn fifteen_stars_find_their_shift_among_noise() {
        let real = scatter(15, 3000.0, 1);
        let mut reference = real.clone();
        reference.extend(scatter(185, 3000.0, 2));
        let mut target: Vec<Star> =
            real.iter().map(|s| Star::new(s.x + 77.3, s.y - 41.6, 1.0, 0.5, 30.0)).collect();
        target.extend(scatter(185, 3000.0, 3));

        let shift = vote_translation(&reference, &target, 0.0).expect("the shift gathers votes").transform;

        assert!((shift.tx + 77.3).abs() < 0.5 && (shift.ty - 41.6).abs() < 0.5, "{shift:?}");
        assert_eq!((shift.rotation, shift.scale), (0.0, 1.0));
    }

    #[test]
    fn a_shift_on_a_bin_edge_is_found_whole() {
        let reference = scatter(40, 3000.0, 4);
        let target: Vec<Star> =
            reference.iter().map(|s| Star::new(s.x - BIN_PX * 3.0, s.y, 1.0, 0.5, 30.0)).collect();

        let shift = vote_translation(&reference, &target, 0.0).unwrap().transform;

        assert!((shift.tx - BIN_PX * 3.0).abs() < 0.01 && shift.ty.abs() < 0.01, "{shift:?}");
    }

    #[test]
    fn stars_out_of_reach_propose_nothing() {
        let reference = vec![Star::new(0.0, 0.0, 1.0, 0.5, 30.0)];
        let target = vec![Star::new(2000.0, 2000.0, 1.0, 0.5, 30.0)];

        assert!(vote_translation(&reference, &target, 0.0).is_none());
    }

    /// `stars` as a sub sees them when `truth` maps it onto the reference.
    fn observed(stars: &[Star], truth: &AffineTransform) -> Vec<Star> {
        stars
            .iter()
            .map(|s| {
                let (x, y) = truth.inverse_transform_point(s.x, s.y);
                Star::new(x, y, 1.0, 0.5, 30.0)
            })
            .collect()
    }

    /// Turned by `degrees` about the frame centre, then shifted.
    fn turned_about_the_centre(degrees: f32, (dx, dy): (f32, f32)) -> AffineTransform {
        let (theta, c) = (degrees.to_radians(), 1500.0);
        let (sin, cos) = theta.sin_cos();
        AffineTransform::new(theta, 1.0, c - (cos * c - sin * c) + dx, c - (sin * c + cos * c) + dy)
    }

    /// How far off at the corners a vote may land: a turn step, well inside the window.
    const VOTE_SLACK_PX: f32 = 5.0;

    fn worst_corner_error(a: &AffineTransform, b: &AffineTransform) -> f32 {
        [(0.0, 0.0), (3000.0, 0.0), (0.0, 3000.0), (3000.0, 3000.0)]
            .into_iter()
            .map(|(x, y)| {
                let (ax, ay) = a.transform_point(x, y);
                let (bx, by) = b.transform_point(x, y);
                (ax - bx).hypot(ay - by)
            })
            .fold(0.0, f32::max)
    }

    /// Half a degree is a minute of the Cat's Eye near transit: a shift alone no longer
    /// gathers the corners' votes, a turn grid does.
    #[test]
    fn a_turned_field_is_found_at_its_turn() {
        let real = scatter(15, 3000.0, 11);
        let truth = turned_about_the_centre(0.5, (-30.0, 7.0));
        let mut reference = real.clone();
        reference.extend(scatter(185, 3000.0, 12));
        let mut target = observed(&real, &truth);
        target.extend(scatter(185, 3000.0, 13));

        let vote = vote_translation(&reference, &target, 0.0).expect("the turn gathers votes");

        assert!(worst_corner_error(&vote.transform, &truth) < VOTE_SLACK_PX, "{:?}", vote.transform);
    }

    /// Past the grid's reach the expected turn has to carry it: a session tracked to 3°
    /// is searched around 3°, not around none.
    #[test]
    fn the_search_is_centred_on_the_expected_turn() {
        let real = scatter(40, 3000.0, 14);
        let truth = turned_about_the_centre(3.0, (12.0, -4.0));
        let target = observed(&real, &truth);

        let near = vote_translation(&real, &target, 2.9f32.to_radians()).expect("found near its turn");

        assert!(worst_corner_error(&near.transform, &truth) < VOTE_SLACK_PX, "{:?}", near.transform);
        let unprimed = vote_translation(&real, &target, 0.0);
        assert!(
            unprimed.is_none_or(|vote| worst_corner_error(&vote.transform, &truth) > 4.0 * VOTE_SLACK_PX),
            "3° is outside the unprimed grid"
        );
    }

    /// Every offset voted is a hypothesis the peak was chosen from, at every turn.
    #[test]
    fn the_trials_count_every_offset_at_every_turn() {
        let reference = vec![Star::new(0.0, 0.0, 1.0, 0.5, 30.0), Star::new(10.0, 0.0, 1.0, 0.5, 30.0)];
        let target = vec![Star::new(5.0, 5.0, 1.0, 0.5, 30.0), Star::new(5.0, 5.0, 1.0, 0.5, 30.0)];
        let turns = (2 * TURN_STEPS + 1) as usize;

        let vote = vote_translation(&reference, &target, 0.0);

        assert_eq!(vote.map(|v| v.trials), None, "two stars cannot gather three votes");
        let third = vec![Star::new(5.0, 5.0, 1.0, 0.5, 30.0); 3];
        assert_eq!(vote_translation(&reference, &third, 0.0).unwrap().trials, 2 * 3 * turns);
    }
}
