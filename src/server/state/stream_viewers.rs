//! Who is watching a stream, per payload family.
//!
//! Every client of a family receives the same payload, sized by a setting rather than by
//! the client (see [`crate::server::state::Resolution`]), so the only thing the producer
//! needs to know about clients is whether any are connected: a family nobody watches is
//! not encoded at all.

use std::sync::atomic::Ordering;
use std::sync::Arc;

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

#[cfg(test)]
mod tests {
    use super::*;

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
