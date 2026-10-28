//! What a real session's live stack must hold to: its bumped subs, its clean subs and
//! its star size, replayed through the application (`instruments::Replay`).
//!
//! The bumped subs below were picked by eye from star stamps of every sub, not from the
//! detector's own score, so the detector cannot grade itself.

use std::path::Path;

use night_amplifier::registration::AffineTransform;
use night_amplifier::session::capture::pipeline::StackingOutcome;
use night_amplifier::Frame;
use serial_test::serial;

use crate::integration::common::FIXTURES_DIR;
use crate::integration::instruments::{half_flux_radii, measurable_stars, replayed, Replay, ReplayRecord, StarSize};

/// A deep-sky session and the subs in it (1-based) whose every star is visibly doubled
/// or smeared into separate images: the mount moved during the exposure.
struct Session {
    name: &'static str,
    bumped: &'static [usize],
}

const SESSIONS: &[Session] = &[
    // A Dobsonian on a rough night: a quarter of its subs are multi-image.
    Session { name: "250mm-dob-imx533-dumbbell-fits", bumped: &[1, 8, 10, 12, 16, 22, 23, 24, 27] },
    Session { name: "m101-pinwheel-galaxy-imx533", bumped: &[6, 7, 17, 24, 39] },
    Session { name: "m33-triangulum-galaxy-imx533", bumped: &[3, 35] },
    Session { name: "ic-59-ghost-of-cassiopeia-nebula", bumped: &[] },
    // 0.4 s subs, too short to catch a bump; 32 is smeared, not split.
    Session { name: "hercules-globular-cluster-imx533-fixture", bumped: &[] },
    // Its satellite trails (62, 63) leave the stars alone, so they are not bumps.
    Session { name: "iris-nebula-imx533", bumped: &[2, 6, 13, 15, 55, 61] },
];

/// Sessions the sharpness check runs over: the bump-listed ones plus the managed set
/// whose ladder fits were loosest (the 130 mm dumbbell's corners measured 19 % larger
/// before refinement). Not the 250 mm Orion: its nebula leaves ~10 isolated stars. Not M13:
/// its 0.4 s subs freeze seeing tilt, each star moving 0.61 px rms sub to sub against 0.09
/// px of fixed offset, which no transform per sub follows; on its 2.4 px stars that reads
/// +5 % stacked. `registration_keeps_what_a_least_squares_fit_would` judges it instead.
const SHARPNESS_SESSIONS: &[&str] = &[
    "250mm-dob-imx533-dumbbell-fits",
    "m101-pinwheel-galaxy-imx533",
    "m33-triangulum-galaxy-imx533",
    "ic-59-ghost-of-cassiopeia-nebula",
    "iris-nebula-imx533",
    "130mm-imx464-dumbell-nebulae-png",
];

/// Stars a zone needs before its median means anything.
const MIN_ZONE_STARS: usize = 10;

/// Share of a session's clean subs that must reach the stack. The gate may still drop a
/// soft or loosely fitted sub that looked fine in a stamp; it may not drop clean ones
/// wholesale.
const MIN_CLEAN_RETENTION: f64 = 0.9;

/// How much larger a stack's stars may be than the same stars' mean in its subs. A
/// misregistered sub smears every star it touches, corners first.
const MAX_STACK_STAR_GROWTH: f32 = 1.03;

/// A session the registration oracle judges, and whether its corners hold enough
/// isolated stars to judge (M13's hold 4).
struct OracleSession {
    name: &'static str,
    corners: bool,
}

/// The small-star sessions, where a registration error shows most. M13 is here because
/// its seeing keeps it out of `SHARPNESS_SESSIONS`; the oracle sees the same seeing.
const ORACLE_SESSIONS: &[OracleSession] = &[
    OracleSession { name: "hercules-globular-cluster-imx533-fixture", corners: false },
    OracleSession { name: "iris-nebula-imx533", corners: true },
    OracleSession { name: "ic-59-ghost-of-cassiopeia-nebula", corners: true },
];

/// How much larger a zone's stars may be stacked on production's transforms than on the
/// oracle's least-squares fit of the same subs. Measured 2026-10-26: -0.7 to +0.5 %; a
/// planted ±0.4 px per sub read +1.1 % (Iris centre) to +1.5 % (its corners).
const MAX_REGISTRATION_LOSS: f32 = 0.01;

fn replay(name: &str) -> Replay {
    Replay::open(&session_dir(name))
}

/// `name` played to the end, shared across the run.
fn played(name: &str) -> ReplayRecord {
    replayed(&session_dir(name))
}

fn session_dir(name: &str) -> std::path::PathBuf {
    crate::integration::common::ensure_fixtures_sync_named(&[name]);
    let dir = Path::new(FIXTURES_DIR).join(name);
    assert!(dir.is_dir(), "{}", crate::integration::common::missing_fixture_message(name));
    dir
}

/// A bump that leaves a sub unregistrable used to read as the telescope having been
/// moved, and Wanderer restarted the stack on each one: M101 lost its integration three
/// times over a session that never left the galaxy. No sub of these sessions — all
/// single-target — may read as movement.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_bumped_sub_does_not_read_as_the_sky_moving() {
    for session in SESSIONS {
        let outcomes = played(session.name).outcomes;
        let moved: Vec<String> = outcomes
            .iter()
            .enumerate()
            .filter_map(|(i, o)| {
                let reason = o.rejected_because()?;
                reason.means_the_sky_moved().then(|| format!("{} ({})", i + 1, reason.describe()))
            })
            .collect();
        let doubled = outcomes
            .iter()
            .filter(|o| o.rejected_because() == Some(night_amplifier::session::capture::RejectionReason::StarsDoubled))
            .count();
        println!("  {}: {doubled} refused as doubled, read as movement: {moved:?}", session.name);
        assert!(
            moved.is_empty(),
            "{}: subs {moved:?} would restart a Wanderer stack, but the telescope never moved",
            session.name
        );
    }
}

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn clean_subs_reach_the_stack() {
    for session in SESSIONS {
        let outcomes = played(session.name).outcomes;
        let clean: Vec<usize> =
            (1..=outcomes.len()).filter(|n| !session.bumped.contains(n)).collect();
        let refused: Vec<usize> = clean
            .iter()
            .copied()
            .filter(|&n| !outcomes[n - 1].frame_added)
            .collect();
        let retention = 1.0 - refused.len() as f64 / clean.len() as f64;
        println!(
            "  {}: {:.0}% of {} clean subs stacked, refused {refused:?}",
            session.name,
            retention * 100.0,
            clean.len()
        );
        assert!(
            retention >= MIN_CLEAN_RETENTION,
            "{}: refused clean subs {refused:?}",
            session.name
        );
    }
}

/// The stack must be as sharp as the subs it is made of, star for star.
///
/// Each of the stack's measurable stars is measured again in every sub that joined it, at
/// the position that sub's own registration put it: the same stars on both sides, so a
/// galaxy's knots that only the stack resolves cannot pass for misregistration (M101's
/// centre read 3.63 px stacked against 3.48 for its subs' own, different stars). A
/// perfectly registered stack's star is the average of its subs', so it is held to their
/// mean. Corners are checked on their own: a rotation error grows with distance from the
/// centre.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn the_stack_is_as_sharp_as_its_subs() {
    for &name in SHARPNESS_SESSIONS {
        // Pass one stacks, noting where each sub that ended up in the stack landed. Subs
        // are 108 MB each once demosaiced, so pass two replays them rather than keep them.
        let first = played(name);
        let landed = landed(&first.outcomes);
        let stack = first.stack.expect("a stack");
        let at = measurable_stars(&stack);
        let stacked = half_flux_radii(&stack, &at);

        let mut second = replay(name);
        let mut sums = vec![(0.0f32, 0usize); at.len()];
        let mut sub = 0;
        while let Some(frame) = second.capture() {
            if let Some((_, transform)) = landed.iter().find(|(n, _)| *n == sub) {
                let mapped: Vec<(f32, f32)> =
                    at.iter().map(|&(x, y)| transform.inverse_transform_point(x, y)).collect();
                for (sum, radius) in sums.iter_mut().zip(half_flux_radii(&frame, &mapped)) {
                    if let Some(radius) = radius {
                        *sum = (sum.0 + radius, sum.1 + 1);
                    }
                }
            }
            sub += 1;
        }

        // Only stars measured in the stack and in at least half the subs that made it.
        let (mut in_stack, mut in_subs) = (Vec::new(), Vec::new());
        for (stacked, &(sum, count)) in stacked.iter().zip(&sums) {
            let both = stacked.is_some() && 2 * count >= landed.len();
            in_stack.push(stacked.filter(|_| both));
            in_subs.push(both.then(|| sum / count as f32));
        }
        let (w, h) = (stack.width(), stack.height());
        let stack = StarSize::of(&in_stack, &at, w, h);
        let subs = StarSize::of(&in_subs, &at, w, h);
        println!(
            "  {name}: HFR centre {:.3} / corners {:.3} px stacked, {:.3} / {:.3} px mean of {} subs \
             ({} / {} stars)",
            stack.centre, stack.corners, subs.centre, subs.corners, landed.len(),
            stack.centre_stars, stack.corner_stars
        );
        let zones = [
            ("centre", stack.centre, subs.centre, stack.centre_stars),
            ("corner", stack.corners, subs.corners, stack.corner_stars),
        ];
        for (zone, stacked, subs, stars) in zones {
            assert!(stars >= MIN_ZONE_STARS, "{name}: only {stars} {zone} stars to judge");
            assert!(
                stacked <= subs * MAX_STACK_STAR_GROWTH,
                "{name}: {zone} stars grew from {subs:.3} px in the subs to {stacked:.3} px in the stack"
            );
        }
    }
}

/// Which subs the final stack holds, and where each landed (target -> reference).
fn landed(outcomes: &[StackingOutcome]) -> Vec<(usize, AffineTransform)> {
    let mut landed = Vec::new();
    for (sub, outcome) in outcomes.iter().enumerate() {
        match &outcome.admission {
            None if outcome.frame_added => landed = vec![(sub, AffineTransform::identity())],
            Some(a) if a.rebased => landed = vec![(sub, AffineTransform::identity())],
            Some(a) if a.added => landed.push((sub, a.transform.expect("a stacked sub landed somewhere"))),
            _ => {}
        }
    }
    landed
}

/// A star's position in a sub, and in the stack.
type Pair = ((f64, f64), (f64, f64));

/// The least-squares similarity taking each pair's sub position onto its stack position,
/// in closed form (2-D Umeyama), refit once without pairs past 3 sigma. Written apart
/// from `registration::refine` so the oracle cannot share a defect with what it judges.
fn oracle_fit(pairs: &[Pair]) -> AffineTransform {
    let fit = |pairs: &[&Pair]| {
        let n = pairs.len() as f64;
        let (mut p, mut q) = ((0.0, 0.0), (0.0, 0.0));
        for ((px, py), (qx, qy)) in pairs.iter().copied() {
            p = (p.0 + px / n, p.1 + py / n);
            q = (q.0 + qx / n, q.1 + qy / n);
        }
        let (mut a, mut b, mut norm) = (0.0, 0.0, 0.0);
        for ((px, py), (qx, qy)) in pairs.iter().copied() {
            let (px, py, qx, qy) = (px - p.0, py - p.1, qx - q.0, qy - q.1);
            a += px * qx + py * qy;
            b += px * qy - py * qx;
            norm += px * px + py * py;
        }
        let (rotation, scale) = (b.atan2(a), a.hypot(b) / norm);
        let (sin, cos) = rotation.sin_cos();
        let tx = q.0 - scale * (cos * p.0 - sin * p.1);
        let ty = q.1 - scale * (sin * p.0 + cos * p.1);
        AffineTransform::new(rotation as f32, scale as f32, tx as f32, ty as f32)
    };
    let residual = |t: &AffineTransform, ((px, py), (qx, qy)): &Pair| {
        let (x, y) = t.transform_point(*px as f32, *py as f32);
        (f64::from(x) - qx).hypot(f64::from(y) - qy)
    };
    let all: Vec<_> = pairs.iter().collect();
    let first = fit(&all);
    let sigma = (all.iter().map(|p| residual(&first, p).powi(2)).sum::<f64>() / all.len() as f64).sqrt();
    let kept: Vec<_> = all.into_iter().filter(|p| residual(&first, p) <= 3.0 * sigma).collect();
    fit(&kept)
}

/// A plain mean of warped subs, a sample counted only where its warp reached (as
/// `MasterStack` skips the border).
struct MeanStack {
    sum: Vec<f32>,
    count: Vec<u16>,
}

impl MeanStack {
    fn new(len: usize) -> Self {
        Self { sum: vec![0.0; len], count: vec![0; len] }
    }

    fn add(&mut self, frame: &Frame, transform: &AffineTransform) {
        let warped = night_amplifier::stacking::warp_frame(frame, transform, 0.0).expect("the sub warps");
        for ((sum, count), &v) in self.sum.iter_mut().zip(&mut self.count).zip(warped.data()) {
            if v.abs() >= 1e-6 {
                *sum += v;
                *count += 1;
            }
        }
    }

    fn frame(&self, like: &Frame) -> Frame {
        let mean = self.sum.iter().zip(&self.count).map(|(s, &n)| if n == 0 { 0.0 } else { s / f32::from(n) }).collect();
        Frame::from_f32_vec(mean, like.width(), like.height(), like.channels()).expect("same shape")
    }
}

/// The session's subs stacked twice by the test, on production's transforms and on the
/// oracle's, with the stack stars both are measured at and the median pairs per fit.
fn production_and_oracle_stacks(name: &str) -> (Frame, Frame, Vec<(f32, f32)>, usize) {
    let first = played(name);
    let landed = landed(&first.outcomes);
    let stack = first.stack.expect("a stack");
    let at = measurable_stars(&stack);
    let anchors: Vec<(f32, f32)> = night_amplifier::detection::detect_stars_adaptive(&stack)
        .unwrap_or_default()
        .iter()
        .filter(|s| s.peak < 0.7)
        .map(|s| (s.x, s.y))
        .collect();

    let (mut production, mut oracle) = (MeanStack::new(stack.data().len()), MeanStack::new(stack.data().len()));
    let mut pairs_per_fit = Vec::new();
    let mut second = replay(name);
    let mut sub = 0;
    while let Some(frame) = second.capture() {
        if let Some((_, transform)) = landed.iter().find(|(n, _)| *n == sub) {
            let found = night_amplifier::detection::detect_stars_adaptive(&frame).unwrap_or_default();
            let pairs: Vec<Pair> = anchors
                .iter()
                .filter_map(|&(x, y)| {
                    let (mx, my) = transform.inverse_transform_point(x, y);
                    let nearest = found
                        .iter()
                        .filter(|s| s.peak < 0.7)
                        .min_by(|a, b| (a.x - mx).hypot(a.y - my).total_cmp(&(b.x - mx).hypot(b.y - my)))?;
                    ((nearest.x - mx).hypot(nearest.y - my) < 3.0)
                        .then(|| ((f64::from(nearest.x), f64::from(nearest.y)), (f64::from(x), f64::from(y))))
                })
                .collect();
            assert!(pairs.len() >= 8, "{name}: sub {} paired only {} stars for the oracle", sub + 1, pairs.len());
            pairs_per_fit.push(pairs.len());
            production.add(&frame, transform);
            oracle.add(&frame, &oracle_fit(&pairs));
        }
        sub += 1;
    }
    pairs_per_fit.sort_unstable();
    (production.frame(&stack), oracle.frame(&stack), at, pairs_per_fit[pairs_per_fit.len() / 2])
}

/// Registration must keep what a least-squares fit over every star of the same subs
/// keeps. Both stacks are the test's own, the same subs, warp and plain mean on another
/// transform, so seeing, frame weights and interpolation cancel: what the sharpness test
/// cannot tell from M13's 0.61 px of seeing per sub, this can.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn registration_keeps_what_a_least_squares_fit_would() {
    for session in ORACLE_SESSIONS {
        let name = session.name;
        let (production, oracle, at, pairs) = production_and_oracle_stacks(name);
        let (w, h) = (production.width(), production.height());
        let size = |stack: &Frame| StarSize::of(&half_flux_radii(stack, &at), &at, w, h);
        let (production, oracle) = (size(&production), size(&oracle));
        println!(
            "  {name}: HFR centre {:.3} / corners {:.3} px on production's transforms, {:.3} / {:.3} on the \
             oracle's ({pairs} pairs a fit, {} / {} stars)",
            production.centre, production.corners, oracle.centre, oracle.corners,
            production.centre_stars, production.corner_stars
        );
        let mut zones = vec![("centre", production.centre, oracle.centre, production.centre_stars)];
        if session.corners {
            zones.push(("corner", production.corners, oracle.corners, production.corner_stars));
        }
        for (zone, ours, best, stars) in zones {
            assert!(stars >= MIN_ZONE_STARS, "{name}: only {stars} {zone} stars to judge");
            assert!(
                ours <= best * (1.0 + MAX_REGISTRATION_LOSS),
                "{name}: {zone} stars measure {ours:.3} px on production's transforms against {best:.3} px on \
                 a least-squares fit of the same subs"
            );
        }
    }
}

/// The oracle must be right before it judges anything.
#[test]
fn the_oracle_fit_recovers_a_known_similarity() {
    let truth = AffineTransform::new(3.1e-3, 1.0007, 41.25, -17.5);
    let mut pairs: Vec<Pair> = (0..60)
        .map(|i| {
            let p = ((i * 47 % 3000) as f32 + 0.3, (i * 89 % 3000) as f32 + 0.7);
            let q = truth.transform_point(p.0, p.1);
            ((f64::from(p.0), f64::from(p.1)), (f64::from(q.0), f64::from(q.1)))
        })
        .collect();
    pairs[7].1 .0 += 25.0; // a mismatched pair the clip must drop
    let fit = oracle_fit(&pairs);
    for (x, y) in [(0.0, 0.0), (3008.0, 3008.0), (0.0, 3008.0)] {
        let (a, b) = (fit.transform_point(x, y), truth.transform_point(x, y));
        assert!((a.0 - b.0).hypot(a.1 - b.1) < 0.01, "({x}, {y}): fit {a:?}, truth {b:?}");
    }
}
