//! Planetary stacking on 2026-10-24's drifting Moon (`planetary`), replayed through the
//! simulated camera, `convert_captured_frame` and `stack_frame` as `instruments::Replay`
//! replays deep-sky sessions.

use super::instruments::Replay;
use super::planetary::{binned_sharpness, moon, planetary_settings, FRAMES};

/// Every frame joins the stack where the exhaustive search puts it, the drift past ±50 px
/// and the jitter included: the old search stacked 16 of these 30 at a wrong shift.
#[test]
#[ignore]
fn every_moon_frame_lands_where_an_exhaustive_search_puts_it() {
    let mut replay = Replay::with_settings(&moon(), planetary_settings());
    assert_eq!(replay.subs(), FRAMES.len());
    let outcomes = replay.run();
    let mut misplaced = Vec::new();
    for (outcome, (original, why, (dx, dy))) in outcomes.iter().zip(FRAMES).skip(1) {
        let landed = outcome.admission.as_ref().and_then(|a| a.transform);
        let Some(landed) = landed else {
            misplaced.push(format!("frame {original} ({why}) was not stacked"));
            continue;
        };
        let (ex, ey) = (landed.tx - dx as f32, landed.ty - dy as f32);
        if ex.abs() > 0.75 || ey.abs() > 0.75 {
            misplaced.push(format!(
                "frame {original} ({why}) landed at ({:.2}, {:.2}), measured ({dx}, {dy})",
                landed.tx, landed.ty
            ));
        }
    }
    assert!(misplaced.is_empty(), "{}", misplaced.join("\n"));
    assert_eq!(replay.depth(), FRAMES.len());
}

/// Thirty aligned frames are as sharp as one, where the same frames averaged where they
/// fell lose most of it: a wrong shift on any part of the set shows here.
#[test]
#[ignore]
fn the_moon_stack_is_as_sharp_as_its_reference() {
    let mut replay = Replay::with_settings(&moon(), planetary_settings());
    let reference = replay.capture().expect("a first frame");
    replay.stack(&reference, false);
    let mut unaligned = reference.clone();
    let mut count = 1.0f32;
    while let Some(frame) = replay.capture() {
        replay.stack(&frame, false);
        for (sum, &v) in unaligned.data_mut().iter_mut().zip(frame.data()) {
            *sum += v;
        }
        count += 1.0;
    }
    unaligned.data_mut().iter_mut().for_each(|v| *v /= count);

    let sharp = binned_sharpness(&reference);
    let stacked = binned_sharpness(&replay.snapshot()) / sharp;
    let smeared = binned_sharpness(&unaligned) / sharp;
    eprintln!("stack {stacked:.3} of the reference's sharpness, unaligned average {smeared:.3}");
    assert!(stacked > 0.9, "the stack kept {stacked:.3} of the reference's sharpness");
    assert!(smeared < 0.5, "the measure cannot tell: unaligned frames kept {smeared:.3}");
}
