//! The frame gate's verdicts, one behaviour at a time.

use super::*;
use night_amplifier_core::registration::AffineTransform;

/// Fills the gate's history so it is past warm-up, with residuals centred on
/// `residual` and star sizes on `fwhm`.
fn seeded(residual: f32, fwhm: f32) -> FrameGate {
    let mut gate = FrameGate::default();
    for _ in 0..WARMUP_FRAMES + 3 {
        gate.history.record(residual, Some(fwhm));
    }
    gate
}

/// A clean sparse field's doubled-star share.
const CLEAN: f32 = 0.05;

/// A sub whose collapsed stars register against the reference: this field, bumped.
const THIS_FIELD: fn() -> bool = || true;

/// 200 stars of `fwhm`, as clean as the session's usual sub.
fn stars(fwhm: f32) -> StarField {
    StarField { count: 200, fwhm: Some(fwhm), doubling: CLEAN }
}

fn fit(matched_stars: usize, mean_residual: f32) -> AdaptiveRegistrationResult {
    AdaptiveRegistrationResult {
        transform: AffineTransform::identity(),
        matched_stars,
        mean_residual,
        config_used: "test".to_string(),
        attempts: 1,
    }
}

#[test]
fn admits_a_clean_fit() {
    let gate = seeded(0.5, 5.0);
    assert_eq!(gate.judge(&fit(180, 0.6), Some(5.0), 200, 200), None);
}

#[test]
fn rejects_a_fit_built_from_a_handful_of_stars() {
    let gate = seeded(0.5, 5.0);
    // A transform agreeing with 7 of 200 stars is a coincidence, however
    // small its residual over those seven.
    assert_eq!(
        gate.judge(&fit(7, 0.4), Some(5.0), 200, 200),
        Some(RejectionReason::TooFewCorrespondences)
    );
}

#[test]
fn rejects_a_residual_far_above_the_session_median() {
    let gate = seeded(0.5, 5.0);
    assert_eq!(
        gate.judge(&fit(180, 20.0), Some(5.0), 200, 200),
        Some(RejectionReason::ResidualTooHigh)
    );
}

#[test]
fn follows_a_loose_session_rather_than_a_fixed_idea_of_good() {
    // The 250mm Orion fixture registers with a ~5.5 px median residual
    // throughout. A fixed threshold would reject every frame in it; the
    // point of scoring against the session's own median is that a frame is
    // only an outlier relative to its neighbours.
    let gate = seeded(5.5, 5.0);
    assert_eq!(gate.judge(&fit(180, 6.5), Some(5.0), 200, 200), None);
    assert_eq!(
        gate.judge(&fit(180, 20.0), Some(5.0), 200, 200),
        Some(RejectionReason::ResidualTooHigh)
    );
}

/// The other half of the same idea, and the one a median-only rule gets
/// backwards: a rig that tracks *well* must not end up with the strictest
/// gate. The 250 mm dumbbell fixture holds a 0.6 px median residual on 5.4 px
/// stars, where `RESIDUAL_K * median` alone allows only 1.8 px and threw away
/// 9 of its 34 frames for residuals of 1.9–3.3 px — a fraction of one star's
/// width.
#[test]
fn a_well_tracked_session_is_not_punished_for_its_own_precision() {
    let gate = seeded(0.6, 5.4);

    for residual in [1.9, 2.4, 2.7] {
        assert_eq!(
            gate.judge(&fit(150, residual), Some(5.4), 200, 200),
            None,
            "{residual} px is a fraction of a 5.4 px star and must still stack"
        );
    }

    // Past half a star width it is smearing, whatever the session median.
    assert_eq!(
        gate.judge(&fit(150, 8.2), Some(5.4), 200, 200),
        Some(RejectionReason::ResidualTooHigh)
    );
}

/// The star-size floor tracks the session rather than sitting at a constant:
/// the same residual is fine on fat stars and smearing on tight ones.
#[test]
fn the_star_size_floor_follows_the_session_not_a_constant() {
    assert_eq!(
        seeded(0.6, 8.0).judge(&fit(150, 3.5), Some(8.0), 200, 200),
        None,
        "3.5 px is well inside an 8 px star"
    );
    assert_eq!(
        seeded(0.6, 2.5).judge(&fit(150, 3.5), Some(2.5), 200, 200),
        Some(RejectionReason::ResidualTooHigh),
        "3.5 px is wider than a 2.5 px star"
    );
}

#[test]
fn rejects_bloated_stars() {
    let gate = seeded(0.5, 4.0);
    assert_eq!(
        gate.judge(&fit(180, 0.5), Some(9.0), 200, 200),
        Some(RejectionReason::StarsTooLarge)
    );
}

/// `compute_fwhm` counts whole pixels above half maximum, so star size is
/// quantised and its median wanders frame to frame even on a stable night:
/// the 250 mm dumbbell fixture spans 1.60–7.57 px around a 5.4 px median
/// while its residuals hold at 0.6 px. A threshold inside that spread rejects
/// the estimator, not the sky — at 1.35 that set lost its frame 17 (7.57 px)
/// while keeping neighbours at 6.82 and 6.48 px.
#[test]
fn ordinary_scatter_in_measured_star_size_is_not_defocus() {
    let gate = seeded(0.6, 5.4);

    for fwhm in [6.5, 7.6, 9.0] {
        assert_eq!(
            gate.judge(&fit(150, 0.6), Some(fwhm), 200, 200),
            None,
            "{fwhm} px is inside the spread a 5.4 px session measures"
        );
    }

    // Twice the session's star size is defocus, cloud, or shake.
    assert_eq!(
        gate.judge(&fit(150, 0.6), Some(11.0), 200, 200),
        Some(RejectionReason::StarsTooLarge)
    );
}

#[test]
fn admits_a_registered_frame_during_warmup() {
    let mut gate = FrameGate::default();
    gate.history.record(0.5, Some(4.0));
    // Nothing to compare against yet, so a wide residual still counts.
    assert_eq!(gate.judge(&fit(180, 9.0), Some(12.0), 200, 200), None);
}

/// A gate keyed only on accepted frames would latch shut the moment
/// conditions moved past its threshold: nothing accepted means nothing
/// recorded, means the median never catches up. Recording every measured
/// frame lets a sustained change become the new normal.
#[test]
fn a_sustained_change_in_conditions_reopens_the_gate() {
    let mut gate = seeded(0.4, 4.0);
    assert!(gate
        .admit(&fit(180, 9.0), stars(4.0), 200)
        .is_some());

    for _ in 0..HISTORY_LEN {
        if gate.admit(&fit(180, 9.0), stars(4.0), 200).is_none() {
            return;
        }
    }
    panic!("gate never reopened after conditions settled at a new level");
}

/// A frame rejected for having too few correspondences must not move the
/// baseline. Its residual is a mean over the handful of pairs the fit chose
/// for itself — the dumbbell fixture produced one at 8.46 px over 6 of 200
/// stars, in a set whose other frames sit at 1.3–2.0 px. With the median at
/// `sorted[HISTORY_LEN / 2]`, a run of them redefines what the gate calls
/// normal.
#[test]
fn a_coincidental_fit_does_not_move_the_baseline() {
    let mut gate = seeded(0.6, 5.4);
    let before = gate.history.median_residual();

    for _ in 0..HISTORY_LEN {
        assert_eq!(
            gate.admit(&fit(4, 0.05), stars(5.4), 200),
            Some(RejectionReason::TooFewCorrespondences)
        );
    }

    assert_eq!(
        gate.history.median_residual(),
        before,
        "a fit the gate called a coincidence redefined the session"
    );
    assert_eq!(
        gate.judge(&fit(150, 2.4), Some(5.4), 200, 200),
        None,
        "the gate latched shut against a frame it admitted before the burst"
    );
}

/// The verdicts that *are* measurements still have to land, or the gate
/// cannot follow a night that genuinely changes.
#[test]
fn a_frame_rejected_on_its_own_measurements_still_updates_the_baseline() {
    let mut gate = seeded(0.6, 5.4);
    let before = gate.history.median_residual();

    for _ in 0..HISTORY_LEN {
        gate.admit(&fit(150, 12.0), stars(5.4), 200);
    }

    assert!(
        gate.history.median_residual() > before,
        "a sustained rise in residual never reached the baseline"
    );
}

#[test]
fn a_sharper_frame_takes_over_as_reference_early_on() {
    let mut gate = FrameGate::default();
    gate.set_reference(Some(6.0), CLEAN);
    gate.frames_seen = 3;
    assert!(gate.should_rebase(Some(4.0)));
}

#[test]
fn a_marginally_sharper_frame_is_not_worth_the_integration() {
    let mut gate = FrameGate::default();
    gate.set_reference(Some(6.0), CLEAN);
    gate.frames_seen = 3;
    assert!(!gate.should_rebase(Some(5.5)));
}

#[test]
fn the_reference_settles_once_the_window_closes() {
    let mut gate = FrameGate::default();
    gate.set_reference(Some(6.0), CLEAN);
    gate.frames_seen = REBASE_WINDOW + 1;
    assert!(!gate.should_rebase(Some(2.0)));
}

#[test]
fn an_implausibly_sharp_frame_is_detection_noise_not_a_new_reference() {
    let mut gate = seeded(0.5, 5.0);
    gate.set_reference(Some(6.0), CLEAN);
    gate.frames_seen = 3;
    // 1.6 px against a 5.0 px session median is star detection latching onto
    // noise; the dumbbell fixture's frame 11 reports exactly this.
    assert!(!gate.should_rebase(Some(1.6)));
    assert!(gate.should_rebase(Some(3.5)));
}

/// Beating the incumbent is not enough: the reference's own FWHM is a single
/// noisy sample, so a candidate one quantisation step below it is evidence of
/// nothing. This is the difference between the two re-bases the bundled
/// fixtures produce — the Orion set's 2.52 px against a 2.99 px reference in a
/// session that measures 2.26–2.99 px throughout, and the dumbbell set's
/// 4.37 px against a 6.28 px session.
#[test]
fn beating_only_a_noisy_reference_is_not_worth_a_rebase() {
    let mut gate = seeded(0.6, 2.7);
    gate.set_reference(Some(2.99), CLEAN);
    gate.frames_seen = 1;
    assert!(
        !gate.should_rebase(Some(2.52)),
        "one step of the area-based estimator is not a sharper frame"
    );

    // 6.2 rather than the fixture's exact 6.283 — that is tau, and clippy
    // reads the literal as a mis-typed constant.
    let mut gate = seeded(0.6, 6.2);
    gate.set_reference(Some(6.2), CLEAN);
    gate.frames_seen = 2;
    assert!(
        gate.should_rebase(Some(4.37)),
        "30% sharper than the whole session is a real change"
    );
}

/// A sub that would not register because every star in it is doubled was bumped, not
/// moved: Wanderer must hold the stack through it.
#[test]
fn a_bumped_sub_that_will_not_register_is_not_the_sky_moving() {
    let mut gate = seeded(0.6, 5.0);
    gate.set_reference(Some(5.0), CLEAN);
    let bumped = StarField { doubling: 0.9, ..stars(5.0) };

    let verdict = gate.explain_unregistered(bumped, THIS_FIELD);

    assert_eq!(verdict, RejectionReason::StarsDoubled);
    assert!(!verdict.means_the_sky_moved());
}

#[test]
fn a_bloated_sub_that_will_not_register_is_soft_not_moved() {
    let mut gate = seeded(0.6, 2.3);
    gate.set_reference(Some(2.3), CLEAN);

    assert_eq!(gate.explain_unregistered(stars(7.2), THIS_FIELD), RejectionReason::StarsTooLarge);
}

/// An ordinary-looking field that will not register is a different field: that is
/// exactly Wanderer's signal.
#[test]
fn a_clean_sub_that_will_not_register_is_the_sky_moving() {
    let mut gate = seeded(0.6, 5.0);
    gate.set_reference(Some(5.0), CLEAN);

    let verdict = gate.explain_unregistered(stars(5.0), THIS_FIELD);

    assert_eq!(verdict, RejectionReason::RegistrationFailed);
    assert!(verdict.means_the_sky_moved());
}

/// A dense field scores high on every sub. Judged against a constant it would read as
/// doubled throughout; judged against its own session it reads as what it is.
#[test]
fn a_crowded_session_is_judged_against_itself() {
    let mut gate = seeded(0.6, 3.0);
    gate.set_reference(Some(3.0), 0.55);
    for _ in 0..WARMUP_FRAMES {
        gate.explain_unregistered(StarField { doubling: 0.55, ..stars(3.0) }, THIS_FIELD);
    }

    let dense = StarField { doubling: 0.63, ..stars(3.0) };
    assert_eq!(gate.explain_unregistered(dense, THIS_FIELD), RejectionReason::RegistrationFailed);
    let bumped = StarField { doubling: 1.0, ..stars(3.0) };
    assert_eq!(gate.explain_unregistered(bumped, THIS_FIELD), RejectionReason::StarsDoubled);
}

/// Before the session has a baseline of its own, the reference stands in for it.
#[test]
fn the_reference_is_the_doubling_baseline_until_the_session_has_one() {
    let mut gate = FrameGate::default();
    gate.set_reference(Some(5.0), CLEAN);

    let bumped = StarField { doubling: 0.9, ..stars(5.0) };
    assert_eq!(gate.explain_unregistered(bumped, THIS_FIELD), RejectionReason::StarsDoubled);
}

/// A registered doubled sub still stacks: dropping those was measured to cost more
/// sky noise than it buys in star size.
#[test]
fn a_registered_doubled_sub_still_stacks() {
    let mut gate = seeded(0.6, 5.0);
    gate.set_reference(Some(5.0), CLEAN);
    let bumped = StarField { doubling: 0.9, ..stars(5.0) };

    assert_eq!(gate.admit(&fit(150, 0.6), bumped, 200), None);
}

/// A re-based frame became the reference, so it landed on itself — not where its fit
/// against the discarded reference put it.
#[test]
fn a_rebased_frame_lands_on_itself() {
    let mut moved = fit(150, 0.6);
    moved.transform = AffineTransform::from_translation(12.0, -7.0);

    assert_eq!(FrameAdmission::accepted(&moved, false).transform, Some(moved.transform));
    assert_eq!(FrameAdmission::accepted(&moved, true).transform, Some(AffineTransform::identity()));
}

/// A dense new field scores as doubled against a sparse session; unless its stars, made
/// single again, register against the reference, it is the sky having moved.
#[test]
fn a_dense_new_field_is_not_mistaken_for_a_bump() {
    let mut gate = seeded(0.6, 5.0);
    gate.set_reference(Some(5.0), CLEAN);
    let dense = StarField { doubling: 0.6, ..stars(5.0) };

    let verdict = gate.explain_unregistered(dense, || false);

    assert_eq!(verdict, RejectionReason::RegistrationFailed);
    assert!(verdict.means_the_sky_moved());
}

/// Wanderer, swung to a new field whose stars are far larger than the old session's
/// (a Barlow added, a low target in poor seeing): no sub ever registers, so `admit`
/// never runs and the FWHM baseline never moves. A soft verdict must not hold the old
/// stack for the rest of the night — a run of them is the sky having moved.
#[test]
fn a_sustained_run_of_bloated_unregistered_subs_is_the_sky_moving() {
    let mut gate = seeded(0.6, 3.0);
    gate.set_reference(Some(3.0), CLEAN);

    let verdicts: Vec<_> =
        (0..HISTORY_LEN).map(|_| gate.explain_unregistered(stars(7.0), THIS_FIELD)).collect();

    assert!(
        verdicts.iter().any(|v| v.means_the_sky_moved()),
        "{} bloated subs in a row and Wanderer still holds the old stack",
        verdicts.len()
    );
}

/// Gusts spread over a night are each a soft frame: a sub that registers between them
/// proves the field is still there and starts the count again.
#[test]
fn bloated_subs_between_registered_ones_never_add_up_to_movement() {
    let mut gate = seeded(0.6, 3.0);
    gate.set_reference(Some(3.0), CLEAN);

    for _ in 0..4 {
        for _ in 0..BLOATED_RUN_LIMIT {
            assert_eq!(gate.explain_unregistered(stars(7.0), THIS_FIELD), RejectionReason::StarsTooLarge);
        }
        gate.admit(&fit(150, 0.6), stars(3.0), 200);
    }
}

/// A rough night: most subs are doubled, and most of those still register and stack.
/// Their shares must not become the baseline, or the next bump that fails to register —
/// a field already proven to be this one — reads as movement and Wanderer restarts.
#[test]
fn a_run_of_doubled_subs_does_not_become_the_baseline() {
    let mut gate = seeded(0.6, 5.0);
    gate.set_reference(Some(5.0), CLEAN);
    let doubled = StarField { doubling: 0.9, ..stars(5.0) };
    for i in 0..HISTORY_LEN {
        let sub = if i % 5 < 3 { doubled } else { stars(5.0) };
        gate.admit(&fit(150, 0.6), sub, 200);
    }

    assert_eq!(gate.explain_unregistered(doubled, THIS_FIELD), RejectionReason::StarsDoubled);
}

/// The doubled verdict is a cost filter in front of `is_this_field`; once that proof
/// says this field, a share no higher than the session's own still means a bump.
/// Same scenario as above, through unregistered subs only.
#[test]
fn unregistered_bumps_do_not_raise_their_own_bar() {
    let mut gate = seeded(0.6, 5.0);
    gate.set_reference(Some(5.0), CLEAN);
    let doubled = StarField { doubling: 0.9, ..stars(5.0) };
    for i in 0..HISTORY_LEN {
        if i % 5 < 3 {
            gate.explain_unregistered(doubled, THIS_FIELD);
        } else {
            gate.admit(&fit(150, 0.6), stars(5.0), 200);
        }
    }

    assert_eq!(gate.explain_unregistered(doubled, THIS_FIELD), RejectionReason::StarsDoubled);
}
