//! A fixture set counts as present only if it holds what its published archive does
//! (`common::check_fixture_contents`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::integration::common::{
    check_fixture_contents, contents_of, publish_staged, Contents, Fixture, DEFAULT_FIXTURES,
};

/// Two 6-byte frames, one in a subdirectory (as `early-bad-subs-imx533` nests its sessions).
const SET: Fixture = Fixture {
    name: "set",
    url: "https://drive.usercontent.google.com/download?id=current&export=download",
    contents: Contents { files: 2, bytes: 12 },
};

/// `SET` as its archive extracts, in `root/<name>`.
fn extracted(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("session")).unwrap();
    fs::write(dir.join("frame_000001.fits"), b"SIMPLE").unwrap();
    fs::write(dir.join("session").join("frame_000002.fits"), b"SIMPLE").unwrap();
    dir
}

#[test]
fn a_set_as_published_is_accepted_whatever_dotfiles_join_it() {
    let root = tempfile::tempdir().unwrap();
    let dir = extracted(root.path(), "set");
    fs::write(dir.join(".DS_Store"), b"finder").unwrap();
    fs::create_dir_all(dir.join(".cache")).unwrap();
    fs::write(dir.join(".cache").join("frame.fits"), b"ignored").unwrap();
    assert_eq!(contents_of(&dir).unwrap(), SET.contents);
    assert_eq!(check_fixture_contents(&dir, &SET), Ok(()));
}

/// The IC 59 failure: the same frames by name, cut smaller by hand.
#[test]
fn a_hand_cut_copy_is_refused_and_left_as_it_was() {
    let root = tempfile::tempdir().unwrap();
    let dir = extracted(root.path(), "set");
    fs::write(dir.join("frame_000001.fits"), b"SIM").unwrap();
    let error = check_fixture_contents(&dir, &SET).unwrap_err();
    assert!(error.contains("holds 2 files / 9 bytes"), "{error}");
    assert!(error.contains("registers 2 files / 12 bytes"), "{error}");
    assert_eq!(fs::read(dir.join("frame_000001.fits")).unwrap(), b"SIM", "a refused copy must be left as it was");
}

#[test]
fn a_set_missing_a_frame_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let dir = extracted(root.path(), "set");
    fs::remove_file(dir.join("session").join("frame_000002.fits")).unwrap();
    assert!(check_fixture_contents(&dir, &SET).is_err());
}

/// The verdict is the set's own, so nothing left behind by a deleted download (as the
/// `.source` marks were) can pass the next copy put in its place by hand.
#[test]
fn a_deleted_set_replaced_by_hand_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let dir = extracted(root.path(), "set");
    assert_eq!(check_fixture_contents(&dir, &SET), Ok(()));
    fs::remove_dir_all(&dir).unwrap();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("frame_000001.fits"), b"HAND-CUT").unwrap();
    assert!(check_fixture_contents(&dir, &SET).is_err());
}

/// nextest runs each test in its own process, so two may download one set: the one that
/// loses the rename must leave the live set alone, or a third test reads it mid-change.
#[test]
fn a_losing_publisher_never_disturbs_a_reader() {
    let root = tempfile::tempdir().unwrap();
    let dir = extracted(root.path(), "set");
    let loser = root.path().join(".set.loser");
    extracted(&loser, "set");
    let stop = std::sync::atomic::AtomicBool::new(false);
    let refusals = std::thread::scope(|s| {
        s.spawn(|| {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                publish_staged(&loser, "set", &dir).unwrap();
            }
        });
        let refusals = (0..5_000).filter(|_| check_fixture_contents(&dir, &SET).is_err()).count();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        refusals
    });
    assert_eq!(refusals, 0, "a published set was refused {refusals} times while a loser published it again");
    assert!(loser.join("set").is_dir(), "the loser's staging is its own to clean up");
}

#[test]
fn every_registered_set_is_named_once_and_holds_something() {
    for (i, fixture) in DEFAULT_FIXTURES.iter().enumerate() {
        assert!(fixture.contents.files > 0 && fixture.contents.bytes > 0, "{} registers no contents", fixture.name);
        let twin = DEFAULT_FIXTURES[i + 1..].iter().any(|other| other.name == fixture.name);
        assert!(!twin, "{} is registered twice", fixture.name);
    }
}
