//! Who is watching a stream, per payload family.
//!
//! Every client of a family receives the same payload, sized by a setting rather than by
//! the client (see [`crate::state::Resolution`]), so the only thing the producer
//! needs to know about clients is whether any are connected: a family nobody watches is
//! not encoded at all.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::FrameStream;

/// The two payload families a stream can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    /// Dynamic JPEG (SA10) — `/ws/stream` and `/ws/eyepiece`, sized by Streaming Resolution.
    Jpeg,
    /// RGB8+LZ4 (SA09) — `/ws/eyepiece_quality`, sized by Eyepiece Streaming Resolution.
    Lossless,
}

impl StreamKind {
    pub const COUNT: usize = 2;

    pub const fn all() -> [StreamKind; Self::COUNT] {
        [Self::Jpeg, Self::Lossless]
    }

    /// Stable label for logs and telemetry attributes.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Lossless => "lossless",
        }
    }
}

/// Counts one client of a stream family for as long as it is alive.
///
/// The producer skips a family with no viewers — and the guide loop renders nothing at all
/// without any — so the decrement must happen even when a handler exits early or panics,
/// hence a `Drop` guard rather than manual bookkeeping.
pub struct ViewerGuard {
    stream: Arc<FrameStream>,
    kind: StreamKind,
}

impl ViewerGuard {
    pub fn new(stream: Arc<FrameStream>, kind: StreamKind) -> Self {
        stream.viewer_counter(kind).fetch_add(1, Ordering::SeqCst);
        Self { stream, kind }
    }
}

impl Drop for ViewerGuard {
    fn drop(&mut self) {
        self.stream.viewer_counter(self.kind).fetch_sub(1, Ordering::SeqCst);
    }
}

/// Every open image-stream connection, by page, for `GET /api/streams/viewers`: a kiosk's
/// watchdog asks whether its own page is alive. A crashed tab closes its socket; a hung one
/// stops pinging (every 30 s), and the handler, stuck sending it a frame, stops reading:
/// either way `heard_at` stops moving.
#[derive(Debug, Default)]
pub struct StreamClients {
    clients: Mutex<HashMap<u64, StreamClientInfo>>,
    next_id: AtomicU64,
}

/// One connection, as [`StreamClients::snapshot`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamClientInfo {
    /// The frontend page that opened the socket (`/eyepiece_quality`).
    pub page: &'static str,
    /// From this machine (loopback): the kiosk's own browser, not a phone on the Wi-Fi.
    pub local: bool,
    pub connected_at: Instant,
    /// The last message the client sent, or its connect.
    pub heard_at: Instant,
}

impl StreamClients {
    /// Lists the connection until the returned registration drops.
    pub fn register(self: &Arc<Self>, page: &'static str, local: bool) -> StreamClientRegistration {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        let info = StreamClientInfo { page, local, connected_at: now, heard_at: now };
        self.lock().insert(id, info);
        StreamClientRegistration { clients: Arc::clone(self), id }
    }

    pub fn snapshot(&self) -> Vec<StreamClientInfo> {
        self.lock().values().copied().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, StreamClientInfo>> {
        self.clients.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A connection's entry in [`StreamClients`], removed on drop (an unwinding handler too).
#[derive(Debug)]
pub struct StreamClientRegistration {
    clients: Arc<StreamClients>,
    id: u64,
}

impl StreamClientRegistration {
    /// The client sent something: it is alive.
    pub fn heard(&self) {
        if let Some(info) = self.clients.lock().get_mut(&self.id) {
            info.heard_at = Instant::now();
        }
    }
}

impl Drop for StreamClientRegistration {
    fn drop(&mut self) {
        self.clients.lock().remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registration_lists_its_client_until_dropped() {
        let clients = Arc::new(StreamClients::default());
        let kiosk = clients.register("/eyepiece_quality", true);
        let phone = clients.register("/eyepiece", false);
        let mut listed = clients.snapshot();
        listed.sort_by_key(|info| info.page);
        assert_eq!(
            listed.iter().map(|info| (info.page, info.local)).collect::<Vec<_>>(),
            [("/eyepiece", false), ("/eyepiece_quality", true)]
        );

        drop(phone);
        assert_eq!(clients.snapshot().len(), 1);
        drop(kiosk);
        assert!(clients.snapshot().is_empty());
    }

    #[test]
    fn hearing_from_a_client_moves_only_its_own_entry() {
        let clients = Arc::new(StreamClients::default());
        let _quiet = clients.register("/eyepiece", false);
        let talking = clients.register("/eyepiece_quality", true);
        std::thread::sleep(std::time::Duration::from_millis(5));

        talking.heard();

        for info in clients.snapshot() {
            let moved = info.heard_at > info.connected_at;
            assert_eq!(moved, info.page == "/eyepiece_quality", "{info:?}");
        }
    }

    #[test]
    fn a_panicking_handler_still_removes_its_client() {
        let clients = Arc::new(StreamClients::default());
        let handle = {
            let clients = Arc::clone(&clients);
            std::thread::spawn(move || {
                let _registration = clients.register("/eyepiece_quality", true);
                panic!("intentional: checking the registration is removed while unwinding");
            })
        };
        assert!(handle.join().is_err());
        assert!(clients.snapshot().is_empty());
    }

    #[test]
    fn a_guard_counts_its_viewer_until_dropped() {
        let stream = Arc::new(FrameStream::default());
        let first = ViewerGuard::new(Arc::clone(&stream), StreamKind::Jpeg);
        let second = ViewerGuard::new(Arc::clone(&stream), StreamKind::Jpeg);
        assert_eq!(stream.viewer_count(StreamKind::Jpeg), 2);

        drop(first);
        assert_eq!(stream.viewer_count(StreamKind::Jpeg), 1);
        drop(second);
        assert_eq!(stream.viewer_count(StreamKind::Jpeg), 0);
    }

    /// Unwinding through the guard still releases the viewer, or a panicked handler
    /// would keep the producer encoding for nobody forever.
    #[test]
    fn a_panicking_handler_still_releases_its_viewer() {
        let stream = Arc::new(FrameStream::default());
        let handle = {
            let stream = Arc::clone(&stream);
            std::thread::spawn(move || {
                let _guard = ViewerGuard::new(stream, StreamKind::Lossless);
                panic!("intentional: checking the guard releases the viewer while unwinding");
            })
        };
        assert!(handle.join().is_err());
        assert_eq!(stream.viewer_count(StreamKind::Lossless), 0);
    }

    /// A JPEG client must not make the producer encode the lossless family, or the reverse.
    #[test]
    fn families_count_viewers_independently() {
        let stream = Arc::new(FrameStream::default());
        let _jpeg = ViewerGuard::new(Arc::clone(&stream), StreamKind::Jpeg);
        assert_eq!(stream.viewer_count(StreamKind::Lossless), 0);

        let _lossless = ViewerGuard::new(Arc::clone(&stream), StreamKind::Lossless);
        assert_eq!(stream.viewer_count(StreamKind::Jpeg), 1);
        assert_eq!(stream.viewer_count(StreamKind::Lossless), 1);
    }

    /// The guide loop's render gate: either family counts as someone watching.
    #[test]
    fn has_viewers_tracks_both_families() {
        let stream = Arc::new(FrameStream::default());
        assert!(!stream.has_viewers());

        let jpeg = ViewerGuard::new(Arc::clone(&stream), StreamKind::Jpeg);
        assert!(stream.has_viewers());
        drop(jpeg);
        assert!(!stream.has_viewers());

        let lossless = ViewerGuard::new(Arc::clone(&stream), StreamKind::Lossless);
        assert!(stream.has_viewers());
        drop(lossless);
        assert!(!stream.has_viewers());
    }

    /// Many handlers registering and leaving at once must net to zero.
    #[test]
    fn concurrent_joins_and_leaves_balance() {
        let stream = Arc::new(FrameStream::default());
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let stream = Arc::clone(&stream);
                std::thread::spawn(move || {
                    let kind = StreamKind::all()[i % StreamKind::COUNT];
                    for _ in 0..1000 {
                        drop(ViewerGuard::new(Arc::clone(&stream), kind));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(!stream.has_viewers());
    }
}
