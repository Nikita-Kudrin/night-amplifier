//! Writes that survive a power cut, not just a successful `write`.
//!
//! A `write` returns once the bytes are in the OS's page cache; Linux writes them out up to
//! ~30 s later. 2026-10-16's Orange Pi lost power 2-13 s after a Stop and kept a 0-byte
//! stacked FITS, PNG and three raw frames, all logged "written successfully". `sync_all`
//! returns only once the device acknowledged a cache flush (`F_FULLFSYNC` on macOS,
//! `FlushFileBuffers` on Windows). A new file's *name* lives in its directory, which needs
//! its own sync on Unix; Windows cannot open a directory for that and journals it anyway.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing::warn;

/// One atomic write at a time, process-wide. Every writer of a path stages it in the same
/// `<name>.tmp`, and two at once let one truncate the other's staged file just before its
/// rename: a reader saw a 0-byte `settings.json` hundreds of times a second. Config writes
/// are rare, so one lock serves every path.
static WRITES: Mutex<()> = Mutex::new(());

/// Wait until the file at `path` is on the storage device. Opened for writing, never
/// truncated: Windows refuses to flush a handle opened read-only.
pub fn sync_file(path: &Path) -> io::Result<()> {
    OpenOptions::new().write(true).open(path)?.sync_all()
}

/// Wait until `dir`'s entries — the names of the files created in it — are on the device.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        tolerate_unsupported(File::open(dir)?.sync_all(), dir)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// FUSE exFAT/NTFS and some network mounts refuse `fsync` on a directory (`EINVAL`,
/// `ENOTSUP`), leaving names to their own writeback; SQLite treats that as done. Failing on
/// it would call every capture saved to such a USB drive "not saved".
#[cfg(unix)]
fn tolerate_unsupported(result: io::Result<()>, dir: &Path) -> io::Result<()> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);

    match result {
        Err(e) if matches!(e.kind(), io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported) => {
            if !WARNED.swap(true, Ordering::Relaxed) {
                warn!(dir = %dir.display(), error = %e, "This filesystem cannot sync a directory; new file names rely on its writeback");
            }
            Ok(())
        }
        other => other,
    }
}

/// Replace the file at `path` with `contents`, so that after a crash it holds either the old
/// contents or the new — never a truncated mix or nothing, which an in-place `fs::write`
/// leaves. Written beside it as `<name>.tmp`, synced, renamed over it, then the directory
/// synced so the rename itself is durable. Blocks for the device: keep it off the runtime.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let _serial = WRITES.lock().unwrap_or_else(|e| e.into_inner());
    replace(path, contents)
}

/// [`write_atomic`] with the contents built under its lock, for a caller writing a snapshot
/// of shared state: taken outside, two racing writers could land the older snapshot last.
pub fn write_atomic_with(path: &Path, contents: impl FnOnce() -> io::Result<Vec<u8>>) -> io::Result<()> {
    let _serial = WRITES.lock().unwrap_or_else(|e| e.into_inner());
    replace(path, &contents()?)
}

fn replace(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = temporary_path(path);
    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(contents)?;
        file.sync_all()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&temporary, path)) {
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }
    // The new contents are in place either way: a directory sync that fails leaves only the
    // rename to the kernel's writeback, which is no reason to tell the caller nothing was saved.
    if let Err(e) = sync_dir(parent_dir(path)) {
        warn!(path = %path.display(), error = %e, "Replaced, but its directory could not be synced");
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// `path`'s directory; `.` for a bare file name, whose parent is the empty path.
pub(crate) fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"old").unwrap();

        write_atomic(&path, b"new").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from("settings.json")]);
    }

    /// A write that cannot finish must not cost the file it was replacing — `fs::write`
    /// truncates first, so a full disk or a power cut there left settings at their defaults.
    #[test]
    fn a_failed_atomic_write_keeps_the_previous_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"old").unwrap();
        // A directory where the temporary goes: creating it fails, as a full disk would.
        std::fs::create_dir(temporary_path(&path)).unwrap();

        assert!(write_atomic(&path, b"new").is_err());

        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn an_atomic_write_creates_a_missing_file_and_a_bare_name_resolves_to_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("push_to_state.json");

        write_atomic(&path, b"{}").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        assert_eq!(parent_dir(Path::new("settings.json")), Path::new("."));
        assert_eq!(temporary_path(&path), dir.path().join("push_to_state.json.tmp"));
    }

    /// `save_settings` runs from the settings service, a reconnect, an install and Focus
    /// mode, and Pro's Push-To state flushes from two threads: writers of one path overlap.
    /// Each write must succeed, and a reader must never see anything but one writer's whole
    /// contents — the guarantee `write_atomic` exists for.
    #[test]
    fn overlapping_atomic_writes_to_one_path_all_succeed_and_never_tear() {
        use std::sync::atomic::{AtomicBool, Ordering};

        const WRITERS: u8 = 6;
        const ROUNDS: usize = 150;
        const SIZE: usize = 4 * 1024;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        write_atomic(&path, &[b'0'; SIZE]).unwrap();
        let done = AtomicBool::new(false);

        let (failures, torn) = std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                let mut torn = Vec::new();
                while !done.load(Ordering::Relaxed) {
                    let seen = std::fs::read(&path).unwrap();
                    if seen.len() != SIZE || seen.iter().any(|byte| *byte != seen[0]) {
                        torn.push(seen.len());
                    }
                }
                torn
            });
            let writers: Vec<_> = (0..WRITERS)
                .map(|writer| {
                    let path = &path;
                    scope.spawn(move || {
                        let contents = vec![b'a' + writer; SIZE];
                        (0..ROUNDS).filter_map(|_| write_atomic(path, &contents).err().map(|e| e.to_string())).collect::<Vec<_>>()
                    })
                })
                .collect();
            let failures: Vec<String> = writers.into_iter().flat_map(|writer| writer.join().unwrap()).collect();
            done.store(true, Ordering::Relaxed);
            (failures, reader.join().unwrap())
        });

        assert!(torn.is_empty(), "a reader saw {} partial files, lengths {:?}", torn.len(), &torn[..torn.len().min(5)]);
        assert!(failures.is_empty(), "{} writes failed, first: {}", failures.len(), failures[0]);
    }

    /// The snapshot is taken under the write lock, so of two racing writers the one that
    /// writes last also built its contents last.
    #[test]
    fn contents_built_under_the_lock_land_in_the_order_they_were_built() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("push_to_state.json");
        let version = AtomicU32::new(0);

        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..20 {
                        write_atomic_with(&path, || Ok(version.fetch_add(1, Ordering::SeqCst).to_string().into_bytes())).unwrap();
                    }
                });
            }
        });

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "159");
    }

    #[test]
    fn contents_that_cannot_be_built_leave_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"old").unwrap();

        assert!(write_atomic_with(&path, || Err(io::Error::other("not serialisable"))).is_err());

        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert!(!temporary_path(&path).exists());
    }

    /// FUSE exFAT/NTFS answer a directory `fsync` with `EINVAL` or `ENOTSUP`: no reason to
    /// call a capture unsaved. Every other error still is one.
    #[cfg(unix)]
    #[test]
    fn a_filesystem_that_cannot_sync_a_directory_is_not_a_failure() {
        let dir = Path::new("captures/raw");
        let refused = |kind| Err(io::Error::from(kind));

        tolerate_unsupported(refused(io::ErrorKind::InvalidInput), dir).unwrap();
        tolerate_unsupported(refused(io::ErrorKind::Unsupported), dir).unwrap();
        let eio = tolerate_unsupported(Err(io::Error::from_raw_os_error(5)), dir).unwrap_err();
        assert_eq!(eio.raw_os_error(), Some(5));
    }

    #[test]
    fn syncing_a_missing_file_is_an_error_not_a_silent_success() {
        let dir = tempfile::tempdir().unwrap();

        assert!(sync_file(&dir.path().join("frame_000164.fits")).is_err());
        sync_file(&{
            let path = dir.path().join("frame_000163.fits");
            std::fs::write(&path, b"SIMPLE").unwrap();
            path
        })
        .unwrap();
        sync_dir(dir.path()).unwrap();
    }
}
