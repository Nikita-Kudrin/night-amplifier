//! The live stack over real sessions, replayed through the application.
//!
//! Every test here goes through `instruments::Replay`, i.e. the simulated camera,
//! `convert_captured_frame` and `stack_frame`: what they judge is the verdict a real
//! session would get, not one from a re-wired copy of the pipeline.

use std::path::Path;

use serial_test::serial;

use crate::integration::common::{
    FIXTURES_DIR, MANAGED_FIXTURE_SETS, MAX_REBASES_PER_SESSION, MAX_RESIDUAL_REJECTION_SHARE,
    MAX_WANDERER_RESET_SHARE, MIN_LIVE_STACKING_RETENTION,
};
use crate::integration::instruments::Replay;

/// A managed set replayed from the start, or `None` when this machine does not have it.
fn managed_replay(name: &str) -> Option<Replay> {
    let dir = Path::new(FIXTURES_DIR).join(name);
    dir.is_dir().then(|| Replay::open(&dir))
}

/// Drives the live stack over the managed fixture sets and holds the line on two numbers
/// that regressed together: how many frames survive registration, and how well the
/// survivors align.
///
/// Restricted to `MANAGED_FIXTURE_SETS` since `tests/fixtures/` is gitignored and may hold
/// stray capture output on any given machine. Registration used to run on the 30 stars
/// `DetectionConfig::fast()` returned, stacking every transform regardless of fit — 6 px
/// residuals included, which is what smeared the result.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn live_stacking_keeps_frames_and_aligns_them_well() {
    crate::integration::common::ensure_fixtures_sync();

    let mut sets_checked = 0;
    for name in MANAGED_FIXTURE_SETS {
        let Some(mut replay) = managed_replay(name) else {
            continue;
        };
        let outcomes = replay.run();

        let mut accepted = Vec::new();
        let mut dropped_on_alignment = Vec::new();
        let rebases = outcomes.iter().filter(|o| o.stack_reset).count();
        for admission in outcomes.iter().filter_map(|o| o.admission.as_ref()) {
            if !admission.mean_residual.is_finite() {
                continue; // never registered, so it has no fit to judge
            }
            match admission.rejected_because {
                None => accepted.push(admission.mean_residual),
                // A frame can align perfectly and still be dropped for bloated
                // stars, so only the alignment verdicts belong in this
                // comparison.
                Some(reason) if reason.is_about_alignment_quality() => {
                    dropped_on_alignment.push(admission.mean_residual)
                }
                Some(_) => {}
            }
        }

        // Retention is what ended up *in the stack*, not how many admissions
        // came back `added`. A re-base is an admission that throws away every
        // frame before it, so counting admissions would score a gate that
        // re-based on every frame as perfect.
        let integrated = replay.depth();
        let rate = integrated as f64 / replay.subs() as f64;
        println!(
            "  {name}: {integrated}/{} frames integrated ({:.0}%), {} dropped on alignment, {rebases} re-base(s)",
            replay.subs(),
            rate * 100.0,
            dropped_on_alignment.len()
        );

        assert!(
            rate >= MIN_LIVE_STACKING_RETENTION,
            "{name}: only {:.0}% of frames reached the stack, expected at least {:.0}%",
            rate * 100.0,
            MIN_LIVE_STACKING_RETENTION * 100.0
        );

        assert!(
            rebases <= MAX_REBASES_PER_SESSION,
            "{name}: re-based {rebases} times — each one discards the integration \
             built so far and drops the preview back to a single sub, so this is \
             the gate chasing noise in the FWHM estimate"
        );

        // The gate decides online, against a rolling median that moves as the
        // night goes on, so no fixed threshold describes its verdicts after the
        // fact and no strict per-frame ordering is guaranteed. What must hold is
        // the distributional claim the gate exists to make: the frames it kept
        // align better than the ones it rejected for aligning badly. If that
        // fails, it is discarding integration without buying sharpness.
        if !dropped_on_alignment.is_empty() {
            let kept = median_of(&accepted);
            let dropped = median_of(&dropped_on_alignment);
            assert!(
                kept < dropped,
                "{name}: kept frames align at {kept:.2} px, dropped ones at {dropped:.2} px — \
                 the gate is not improving the stack"
            );
        }

        sets_checked += 1;
    }

    assert!(sets_checked > 0, "No fixture sets were checked");
}

fn median_of(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

/// A rig that tracks well must not end up with the strictest gate.
///
/// `RESIDUAL_K * median_residual` on its own is scale-multiplicative, so the
/// tighter a session's own scatter the tighter its limit: the 250 mm dumbbell
/// fixture holds a ~0.6 px median residual on ~5.4 px stars, and that rule
/// allowed only 1.8 px and dropped 9 of its 34 frames for residuals of 1.9–3.3 px
/// — a fraction of one star's width. Misalignment only matters against the width
/// of what it is smearing, so the limit carries a floor tied to star size.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_well_tracked_session_is_not_punished_for_its_own_precision() {
    use night_amplifier::session::capture::RejectionReason;

    crate::integration::common::ensure_fixtures_sync();

    let mut sets_checked = 0;

    for name in MANAGED_FIXTURE_SETS {
        let Some(mut replay) = managed_replay(name) else {
            continue;
        };
        let outcomes = replay.run();

        let mut residuals = Vec::new();
        let mut residual_rejections = 0;
        for admission in outcomes.iter().filter_map(|o| o.admission.as_ref()) {
            if admission.rejected_because == Some(RejectionReason::ResidualTooHigh) {
                residual_rejections += 1;
            }
            if admission.mean_residual.is_finite() {
                residuals.push(admission.mean_residual);
            }
        }
        if residuals.is_empty() {
            continue;
        }
        let fwhm_of_stack = night_amplifier::detection::detect_stars_adaptive(&replay.snapshot())
            .ok()
            .and_then(|stars| night_amplifier::detection::compute_median_fwhm(&stars));

        let median_residual = median_of(&residuals);
        let Some(star_width) = fwhm_of_stack else {
            continue;
        };
        let offered = replay.subs() - 1;
        let share = residual_rejections as f64 / offered as f64;

        println!(
            "  {name}: median residual {median_residual:.2} px on {star_width:.2} px stars, \
             {residual_rejections}/{offered} dropped on residual ({:.0}%)",
            share * 100.0
        );

        // Only sessions that are actually tracking well make the claim; a set
        // whose residuals genuinely approach its star width should still lose
        // frames.
        if median_residual < 0.5 * star_width {
            assert!(
                share <= MAX_RESIDUAL_REJECTION_SHARE,
                "{name}: aligns to {median_residual:.2} px on {star_width:.2} px stars — \
                 well inside one star — yet {:.0}% of frames were dropped for a high \
                 residual. The gate is scoring precision against itself again.",
                share * 100.0
            );
            sets_checked += 1;
        }
    }

    assert!(
        sets_checked > 0,
        "no well-tracked fixture set was available to check"
    );
}

/// Wanderer mode restarts the stack when a frame cannot be placed against the
/// reference — the user having swung the telescope to a new object.
///
/// The gate widened "did not stack" well past that: it also rejects frames that
/// aligned perfectly and were merely soft or loose. Wanderer read every rejection
/// as movement, so a passing cloud restarted the integration. Checked over the
/// real fixture sets because it is the pipeline, not the classifier, that decides
/// which verdict a given frame gets.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn wanderer_holds_the_stack_through_the_frames_a_session_dislikes() {
    crate::integration::common::ensure_fixtures_sync();

    let mut sets_checked = 0;

    for name in MANAGED_FIXTURE_SETS {
        let Some(mut replay) = managed_replay(name) else {
            continue;
        };

        let mut would_reset = 0;
        let mut quality_rejections = 0;
        for outcome in replay.run() {
            let Some(reason) = outcome.rejected_because() else {
                continue;
            };
            if reason.means_the_sky_moved() {
                would_reset += 1;
                continue;
            }
            quality_rejections += 1;
        }

        let offered = replay.subs() - 1;
        let reset_share = would_reset as f64 / offered as f64;
        println!(
            "  {name}: {would_reset}/{offered} frames would reset the stack ({:.0}%), \
             {quality_rejections} soft frames held it",
            reset_share * 100.0
        );

        // A frame that genuinely will not register has always meant movement and
        // still does. What must not happen is Wanderer restarting for most of a
        // session that is simply having a rough night.
        assert!(
            reset_share <= MAX_WANDERER_RESET_SHARE,
            "{name}: Wanderer would restart the stack on {:.0}% of a real session — \
             quality verdicts are being read as the telescope having been moved",
            reset_share * 100.0
        );
        sets_checked += 1;
    }

    assert!(sets_checked > 0, "No fixture sets were checked");
}

/// The other half: a genuinely different sky must still restart the stack, or
/// Wanderer does not work at all.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn wanderer_reads_a_new_target_as_movement() {
    crate::integration::common::ensure_fixtures_sync();

    let mut pairs_checked = 0;

    // Sets sharing a sensor, so a frame from one can stand in for the telescope
    // having been swung to the other.
    let pairings = [
        (
            "130mm-imx464-dumbell-nebulae-png",
            "130mm-imx464-ring-nebulae-png",
        ),
        (
            "130mm-imx464-ring-nebulae-png",
            "250mm-dob-imx464-orion-png",
        ),
        // A thin sub swung in, which the ladder may hand to translation voting: a
        // voting rung that found a "shift" between any two fields would stack it. (The
        // other way round the dumbbell's stars read as bloated against the Cat's Eye's
        // noise-sized ones, a soft verdict until `BLOATED_RUN_LIMIT`.)
        (
            "250mm-dob-imx533-dumbbell-fits",
            "cats-eye-nebula-imx533",
        ),
    ];

    for (home, elsewhere) in pairings {
        let (Some(mut replay), Some(mut other)) = (managed_replay(home), managed_replay(elsewhere))
        else {
            continue;
        };
        // Build a real stack first, so the verdict is reached the way it would be
        // mid-session rather than against a cold gate.
        let reference = replay.capture().expect("a first sub");
        replay.stack(&reference, false);
        for _ in 0..6 {
            replay.step();
        }
        let swung = other.capture().expect("a first sub");
        if (swung.width(), swung.height()) != (reference.width(), reference.height()) {
            continue;
        }

        let outcome = replay.stack(&swung, false);
        let reason = outcome.rejected_because().unwrap_or_else(|| {
            panic!("{home} -> {elsewhere}: a completely different field was stacked")
        });
        assert!(
            reason.means_the_sky_moved(),
            "{home} -> {elsewhere}: a completely different field must reset the stack, \
             got {}",
            reason.describe()
        );

        println!(
            "  {home} -> {elsewhere}: reads as movement ({})",
            reason.describe()
        );
        pairs_checked += 1;
    }

    assert!(pairs_checked > 0, "no fixture pairing was available to check");
}

/// Thin subs of a bright planetary nebula still stack, and stack where they belong.
///
/// The Cat's Eye set (0.7 s at gain 0, IMX533 behind a 250 mm Dob) holds ~15 real stars
/// a sub, while noise on the nebula shatters it into ~45 local maxima that outshine
/// them: 27 of the 30 stars the ladder's first rung fits from were nebula, every fit was
/// a coincidence (rotations up to 2.9 rad), and the gate refused every sub after the
/// reference. The session drifts ~4 px a sub (~120 px over the set) and never turns, so a
/// sub that stacks must have landed on a near-translation — a loosened gate admitting
/// the coincidences fails here too.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn thin_subs_of_a_bright_nebula_still_stack() {
    const SET: &str = "cats-eye-nebula-imx533";
    crate::integration::common::ensure_fixtures_sync_named(&[SET]);
    let mut replay = managed_replay(SET)
        .unwrap_or_else(|| panic!("{}", crate::integration::common::missing_fixture_message(SET)));

    let outcomes = replay.run();
    for (sub, admission) in outcomes.iter().enumerate().filter_map(|(i, o)| Some((i + 1, o.admission?))) {
        let Some(transform) = admission.transform else {
            continue;
        };
        assert!(
            transform.rotation.abs() < 0.01 && (transform.scale - 1.0).abs() < 0.01,
            "sub {sub} stacked on rotation {:.4} rad, scale {:.4}: a session that only drifts \
             was aligned by a coincidental fit",
            transform.rotation,
            transform.scale
        );
    }

    let rate = replay.depth() as f64 / replay.subs() as f64;
    println!("  {SET}: {}/{} subs integrated ({:.0}%)", replay.depth(), replay.subs(), rate * 100.0);
    assert!(
        rate >= MIN_LIVE_STACKING_RETENTION,
        "{SET}: only {:.0}% of subs reached the stack, expected at least {:.0}%",
        rate * 100.0,
        MIN_LIVE_STACKING_RETENTION * 100.0
    );
}

/// The Cat's Eye's thin subs keep stacking while the field rotates, as it does on the
/// alt-az Dob that took them: near transit at 55°N the Cat's Eye turns ~0.7° a minute
/// against the session's fixed reference. Each sub is turned 0.03° further than the last
/// (0.78° by the end, a minute or more of a real session), plus a half-pixel shift so
/// the unrotated control interpolates too: unrotated, every sub stacks.
///
/// Translation voting cannot follow a turn, and the subs only it registers (4 of 26)
/// then read as the telescope having moved — each one a stack restart in Wanderer mode.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn thin_subs_hold_the_stack_while_the_field_rotates() {
    use night_amplifier::registration::AffineTransform;
    const SET: &str = "cats-eye-nebula-imx533";
    const TURN_PER_SUB_DEG: f32 = 0.03;
    crate::integration::common::ensure_fixtures_sync_named(&[SET]);
    let mut replay = managed_replay(SET)
        .unwrap_or_else(|| panic!("{}", crate::integration::common::missing_fixture_message(SET)));

    let mut moved = Vec::new();
    let mut sub = 0;
    while let Some(frame) = replay.capture() {
        let (sin, cos) = (TURN_PER_SUB_DEG * sub as f32).to_radians().sin_cos();
        let (cx, cy) = (frame.width() as f32 / 2.0, frame.height() as f32 / 2.0);
        let turn = AffineTransform::new(
            (TURN_PER_SUB_DEG * sub as f32).to_radians(),
            1.0,
            cx - (cos * cx - sin * cy) + 0.5,
            cy - (sin * cx + cos * cy) + 0.5,
        );
        let turned = night_amplifier::stacking::warp_frame(&frame, &turn, 0.0).expect("the sub warps");
        let outcome = replay.stack(&turned, false);
        if outcome.rejected_because().is_some_and(|reason| reason.means_the_sky_moved()) {
            moved.push(sub);
        }
        sub += 1;
    }

    println!("  {SET} turning {TURN_PER_SUB_DEG}°/sub: {}/{} subs stacked", replay.depth(), replay.subs());
    assert!(
        moved.is_empty(),
        "subs {moved:?} read as movement once the field had turned — Wanderer would restart the stack on each"
    );
}
