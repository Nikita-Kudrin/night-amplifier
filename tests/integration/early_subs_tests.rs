//! Bad first subs against the stack's probation, on real defects and injected ones.
//!
//! Each case replays the same subs three ways (see `early_subs`): the oracle, without the
//! bad subs; the stack as it streamed before probation; and the probation as shipped.
//! Community has no pixel rejection, so a thin trail stays at 1/N here as it does
//! mid-session; Pro's `early_subs_regression` holds its batch rejection to removing it.
//!
//! `cargo test --test integration_pipeline early_subs -- --ignored --nocapture --test-threads=1`

use std::path::Path;

use crate::integration::common::{ensure_fixtures_sync_named, missing_fixture_message, FIXTURES_DIR};
use crate::integration::early_subs::{compare, injected_cases, misses, real_cases, Comparison, Defect, Expect, Session, FIXTURE};
use crate::integration::stack_depth_grain_tests::managed_session;

/// A clean real session the synthetic defects go into: 1024² of M27, fast to replay.
const CLEAN: &str = "deep-stack-dumbbell-106";

/// Subs of [`CLEAN`] replayed: past the probation and the rejection warm-up both.
const CLEAN_SUBS: usize = 20;

fn real(name: &str) -> Comparison {
    ensure_fixtures_sync_named(&[FIXTURE]);
    let session = Session::open(Path::new(FIXTURES_DIR), name)
        .unwrap_or_else(|| panic!("{}", missing_fixture_message(FIXTURE)));
    let comparison = compare(&session.dir, session.subs, &session.bad, &[]);
    comparison.print(name);
    comparison
}

fn injected(label: &str, defect: Defect, at: &[usize]) -> Comparison {
    let dir = managed_session(CLEAN)[0].parent().expect("a session directory").to_path_buf();
    let defects: Vec<(usize, Defect)> = at.iter().map(|&k| (k, defect)).collect();
    let comparison = compare(&dir, CLEAN_SUBS, at, &defects);
    comparison.print(label);
    comparison
}

/// Trails Community cannot take out of a sub: no worse than the stream.
fn community_trails() -> Vec<(&'static str, Defect, Vec<usize>, Expect)> {
    vec![
        ("trail@1", Defect::Trail { sigmas: 40.0 }, vec![1], Expect::NoWorse { hfr: 0.01 }),
        ("trail@1-3", Defect::Trail { sigmas: 40.0 }, vec![1, 2, 3], Expect::NoWorse { hfr: 0.01 }),
    ]
}

/// The four real sessions that open on bad subs stack as their oracle does: the
/// trail out, a bumped or smeared reference replaced, no good sub lost.
#[test]
#[ignore]
fn real_bad_first_subs_stack_as_their_oracle() {
    let failures: Vec<String> = real_cases().into_iter().flat_map(|(name, expect)| misses(name, &real(name), expect)).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Bumps, cloud, defocus and smear injected into a clean session's first subs.
#[test]
#[ignore]
fn injected_bad_first_subs_stack_as_their_oracle() {
    let failures: Vec<String> = injected_cases()
        .into_iter()
        .chain(community_trails())
        .flat_map(|(label, defect, at, expect)| misses(label, &injected(label, defect, &at), expect))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
