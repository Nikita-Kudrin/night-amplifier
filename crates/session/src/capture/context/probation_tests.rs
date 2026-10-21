use super::{affordable, Probation, PROBATION_SUBS};
use crate::capture::frame_gate::{RejectionReason, Restacked};
use crate::capture::pipeline::{stack_frame_with, StackingOutcome};
use crate::capture::{LiveStacker, StackSettings};
use crate::state::CaptureSettings;
use night_amplifier_core::frame::Frame;
use night_amplifier_core::plugins::Plugins;
use night_amplifier_core::stacking::StackingType;

const SIZE: usize = 160;

/// Twelve stars, `(x, y, peak)`, spread over the frame for triangle matching.
const STARS: [(f32, f32, f32); 12] = [
    (18.0, 22.0, 0.9),
    (57.0, 31.0, 0.7),
    (91.0, 19.0, 0.8),
    (34.0, 68.0, 0.6),
    (76.0, 84.0, 0.85),
    (110.0, 55.0, 0.75),
    (25.0, 105.0, 0.65),
    (98.0, 112.0, 0.9),
    (63.0, 127.0, 0.7),
    (129.0, 92.0, 0.8),
    (140.0, 30.0, 0.6),
    (45.0, 145.0, 0.75),
];

/// The field moved by `offset`, with only the stars `keep` lets through, each also
/// echoed at `echo` with half its light (a bump) when given.
fn field(offset: f32, keep: impl Fn(usize) -> bool, echo: Option<(f32, f32)>) -> Frame {
    let mut frame = Frame::filled(SIZE, SIZE, 1, 0.02).unwrap();
    let mut add = |sx: f32, sy: f32, peak: f32| {
        let (cx, cy) = (sx.round() as usize, sy.round() as usize);
        for y in cy.saturating_sub(6)..(cy + 6).min(SIZE) {
            for x in cx.saturating_sub(6)..(cx + 6).min(SIZE) {
                let d2 = (x as f32 - sx).powi(2) + (y as f32 - sy).powi(2);
                let v = frame.get_pixel(x, y, 0) + peak * (-d2 / 4.0).exp();
                frame.set_pixel(x, y, 0, v.min(1.0));
            }
        }
    };
    for (_, &(x, y, peak)) in STARS.iter().enumerate().filter(|(i, _)| keep(*i)) {
        let (x, y) = (x + offset, y + offset);
        match echo {
            Some((dx, dy)) => {
                add(x, y, peak / 2.0);
                add(x + dx, y + dy, peak / 2.0);
            }
            None => add(x, y, peak),
        }
    }
    frame
}

fn clean(k: usize) -> Frame {
    field(0.3 * k as f32, |_| true, None)
}

struct Session {
    stack: StackSettings,
    stacker: Option<Box<dyn LiveStacker>>,
    failed: bool,
}

impl Session {
    fn new(probation: usize) -> Self {
        let mut stack = StackSettings::of(&CaptureSettings::default(), &Plugins::none());
        stack.probation_subs = probation;
        Self { stack, stacker: None, failed: false }
    }

    fn offer(&mut self, frame: &Frame) -> StackingOutcome {
        stack_frame_with(frame, StackingType::DeepSky, &self.stack, &mut self.stacker, &mut self.failed, false)
    }

    fn depth(&self) -> usize {
        self.stacker.as_ref().map_or(0, |s| s.depth())
    }
}

/// A starless first sub (a cloud, a dew cap) used to switch stacking off for the night.
#[test]
fn a_starless_first_sub_leaves_the_next_to_be_the_reference() {
    let mut session = Session::new(PROBATION_SUBS);
    let blank = Frame::filled(SIZE, SIZE, 1, 0.02).unwrap();
    let outcome = session.offer(&blank);
    assert!(!outcome.frame_added);
    assert!(!session.failed, "one starless sub ended stacking");

    assert!(session.offer(&clean(0)).frame_added, "the next sub is the reference");
    assert_eq!(session.depth(), 1);
}

/// Nothing bad, nothing changes: every held sub stays, on the reference it streamed on.
#[test]
fn a_clean_probation_keeps_every_sub_on_its_first_reference() {
    let mut session = Session::new(PROBATION_SUBS);
    let outcomes: Vec<StackingOutcome> = (0..PROBATION_SUBS + 2).map(|k| session.offer(&clean(k))).collect();
    assert!(outcomes.iter().all(|o| o.frame_added));
    let commit = outcomes[PROBATION_SUBS - 1].admission.and_then(|a| a.restacked).expect("the probation committed");
    assert_eq!(commit, Restacked::default());
    assert_eq!(session.depth(), PROBATION_SUBS + 2);
}

/// A bumped first sub is the reference only until the probation commits; the stream
/// kept it as the reference for good.
#[test]
fn a_bumped_reference_is_replaced_at_the_commit() {
    let bumped = field(0.0, |_| true, Some((9.0, 4.0)));
    let rereferenced = |probation| {
        let mut session = Session::new(probation);
        session.offer(&bumped);
        (1..PROBATION_SUBS).map(|k| session.offer(&clean(k))).last().and_then(|o| o.admission).map(|a| a.restacked.is_some_and(|r| r.rereferenced))
    };
    assert_eq!(rereferenced(PROBATION_SUBS), Some(true));
    assert_eq!(rereferenced(0), Some(false));
}

/// A sub whose stars mostly fail to line up with the reference's — under cloud, only its
/// brightest show — is taken back out, and the counters hear about it.
#[test]
fn a_sub_that_pairs_few_stars_is_outvoted_and_retracted() {
    let mut session = Session::new(PROBATION_SUBS);
    let thin = field(0.6, |i| i < 4, None);
    let mut outcomes = vec![session.offer(&clean(0)), session.offer(&clean(1)), session.offer(&thin)];
    assert!(outcomes[2].frame_added, "provisionally, the thin sub stacks");
    outcomes.extend((3..PROBATION_SUBS).map(|k| session.offer(&clean(k))));

    let commit = outcomes[PROBATION_SUBS - 1].admission.and_then(|a| a.restacked).expect("the probation committed");
    assert_eq!(commit.retracted, 1, "{commit:?}");
    assert_eq!(session.depth(), PROBATION_SUBS - 1);
    assert!(!RejectionReason::Outvoted.means_the_sky_moved());
}

/// `stars` drawn with a Gaussian of variance `spread / 2`.
fn starfield(stars: &[(f32, f32, f32)], spread: f32) -> Frame {
    let mut frame = Frame::filled(SIZE, SIZE, 1, 0.02).unwrap();
    for &(sx, sy, peak) in stars {
        let (cx, cy) = (sx.round() as usize, sy.round() as usize);
        for y in cy.saturating_sub(9)..(cy + 9).min(SIZE) {
            for x in cx.saturating_sub(9)..(cx + 9).min(SIZE) {
                let d2 = (x as f32 - sx).powi(2) + (y as f32 - sy).powi(2);
                let v = frame.get_pixel(x, y, 0) + peak * (-d2 / spread).exp();
                frame.set_pixel(x, y, 0, v.min(1.0));
            }
        }
    }
    frame
}

/// Twelve stars at positions `seed` scatters: no fit relates two seeds.
fn scattered(seed: u32) -> Vec<(f32, f32, f32)> {
    let mut s = seed.wrapping_mul(2_654_435_761) | 1;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        (s % 10_000) as f32 / 10_000.0
    };
    (0..STARS.len()).map(|_| (14.0 + 132.0 * next(), 14.0 + 132.0 * next(), 0.6 + 0.3 * next())).collect()
}

/// A probation that elects no one leaves sub 1 the blind pick it was, so a much sharper
/// sub inside the re-basing window still takes over, as it does without a probation.
/// Three subs of the field (one short of a majority of six) give the gate a session to
/// judge the sharp one by; 0.72 sits inside `REBASE_MARGIN` and `REBASE_MIN_RATIO` both.
#[test]
fn a_probation_that_elects_no_one_leaves_the_rebase_open() {
    let soft = starfield(&STARS, 12.0);
    let sharp = starfield(&STARS, 12.0 * 0.72 * 0.72);
    let rebased = |probation| {
        let mut session = Session::new(probation);
        assert!(session.offer(&soft).frame_added, "the soft sub is the reference");
        for _ in 0..2 {
            assert!(session.offer(&soft).frame_added);
        }
        for seed in 1..=3 {
            session.offer(&starfield(&scattered(seed), 12.0));
        }
        assert!(session.stacker.as_ref().is_some_and(|s| s.depth() == 3), "nothing was elected or rebuilt");
        session.offer(&sharp).stack_reset
    };
    assert!(rebased(0), "without a probation the sharper sub re-bases");
    assert!(rebased(PROBATION_SUBS), "a probation that elected no one shut the re-base for good");
}

/// Probation 0 is the stack as it streamed: no commit, no restack.
#[test]
fn without_a_probation_the_stack_streams() {
    let mut session = Session::new(0);
    let thin = field(0.6, |i| i < 4, None);
    for k in 0..PROBATION_SUBS {
        let outcome = session.offer(&if k == 2 { thin.clone() } else { clean(k) });
        let admission = outcome.admission;
        assert!(admission.is_none_or(|a| a.restacked.is_none()));
    }
    assert_eq!(session.depth(), PROBATION_SUBS);
}




/// The probation holds what half the memory free at the stack's start affords, beside the
/// commit's f32 scratch frame; unknown memory holds all it asks for.
#[test]
fn the_probation_holds_what_memory_affords() {
    const GB: usize = 1 << 30;
    let imx533 = 3008 * 3008 * 3;
    assert_eq!(affordable(PROBATION_SUBS, imx533, None), PROBATION_SUBS);
    assert_eq!(affordable(PROBATION_SUBS, imx533, Some(3 * GB)), PROBATION_SUBS, "a 4 GB board, 9 MP");

    // A 26 MP colour sensor with 1800 MiB left after its accumulator: (900 - 299) / 149 MiB.
    let imx571 = 6248 * 4176 * 3;
    assert_eq!(affordable(PROBATION_SUBS, imx571, Some(1800 << 20)), 4);
    // Too little for two: no probation, the stack streams.
    assert!(Probation::new(affordable(PROBATION_SUBS, imx571, Some(GB))).is_none());
}
