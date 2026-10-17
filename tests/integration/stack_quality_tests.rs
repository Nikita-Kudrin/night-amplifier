//! What a real session's live stack must hold to: its bumped subs, its clean subs and
//! its star size, replayed through the application (`instruments::Replay`).
//!
//! The bumped subs below were picked by eye from star stamps of every sub, not from the
//! detector's own score, so the detector cannot grade itself.

use std::path::Path;

use serial_test::serial;

use crate::integration::common::FIXTURES_DIR;
use crate::integration::instruments::{half_flux_radii, measurable_stars, Replay, StarSize};

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
];

/// Sessions the sharpness check runs over: the bump-listed ones plus the managed set
/// whose ladder fits were loosest (the 130 mm dumbbell's corners measured 19 % larger
/// before refinement). Not the 250 mm Orion: its nebula leaves ~10 isolated stars.
const SHARPNESS_SESSIONS: &[&str] = &[
    "250mm-dob-imx533-dumbbell-fits",
    "m101-pinwheel-galaxy-imx533",
    "m33-triangulum-galaxy-imx533",
    "ic-59-ghost-of-cassiopeia-nebula",
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

fn replay(name: &str) -> Replay {
    crate::integration::common::ensure_fixtures_sync_named(&[name]);
    let dir = Path::new(FIXTURES_DIR).join(name);
    assert!(dir.is_dir(), "{}", crate::integration::common::missing_fixture_message(name));
    Replay::open(&dir)
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
        let outcomes = replay(session.name).run();
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
        let outcomes = replay(session.name).run();
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
    use night_amplifier::registration::AffineTransform;

    for &name in SHARPNESS_SESSIONS {
        // Pass one stacks, noting where each sub that ended up in the stack landed. Subs
        // are 108 MB each once demosaiced, so pass two replays them rather than keep them.
        let mut first = replay(name);
        let mut landed: Vec<(usize, AffineTransform)> = Vec::new();
        for (sub, outcome) in first.run().iter().enumerate() {
            match &outcome.admission {
                None if outcome.frame_added => landed = vec![(sub, AffineTransform::identity())],
                Some(a) if a.rebased => landed = vec![(sub, AffineTransform::identity())],
                Some(a) if a.added => landed.push((sub, a.transform.expect("a stacked sub landed somewhere"))),
                _ => {}
            }
        }
        let stack = first.snapshot();
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
