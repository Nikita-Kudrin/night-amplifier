//! Replays shared across one test run: a session several tests replay alike is stacked
//! once, stored in the run's directory and read back by the rest, bit for bit.
//!
//! **One run, never reused**: a stack from an earlier build would pass for this one's. The
//! directory is nextest's `NEXTEST_RUN_ID`; a plain `cargo test` process makes a fresh one (a
//! bare pid repeats in a container, and served an earlier build's entries). Each run removes
//! every other run's directory no live process holds. `NA_REPLAY_CACHE` moves the root, turns
//! the cache `off`, or fails a test reaching it (`elsewhere`: CI's other partitions). A key is
//! what a replay reads but the code: file names and sizes, settings, each slot's plugin type,
//! the licence, [`KEY_ENV`]. Not a plugin's own state (a test configuring one differently
//! replays uncached), nor the memory probation reads (`probation::affordable`). One process
//! makes a key under its file lock; the others wait, then read it under a shared one.
//!
//! Like `instruments`, nothing here may name `crate::`.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::cell::Cell;
use std::sync::OnceLock;

use night_amplifier::registration::AffineTransform;
use night_amplifier::session::capture::pipeline::StackingOutcome;
use night_amplifier::session::capture::{FrameAdmission, RejectionReason, Restacked};
use night_amplifier::{Frame, NoiseField};

use super::{AccumulatorSnapshot, ReplayRecord};

const MAGIC: &[u8; 4] = b"NARC";
/// Bump with any change to what an entry holds or how it is written.
const VERSION: u32 = 1;
pub const RUN_LOCK: &str = ".run.lock";
/// Variables that change a stack without changing its inputs.
pub const KEY_ENV: &[&str] = &["NA_NO_REJECTION", "RAYON_NUM_THREADS"];
const FLOATS_PER_CHUNK: usize = 1 << 16;

/// What a replay depends on besides the code, as text. Stored in its entry and checked on
/// every read, so two keys sharing a file name cannot serve each other.
pub struct Key {
    text: String,
}

impl Key {
    pub fn new(engine: &str) -> Self {
        let key = Self { text: format!("{engine} v{VERSION}\n") };
        KEY_ENV.iter().fold(key, |key, name| key.with(name, std::env::var(name).unwrap_or_default()))
    }

    pub fn with(mut self, name: &str, value: impl std::fmt::Display) -> Self {
        self.text.push_str(&format!("{name}={value}\n"));
        self
    }

    /// Each file by path and size.
    pub fn files(self, files: &[PathBuf]) -> Self {
        files.iter().fold(self, |key, file| {
            let len = fs::metadata(file).map_or(0, |m| m.len());
            key.with("file", format!("{} {len}", file.display()))
        })
    }

    pub fn settings(self, settings: &night_amplifier::session::state::CaptureSettings) -> Self {
        self.with("settings", canonical(&serde_json::to_value(settings).expect("settings serialise")))
    }

    /// Each filled slot's plugin type, and whether the accessors answer.
    pub fn plugins(self, plugins: &night_amplifier::plugins::Plugins) -> Self {
        self.with("plugins", format!("{:?}", plugins.implementations())).with("licensed", plugins.licensed())
    }

    pub fn file_name(&self, label: &str) -> String {
        let label: String = label.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        format!("{label}-{:016x}.rec", super::fnv(self.text.as_bytes()))
    }
}

/// `value` as JSON with every object's keys sorted: the settings' maps are `HashMap`s,
/// whose order differs from process to process.
fn canonical(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let fields: Vec<String> = entries.iter().map(|(k, v)| format!("{k:?}:{}", canonical(v))).collect();
            format!("{{{}}}", fields.join(","))
        }
        serde_json::Value::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// Whether replays are shared in this process (`NA_REPLAY_CACHE` neither `off` nor
/// `elsewhere`, its directory usable).
pub fn enabled() -> bool {
    !elsewhere() && run_dir().is_some()
}

thread_local! {
    static READS: Cell<usize> = const { Cell::new(0) };
}

/// Entries this thread has read back rather than made: how a test tells a shared replay
/// from one that only looks shared because every write failed.
pub fn reads() -> usize {
    READS.with(Cell::get)
}

/// `make()`'s value, read from this run's entry for `key` once a test has stored one. A
/// failure on the way only loses the sharing: the value is made here, as without a cache.
pub fn cached<T: Record>(key: &Key, label: &str, make: impl FnOnce() -> T) -> T {
    assert!(
        !elsewhere(),
        "{label}: this test replays through the run's cache, which this CI partition leaves to \
         another runner: add its module to REPLAY_MODULES in .github/workflows/ci.yml"
    );
    let Some(dir) = run_dir() else {
        return make();
    };
    let path = dir.join(key.file_name(label));
    let lock = match File::create(path.with_extension("lock")).and_then(|f| f.lock_shared().map(|()| f)) {
        Ok(lock) => lock,
        Err(e) => {
            println!("  replay cache: {label} not shared: {e}");
            return make();
        }
    };
    if let Some(value) = read_entry(&path, key, label) {
        return value;
    }
    // Not stored yet: wait for the writer's lock, since another process may be making it.
    if let Err(e) = lock.unlock().and_then(|()| lock.lock()) {
        println!("  replay cache: {label} not shared: {e}");
        return make();
    }
    if let Some(value) = read_entry(&path, key, label) {
        return value;
    }
    let value = make();
    if let Err(e) = write_entry(&path, key, &value) {
        println!("  replay cache: {label} not stored: {e}");
    }
    drop(lock);
    value
}

fn elsewhere() -> bool {
    std::env::var("NA_REPLAY_CACHE").is_ok_and(|v| v == "elsewhere")
}

/// `value` as an entry holds it: two values encode alike only if every field is identical.
pub fn encoded<T: Record>(value: &T) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    value.put(&mut w).expect("a Vec takes every write");
    w.0
}

pub fn decoded<T: Record>(bytes: &[u8]) -> io::Result<T> {
    T::take(&mut Reader(bytes))
}

fn run_dir() -> Option<&'static Path> {
    static RUN: OnceLock<Option<Run>> = OnceLock::new();
    RUN.get_or_init(|| {
        let root = match std::env::var("NA_REPLAY_CACHE") {
            Ok(v) if v == "off" || v == "elsewhere" => return None,
            Ok(v) => PathBuf::from(v),
            Err(_) => Path::new(env!("CARGO_TARGET_TMPDIR")).join("replay-cache"),
        };
        let nextest = std::env::var("NEXTEST_RUN_ID").ok();
        Run::open(&root, nextest.as_deref())
            .inspect_err(|e| println!("  replay cache: off, {} is unusable: {e}", root.display()))
            .ok()
    })
    .as_ref()
    .map(|run| run.dir.as_path())
}

/// A run's directory, its lock held shared while this value lives, so no other run sweeps
/// it meanwhile.
pub struct Run {
    pub dir: PathBuf,
    _lock: File,
}

impl Run {
    /// Joins nextest's run `nextest` under `root`, or without one makes this process a
    /// directory no earlier one had, then sweeps every other run no live process holds.
    pub fn open(root: &Path, nextest: Option<&str>) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        let dir = match nextest {
            Some(run) => {
                let dir = root.join(run);
                fs::create_dir_all(&dir)?;
                dir
            }
            None => tempfile::Builder::new()
                .prefix(&format!("pid-{}-", std::process::id()))
                .tempdir_in(root)?
                .keep(),
        };
        let lock = File::create(dir.join(RUN_LOCK))?;
        lock.lock_shared()?;
        sweep(root, &dir);
        Ok(Self { dir, _lock: lock })
    }
}

/// Removes every other run's directory no live process holds. A run caught between two
/// tests holds none and loses its entries, so it replays them again: nothing worse.
fn sweep(root: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for dir in entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p != current && p.is_dir()) {
        let Ok(lock) = File::open(dir.join(RUN_LOCK)) else {
            continue; // still being created
        };
        if lock.try_lock().is_ok() {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

/// `None` until an entry is stored. An entry this run wrote that does not decode is a
/// codec bug, never data to step around.
fn read_entry<T: Record>(path: &Path, key: &Key, label: &str) -> Option<T> {
    let mut r = Reader(BufReader::new(File::open(path).ok()?));
    let decode = |r: &mut Reader<_>| -> io::Result<Option<T>> {
        if r.array::<4>()? != *MAGIC || r.u32()? != VERSION {
            return Err(invalid("not this build's format"));
        }
        if r.text()? != key.text {
            return Ok(None); // another key with this file name: replayed here and stored over it
        }
        T::take(r).map(Some)
    };
    let value =
        decode(&mut r).unwrap_or_else(|e| panic!("replay cache entry {} does not decode: {e}", path.display()))?;
    READS.with(|reads| reads.set(reads.get() + 1));
    println!("  replay cache: {label} replayed earlier in this run");
    Some(value)
}

/// Staged under the process's own name and renamed into place, so a reader only ever
/// opens a whole entry.
fn write_entry<T: Record>(path: &Path, key: &Key, value: &T) -> io::Result<()> {
    let staging = path.with_extension(format!("tmp{}", std::process::id()));
    let written = (|| {
        let mut w = Writer(BufWriter::new(File::create(&staging)?));
        w.0.write_all(MAGIC)?;
        w.u32(VERSION)?;
        w.text(&key.text)?;
        value.put(&mut w)?;
        w.0.into_inner().map_err(|e| e.into_error())?;
        fs::rename(&staging, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&staging);
    }
    written
}

fn invalid(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

pub struct Writer<W: Write>(W);

impl<W: Write> Writer<W> {
    fn u64(&mut self, v: u64) -> io::Result<()> {
        self.0.write_all(&v.to_le_bytes())
    }

    fn u32(&mut self, v: u32) -> io::Result<()> {
        self.0.write_all(&v.to_le_bytes())
    }

    fn usize(&mut self, v: usize) -> io::Result<()> {
        self.u64(v as u64)
    }

    fn f32(&mut self, v: f32) -> io::Result<()> {
        self.0.write_all(&v.to_le_bytes())
    }

    fn bool(&mut self, v: bool) -> io::Result<()> {
        self.0.write_all(&[u8::from(v)])
    }

    fn text(&mut self, s: &str) -> io::Result<()> {
        self.usize(s.len())?;
        self.0.write_all(s.as_bytes())
    }

    fn floats(&mut self, values: &[f32]) -> io::Result<()> {
        self.usize(values.len())?;
        let mut bytes = Vec::with_capacity(FLOATS_PER_CHUNK * 4);
        for chunk in values.chunks(FLOATS_PER_CHUNK) {
            bytes.clear();
            bytes.extend(chunk.iter().flat_map(|v| v.to_le_bytes()));
            self.0.write_all(&bytes)?;
        }
        Ok(())
    }
}

pub struct Reader<R: Read>(R);

impl<R: Read> Reader<R> {
    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        let mut bytes = [0; N];
        self.0.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn usize(&mut self) -> io::Result<usize> {
        usize::try_from(self.u64()?).map_err(invalid)
    }

    fn f32(&mut self) -> io::Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn bool(&mut self) -> io::Result<bool> {
        match self.array::<1>()? {
            [0] => Ok(false),
            [1] => Ok(true),
            [b] => Err(invalid(format!("{b} is not a bool"))),
        }
    }

    fn text(&mut self) -> io::Result<String> {
        let mut bytes = vec![0; self.usize()?];
        self.0.read_exact(&mut bytes)?;
        String::from_utf8(bytes).map_err(invalid)
    }

    /// A chunk at a time: one whole-frame byte buffer beside the floats doubled the
    /// memory a read of a 3008² stack peaks at.
    fn floats(&mut self) -> io::Result<Vec<f32>> {
        let len = self.usize()?;
        let mut values = Vec::with_capacity(len);
        let mut bytes = vec![0; len.min(FLOATS_PER_CHUNK) * 4];
        while values.len() < len {
            let chunk = &mut bytes[..(len - values.len()).min(FLOATS_PER_CHUNK) * 4];
            self.0.read_exact(chunk)?;
            values.extend(chunk.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
        }
        Ok(values)
    }
}

/// A value an entry can hold, field by field. The impls destructure what they write, so a
/// field added to an outcome does not compile until it is stored. `NoiseField` and `Frame`
/// keep theirs private: a field added there must be added here by hand.
pub trait Record: Sized {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()>;
    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self>;
}

impl<T: Record> Record for Option<T> {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        w.bool(self.is_some())?;
        self.as_ref().map_or(Ok(()), |v| v.put(w))
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        if r.bool()? { T::take(r).map(Some) } else { Ok(None) }
    }
}

impl<T: Record> Record for Vec<T> {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        w.usize(self.len())?;
        self.iter().try_for_each(|v| v.put(w))
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        (0..r.usize()?).map(|_| T::take(r)).collect()
    }
}

impl Record for Frame {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        w.usize(self.width())?;
        w.usize(self.height())?;
        w.usize(self.channels())?;
        w.floats(self.data())
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        let (width, height, channels) = (r.usize()?, r.usize()?, r.usize()?);
        Frame::from_f32_vec(r.floats()?, width, height, channels).map_err(invalid)
    }
}

impl Record for NoiseField {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        for n in [self.width(), self.height(), self.channels(), self.source_width(), self.source_height()] {
            w.usize(n)?;
        }
        w.bool(self.variance().is_some())?;
        if let Some(variance) = self.variance() {
            w.floats(variance)?;
        }
        w.floats(self.coverage())
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        let [width, height, channels, source_width, source_height] =
            [r.usize()?, r.usize()?, r.usize()?, r.usize()?, r.usize()?];
        let variance = if r.bool()? { Some(r.floats()?) } else { None };
        let coverage = r.floats()?;
        match variance {
            Some(variance) => NoiseField::new(variance, width, height, channels, source_width, source_height)
                .and_then(|field| field.with_coverage(coverage)),
            None => NoiseField::coverage_only(coverage, width, height, channels, source_width, source_height),
        }
        .map_err(invalid)
    }
}

impl Record for AffineTransform {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        let AffineTransform { rotation, scale, tx, ty } = *self;
        [rotation, scale, tx, ty].into_iter().try_for_each(|v| w.f32(v))
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        Ok(AffineTransform { rotation: r.f32()?, scale: r.f32()?, tx: r.f32()?, ty: r.f32()? })
    }
}

impl Record for Restacked {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        let Restacked { retracted, reinstated, rereferenced } = *self;
        w.usize(retracted)?;
        w.usize(reinstated)?;
        w.bool(rereferenced)
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        Ok(Restacked { retracted: r.usize()?, reinstated: r.usize()?, rereferenced: r.bool()? })
    }
}

impl Record for RejectionReason {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        use RejectionReason::*;
        let code: u32 = match self {
            NoStars => 0,
            TooFewStars => 1,
            RegistrationFailed => 2,
            TooFewCorrespondences => 3,
            ResidualTooHigh => 4,
            StarsTooLarge => 5,
            StarsDoubled => 6,
            StackerError => 7,
            Outvoted => 8,
        };
        w.u32(code)
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        use RejectionReason::*;
        Ok(match r.u32()? {
            0 => NoStars,
            1 => TooFewStars,
            2 => RegistrationFailed,
            3 => TooFewCorrespondences,
            4 => ResidualTooHigh,
            5 => StarsTooLarge,
            6 => StarsDoubled,
            7 => StackerError,
            8 => Outvoted,
            code => return Err(invalid(format!("no rejection reason {code}"))),
        })
    }
}

impl Record for FrameAdmission {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        let FrameAdmission { added, rejected_because, rebased, matched_stars, mean_residual, transform, restacked } =
            *self;
        w.bool(added)?;
        rejected_because.put(w)?;
        w.bool(rebased)?;
        w.usize(matched_stars)?;
        w.f32(mean_residual)?;
        transform.put(w)?;
        restacked.put(w)
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        Ok(FrameAdmission {
            added: r.bool()?,
            rejected_because: Record::take(r)?,
            rebased: r.bool()?,
            matched_stars: r.usize()?,
            mean_residual: r.f32()?,
            transform: Record::take(r)?,
            restacked: Record::take(r)?,
        })
    }
}

impl Record for StackingOutcome {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        let StackingOutcome { display_frame, showing_stack, frame_added, stack_reset, admission, stack_depth, noise } =
            self;
        display_frame.put(w)?;
        w.bool(*showing_stack)?;
        w.bool(*frame_added)?;
        w.bool(*stack_reset)?;
        admission.put(w)?;
        w.u32(*stack_depth)?;
        noise.put(w)
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        Ok(StackingOutcome {
            display_frame: Record::take(r)?,
            showing_stack: r.bool()?,
            frame_added: r.bool()?,
            stack_reset: r.bool()?,
            admission: Record::take(r)?,
            stack_depth: r.u32()?,
            noise: Record::take(r)?,
        })
    }
}

impl Record for AccumulatorSnapshot {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        let AccumulatorSnapshot { depth, frame, noise, measured, coverage, generation } = self;
        w.usize(*depth)?;
        frame.put(w)?;
        noise.put(w)?;
        measured.put(w)?;
        coverage.put(w)?;
        w.u64(*generation)
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        Ok(AccumulatorSnapshot {
            depth: r.usize()?,
            frame: Record::take(r)?,
            noise: Record::take(r)?,
            measured: Record::take(r)?,
            coverage: Record::take(r)?,
            generation: r.u64()?,
        })
    }
}

impl Record for ReplayRecord {
    fn put<W: Write>(&self, w: &mut Writer<W>) -> io::Result<()> {
        let ReplayRecord { subs, depth, outcomes, stack, coverage } = self;
        w.usize(*subs)?;
        w.usize(*depth)?;
        outcomes.put(w)?;
        stack.put(w)?;
        coverage.put(w)
    }

    fn take<R: Read>(r: &mut Reader<R>) -> io::Result<Self> {
        Ok(ReplayRecord {
            subs: r.usize()?,
            depth: r.usize()?,
            outcomes: Record::take(r)?,
            stack: Record::take(r)?,
            coverage: Record::take(r)?,
        })
    }
}
