//! The run's replay cache (`instruments::replay_cache`): an entry reads back bit for bit, a
//! key follows what a replay reads, and tests asking for one key at once replay it once.
//! Community's only: Pro pulls `instruments` into a dozen binaries, which would each run these.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use night_amplifier::registration::AffineTransform;
use night_amplifier::session::capture::pipeline::StackingOutcome;
use night_amplifier::session::capture::{FrameAdmission, RejectionReason, Restacked};
use night_amplifier::session::state::CaptureSettings;
use night_amplifier::{Frame, NoiseField};
use serial_test::serial;

use crate::integration::common::{ensure_fixtures_sync_named, missing_fixture_message, FIXTURES_DIR};
use crate::integration::instruments::replay_cache::{self, decoded, encoded, Key, Run, RUN_LOCK};
use crate::integration::instruments::{
    accumulator_snapshots, for_each_snapshot, load_sub, replayed, session_frames, AccumulatorSnapshot, Replay,
    ReplayRecord,
};

const REASONS: [RejectionReason; 11] = [
    RejectionReason::NoStars,
    RejectionReason::TooFewStars,
    RejectionReason::RegistrationFailed,
    RejectionReason::TooFewCorrespondences,
    RejectionReason::ResidualTooHigh,
    RejectionReason::StarsTooLarge,
    RejectionReason::StarsDoubled,
    RejectionReason::StackerError,
    RejectionReason::Outvoted,
    RejectionReason::TargetOutOfReach,
    RejectionReason::SurfaceNotFound,
];

fn frame(w: usize, h: usize, c: usize, seed: f32) -> Frame {
    let data = (0..w * h * c).map(|i| (i as f32 * 0.37 + seed).sin().abs()).collect();
    Frame::from_f32_vec(data, w, h, c).unwrap()
}

/// A 2x2 grid over 16x12, with variance (some cells not yet measured) or without.
fn field(variance: bool) -> NoiseField {
    let coverage = vec![1.0, 0.75, 0.5, 0.125];
    if !variance {
        return NoiseField::coverage_only(coverage, 2, 2, 3, 16, 12).unwrap();
    }
    let cells = (0..12u32).map(|i| if i.is_multiple_of(5) { f32::NAN } else { 1e-6 * i as f32 }).collect();
    NoiseField::new(cells, 2, 2, 3, 16, 12).unwrap().with_coverage(coverage).unwrap()
}

/// Sub `i` of a made-up session: every rejection reason once, then admitted subs.
fn outcome(i: usize) -> StackingOutcome {
    let reason = i.checked_sub(1).and_then(|k| REASONS.get(k)).copied();
    StackingOutcome {
        display_frame: i.is_multiple_of(3).then(|| frame(5, 4, 3, i as f32)),
        showing_stack: i.is_multiple_of(2),
        frame_added: reason.is_none(),
        stack_reset: i == 4,
        admission: (i > 0).then(|| FrameAdmission {
            added: reason.is_none(),
            rejected_because: reason,
            rebased: i == 7,
            matched_stars: 40 + i,
            mean_residual: if i == 2 { f32::NAN } else { 0.5 + i as f32 / 8.0 },
            transform: (i % 2 == 1)
                .then(|| AffineTransform { rotation: 1e-4 * i as f32, scale: 1.0003, tx: -3.5, ty: 12.25 }),
            restacked: (i == 6).then_some(Restacked { retracted: 1, reinstated: 2, rereferenced: true }),
        }),
        stack_depth: i as u32,
        noise: [None, Some(field(false)), Some(field(true))][i % 3].clone(),
    }
}

#[test]
fn an_entry_reads_back_bit_for_bit() {
    let record = ReplayRecord {
        subs: 14,
        depth: 9,
        outcomes: (0..14).map(outcome).collect(),
        stack: Some(frame(7, 5, 3, 0.1)),
        coverage: Some(field(true)),
    };
    let bytes = encoded(&record);
    let back: ReplayRecord = decoded(&bytes).unwrap();
    assert!(encoded(&back) == bytes, "a record changed on its way through an entry");
    assert_eq!((back.subs, back.depth), (14, 9));
    assert!(back.outcomes[12..].iter().all(|o| o.frame_added), "admitted subs read back too");
    let reasons: Vec<_> = back.outcomes.iter().filter_map(StackingOutcome::rejected_because).collect();
    assert_eq!(reasons, REASONS);
    assert!(back.outcomes[2].admission.unwrap().mean_residual.is_nan());
    assert_eq!(back.outcomes[6].admission.unwrap().restacked, Some(Restacked { retracted: 1, reinstated: 2, rereferenced: true }));
    assert_eq!(back.stack.as_ref().map(Frame::data), record.stack.as_ref().map(Frame::data));
    let stackless = ReplayRecord { subs: 3, depth: 0, outcomes: Vec::new(), stack: None, coverage: None };
    assert!(encoded(&decoded::<ReplayRecord>(&encoded(&stackless)).unwrap()) == encoded(&stackless));

    let snapshots = vec![AccumulatorSnapshot {
        depth: 3,
        frame: frame(6, 4, 3, 0.2),
        noise: field(false),
        measured: field(true),
        coverage: frame(6, 4, 1, 0.3),
        generation: 2,
    }];
    let bytes = encoded(&snapshots);
    assert!(encoded(&decoded::<Vec<AccumulatorSnapshot>>(&bytes).unwrap()) == bytes, "a snapshot changed");
    assert!(decoded::<Vec<AccumulatorSnapshot>>(&bytes[..bytes.len() - 1]).is_err(), "a short entry decoded");
}

#[test]
fn a_key_follows_what_a_replay_reads() {
    let name = |key: Key| key.file_name("set");
    let base = || Key::new("replay").settings(&CaptureSettings::default());
    assert_eq!(name(base()), name(base()));
    assert_ne!(name(base()), name(Key::new("stack").settings(&CaptureSettings::default())));
    assert_ne!(name(base()), name(Key::new("replay").settings(&CaptureSettings { stacking: false, ..Default::default() })));
    assert_ne!(name(base()), name(base().with("depths", "[1]")));

    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("frame_00001.fits");
    std::fs::write(&sub, [0u8; 8]).unwrap();
    let before = name(base().files(std::slice::from_ref(&sub)));
    std::fs::write(&sub, [0u8; 9]).unwrap();
    assert_ne!(before, name(base().files(&[sub])), "a re-cut sub kept its key");

    // Maps are `HashMap`s, each iterating in its own order: the key must not.
    let with_profiles = || {
        let mut settings = CaptureSettings::default();
        for camera in ["ZWO ASI533MC Pro", "QHY5III462C", "Neptune-C II", "SV305", "Ares-C PRO", "Uranus-C"] {
            settings.camera_telescope_profiles.insert(camera.to_string(), Default::default());
        }
        Key::new("replay").settings(&settings)
    };
    assert_eq!(name(with_profiles()), name(with_profiles()));

    // A test's fake fills a slot under the same name as Pro's plugin: the key tells them apart.
    use night_amplifier::plugins::Plugins;
    use night_amplifier::render::{SaturationBoostConfig, SaturationPlugin};
    struct Identity;
    struct Doubling;
    impl SaturationPlugin for Identity {
        fn apply_boost(&self, _: &mut Frame, _: &SaturationBoostConfig) -> night_amplifier::error::Result<()> {
            Ok(())
        }
        fn apply_boost_slice(&self, _: &mut [f32], _: &SaturationBoostConfig) {}
    }
    impl SaturationPlugin for Doubling {
        fn apply_boost(&self, _: &mut Frame, _: &SaturationBoostConfig) -> night_amplifier::error::Result<()> {
            Ok(())
        }
        fn apply_boost_slice(&self, row: &mut [f32], _: &SaturationBoostConfig) {
            row.iter_mut().for_each(|v| *v *= 2.0);
        }
    }
    let with = |plugins: Plugins| name(base().plugins(&plugins));
    let identity = Plugins::none().with_saturation(std::sync::Arc::new(Identity));
    assert_eq!(with(identity.clone()), with(identity.clone()));
    assert_ne!(with(identity.clone()), with(Plugins::none().with_saturation(std::sync::Arc::new(Doubling))));
    assert_ne!(with(identity.clone()), with(identity.always_licensed()), "a licence-free set answers");
    assert_ne!(with(Plugins::none()), with(Plugins::none().with_saturation(std::sync::Arc::new(Identity))));
}

/// Without nextest a process gets a directory no earlier one had: a bare pid repeats (a
/// container's first processes), and the directory it named served an earlier build's stacks.
#[test]
fn a_run_never_reopens_an_earlier_processes_directory() {
    let root = tempfile::tempdir().unwrap();
    let stale = root.path().join(format!("pid-{}", std::process::id()));
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join(RUN_LOCK), b"").unwrap();
    std::fs::write(stale.join("set-0123456789abcdef.rec"), b"an earlier build's entry").unwrap();

    let first = Run::open(root.path(), None).unwrap();
    let second = Run::open(root.path(), None).unwrap();
    assert_ne!(first.dir, second.dir, "two processes, one directory");
    for run in [&first, &second] {
        assert_ne!(run.dir, stale);
        let held: Vec<_> = std::fs::read_dir(&run.dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(held, [RUN_LOCK], "{} does not start empty", run.dir.display());
    }
    assert!(!stale.exists(), "no process held the earlier run, so it was swept");
}

/// Nextest's processes join one run, which lives while any holds it and is swept after.
#[test]
fn a_nextest_run_is_shared_while_held_and_swept_once_let_go() {
    let root = tempfile::tempdir().unwrap();
    let a = Run::open(root.path(), Some("run-a")).unwrap();
    let a_again = Run::open(root.path(), Some("run-a")).unwrap();
    assert_eq!(a.dir, a_again.dir);
    let b = Run::open(root.path(), Some("run-b")).unwrap();
    assert!(a.dir.is_dir(), "a held run survived another run's sweep");

    let a_dir = a.dir.clone();
    drop((a, a_again));
    let _c = Run::open(root.path(), Some("run-c")).unwrap();
    assert!(!a_dir.exists(), "a run nobody holds was kept");
    assert!(b.dir.is_dir());
}

#[test]
fn tests_asking_for_one_key_at_once_replay_it_once() {
    let made = AtomicUsize::new(0);
    let tag = format!("{} {:?}", std::process::id(), std::time::SystemTime::now());
    let ask = || {
        let key = Key::new("lock-test").with("tag", &tag);
        replay_cache::cached(&key, "lock-test", || {
            made.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(300));
            frame(4, 4, 1, 0.5)
        })
    };
    let [a, b] = std::thread::scope(|s| [s.spawn(ask), s.spawn(ask)].map(|t| t.join().unwrap()));
    assert_eq!(a.data(), b.data());
    // Off (`NA_REPLAY_CACHE=off`), every caller replays for itself.
    let expected = if replay_cache::enabled() { 1 } else { 2 };
    assert_eq!(made.load(Ordering::SeqCst), expected, "the cache is {}", if expected == 1 { "on" } else { "off" });
}

/// What a test reads from the run's cache is what it would have replayed itself, on both
/// engines: the simulated-camera replay and the snapshot stack.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_shared_replay_is_the_replay() {
    const SET: &str = "130mm-imx464-ring-nebulae-png";
    ensure_fixtures_sync_named(&[SET]);
    let dir = Path::new(FIXTURES_DIR).join(SET);
    let files = session_frames(&dir).unwrap_or_else(|| panic!("{}", missing_fixture_message(SET)));
    assert!(replay_cache::enabled(), "the run's cache is off (NA_REPLAY_CACHE=off): nothing to compare");

    replayed(&dir); // stored, if no test of this run stored it already
    let reads = replay_cache::reads();
    let shared = replayed(&dir);
    assert_eq!(replay_cache::reads(), reads + 1, "the replay was made again, not read back");
    assert!(shared.outcomes.iter().all(|o| o.display_frame.is_none()), "a display copy was stored");
    let fresh = encoded(&Replay::open(&dir).played());
    assert_same(&encoded(&shared), &fresh, "replay");

    let depths = [4, 12];
    accumulator_snapshots(&files, &depths);
    let reads = replay_cache::reads();
    let shared = encoded(&accumulator_snapshots(&files, &depths));
    assert_eq!(replay_cache::reads(), reads + 1, "the stack was made again, not read back");
    let mut fresh = Vec::new();
    for_each_snapshot(&files, &depths, &load_sub, |s| fresh.push(s));
    assert_same(&shared, &encoded(&fresh), "snapshot stack");
}

fn assert_same(shared: &[u8], fresh: &[u8], engine: &str) {
    let first = shared.iter().zip(fresh).position(|(a, b)| a != b);
    assert!(
        first.is_none() && shared.len() == fresh.len(),
        "the shared {engine} differs from a fresh one: {} against {} bytes, first at {first:?}",
        shared.len(),
        fresh.len()
    );
}
