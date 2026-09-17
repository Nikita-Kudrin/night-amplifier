//! One independent image stream: the payloads, the counter that versions them, and the
//! viewer census that decides which of them are worth producing.
//!
//! There is one per camera role. Two producers sharing a counter is not a small
//! inefficiency but a correctness bug: a payload is served only while its tag matches the
//! published counter, so guide frames advancing the main counter would invalidate every
//! main-stream payload (and vice versa) and wake both sets of clients on every frame.
//!
//! Each family ([`StreamKind`]) keeps exactly one payload: every client of a family gets
//! the same bytes, at the size its streaming-resolution setting chose.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::sync::{watch, Mutex, RwLock};

use super::{RenderReadyFrame, StreamKind};
use crate::telemetry::metrics as telemetry_metrics;

/// Everything singular about one stream of rendered frames.
pub struct FrameStream {
    /// Latest frame in linear form, for encoding a payload on demand.
    latest_raw_frame: RwLock<Option<Arc<RenderReadyFrame>>>,
    /// Versions every payload. Claimed by [`Self::begin_frame`] before the payloads are
    /// stored and published by [`Self::publish_frame`] once they are, so a woken client
    /// never observes a counter whose payloads are still missing.
    frame_counter: AtomicU64,
    /// Carries the counter of the most recently published frame.
    ///
    /// A `watch` channel rather than a `Notify`: `Notify::notify_waiters` wakes only
    /// the tasks registered at that instant and stores no permit, so every frame
    /// published while a handler sat in `socket.send().await` was lost. A watch
    /// receiver latches the version instead, so a send that lands between two polls
    /// makes the next `changed()` return immediately.
    frame_ready: watch::Sender<u64>,
    /// Clients per family. The producer encodes only families somebody watches — and,
    /// for the guide stream, only renders at all when somebody does.
    viewers: [AtomicUsize; StreamKind::COUNT],
    /// The one payload per family, tagged with the counter it was encoded from.
    payloads: [StdRwLock<Option<(u64, bytes::Bytes)>>; StreamKind::COUNT],
    /// Serialises on-demand encodes per family, so clients arriving together share one
    /// encode instead of each converting (and allocating denoise buffers for) the frame.
    on_demand_encodes: [Mutex<()>; StreamKind::COUNT],
}

impl Default for FrameStream {
    fn default() -> Self {
        Self {
            latest_raw_frame: RwLock::new(None),
            frame_counter: AtomicU64::new(0),
            frame_ready: watch::channel(0).0,
            viewers: std::array::from_fn(|_| AtomicUsize::new(0)),
            payloads: std::array::from_fn(|_| StdRwLock::new(None)),
            on_demand_encodes: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}

impl FrameStream {
    /// Subscribe to frame publications.
    ///
    /// The receiver starts with the current counter already marked seen, so the first
    /// `changed()` resolves on the next frame and not on the backlog.
    pub fn subscribe_frames(&self) -> watch::Receiver<u64> {
        self.frame_ready.subscribe()
    }

    /// The frame version currently published.
    pub fn frame_counter(&self) -> u64 {
        self.frame_counter.load(Ordering::SeqCst)
    }

    /// Claim the counter for the frame being rendered. See the struct doc for why this
    /// is split from [`Self::publish_frame`].
    pub fn begin_frame(&self) -> u64 {
        self.frame_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Wake every client waiting on a new frame.
    ///
    /// `send_replace` rather than `send`: the latter fails when nothing is subscribed,
    /// and a stream with no viewers still has to advance the version it publishes.
    pub fn publish_frame(&self) {
        telemetry_metrics::record_frame_published();
        self.frame_ready.send_replace(self.frame_counter());
    }

    /// Set the latest linear frame for on-demand encoding.
    pub async fn set_latest_raw_frame(&self, frame: Arc<RenderReadyFrame>) {
        *self.latest_raw_frame.write().await = Some(frame);
    }

    /// The most recently rendered linear frame, if any: the live preview immediately
    /// before encoding, used to serve a newly-connected client without waiting for the
    /// next exposure.
    pub async fn get_latest_raw_frame(&self) -> Option<Arc<RenderReadyFrame>> {
        self.latest_raw_frame.read().await.clone()
    }

    pub(super) fn viewer_counter(&self, kind: StreamKind) -> &AtomicUsize {
        &self.viewers[kind as usize]
    }

    /// How many clients currently watch a family.
    pub fn viewer_count(&self, kind: StreamKind) -> usize {
        self.viewer_counter(kind).load(Ordering::SeqCst)
    }

    /// Whether anyone at all is watching this stream, in either family.
    ///
    /// The guide loop's render gate: with nobody watching there is no reason to pay for
    /// background extraction, the stretch solve and an encode, so it does none of them.
    /// The solver is fed either way — see `capture::guide_task`.
    pub fn has_viewers(&self) -> bool {
        StreamKind::all().into_iter().any(|kind| self.viewer_count(kind) > 0)
    }

    /// The family's payload, if it was encoded from exactly the frame `counter`.
    pub fn payload(&self, kind: StreamKind, counter: u64) -> Option<bytes::Bytes> {
        match &*self.payloads[kind as usize].read().unwrap_or_else(|e| e.into_inner()) {
            Some((tag, payload)) if *tag == counter => Some(payload.clone()),
            _ => None,
        }
    }

    /// Store a family's payload for frame `counter` and hand back a shareable handle.
    ///
    /// A payload for an older frame than the one stored is returned untouched, so a slow
    /// on-demand encode can still send it without rolling the slot backwards. Storing
    /// does not advance the counter — see [`Self::begin_frame`].
    pub fn set_payload(
        &self,
        kind: StreamKind,
        counter: u64,
        data: impl Into<bytes::Bytes>,
    ) -> bytes::Bytes {
        let data = data.into();
        let mut slot = self.payloads[kind as usize].write().unwrap_or_else(|e| e.into_inner());
        if matches!(&*slot, Some((tag, _)) if *tag > counter) {
            return data;
        }
        telemetry_metrics::record_latest_frame_size(kind.label(), data.len() as u64);
        *slot = Some((counter, data.clone()));
        data
    }

    /// The lock that serialises on-demand encodes of a family.
    pub fn on_demand_encode_lock(&self, kind: StreamKind) -> &Mutex<()> {
        &self.on_demand_encodes[kind as usize]
    }

    /// Drop every payload and forget the frame. Called when a stream's producer stops,
    /// so a reconnecting client is not served a frame from a camera that has gone.
    pub async fn clear(&self) {
        *self.latest_raw_frame.write().await = None;
        for slot in &self.payloads {
            *slot.write().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The regression this channel exists for.
    ///
    /// `Notify::notify_waiters` wakes only the tasks registered at that instant and
    /// leaves no permit behind, so every frame published while a handler sat in
    /// `socket.send().await` was dropped — the client then showed a frame one exposure
    /// out of date, or, if that was the session's last frame, never caught up at all.
    /// Here the receiver is deliberately not polled across the publish.
    #[tokio::test]
    async fn a_frame_published_while_nobody_polls_is_still_delivered() {
        let stream = FrameStream::default();
        let mut frames = stream.subscribe_frames();

        let counter = stream.begin_frame();
        stream.publish_frame();

        tokio::time::timeout(Duration::from_millis(100), frames.changed())
            .await
            .expect("the wakeup was lost")
            .expect("sender dropped");
        assert_eq!(*frames.borrow_and_update(), counter);
    }

    /// Several frames landing between polls collapse into one wakeup carrying the
    /// newest counter — the client skips to the current frame instead of replaying a
    /// backlog it has no use for.
    #[tokio::test]
    async fn a_burst_between_polls_collapses_to_the_newest_frame() {
        let stream = FrameStream::default();
        let mut frames = stream.subscribe_frames();

        for _ in 0..5 {
            stream.begin_frame();
            stream.publish_frame();
        }

        tokio::time::timeout(Duration::from_millis(100), frames.changed())
            .await
            .expect("the wakeup was lost")
            .expect("sender dropped");
        assert_eq!(*frames.borrow_and_update(), 5);

        // And nothing is left queued behind it.
        assert!(
            tokio::time::timeout(Duration::from_millis(20), frames.changed())
                .await
                .is_err()
        );
    }

    /// A subscriber starts level with the stream, so it does not immediately wake on a
    /// frame that was already published before it connected.
    #[tokio::test]
    async fn a_new_subscriber_does_not_wake_on_the_backlog() {
        let stream = FrameStream::default();
        stream.begin_frame();
        stream.publish_frame();

        let mut frames = stream.subscribe_frames();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), frames.changed())
                .await
                .is_err(),
            "a connecting client woke on a frame it had already been handed"
        );
    }

    /// Publishing with nobody subscribed must not fail — `watch::Sender::send` errors
    /// when there are no receivers, which would have made an unwatched stream stop
    /// advancing its published version.
    #[tokio::test]
    async fn publishing_with_no_subscribers_still_advances_the_version() {
        let stream = FrameStream::default();
        stream.begin_frame();
        stream.publish_frame();

        let mut frames = stream.subscribe_frames();
        assert_eq!(*frames.borrow_and_update(), 1);
    }

    /// A payload is served only for the exact frame it was encoded from: an older tag is
    /// a previous frame (or session), a newer one does not exist yet for this counter.
    #[test]
    fn a_payload_is_served_only_for_its_own_frame() {
        let stream = FrameStream::default();
        let counter = stream.begin_frame();
        stream.set_payload(StreamKind::Jpeg, counter, vec![1]);

        assert_eq!(stream.payload(StreamKind::Jpeg, counter).unwrap().as_ref(), &[1]);
        assert!(stream.payload(StreamKind::Jpeg, counter - 1).is_none());
        assert!(stream.payload(StreamKind::Jpeg, counter + 1).is_none());
    }

    /// A slow on-demand encode finishing after the producer stored a newer frame must not
    /// roll the slot back — it is still handed back so its caller can send it.
    #[test]
    fn an_older_payload_never_replaces_a_newer_one() {
        let stream = FrameStream::default();
        stream.set_payload(StreamKind::Lossless, 5, vec![5]);

        let late = stream.set_payload(StreamKind::Lossless, 4, vec![4]);
        assert_eq!(late.as_ref(), &[4]);
        assert_eq!(stream.payload(StreamKind::Lossless, 5).unwrap().as_ref(), &[5]);
        assert!(stream.payload(StreamKind::Lossless, 4).is_none());
    }

    /// Two encodes of one frame (the producer and an on-demand encode) may both store;
    /// the later one wins and both are the same frame.
    #[test]
    fn a_payload_for_the_same_frame_replaces_the_previous_one() {
        let stream = FrameStream::default();
        stream.set_payload(StreamKind::Jpeg, 3, vec![1]);
        stream.set_payload(StreamKind::Jpeg, 3, vec![2]);
        assert_eq!(stream.payload(StreamKind::Jpeg, 3).unwrap().as_ref(), &[2]);
    }

    /// Each family has its own slot: a lossless payload must never be served to a JPEG
    /// client, nor evict the JPEG payload of the same frame.
    #[test]
    fn families_store_payloads_independently() {
        let stream = FrameStream::default();
        let counter = stream.begin_frame();
        stream.set_payload(StreamKind::Lossless, counter, vec![1]);
        stream.set_payload(StreamKind::Jpeg, counter, vec![2]);

        assert_eq!(stream.payload(StreamKind::Lossless, counter).unwrap().as_ref(), &[1]);
        assert_eq!(stream.payload(StreamKind::Jpeg, counter).unwrap().as_ref(), &[2]);
    }

    /// A stopped producer must leave nothing for a reconnecting client, in either family.
    #[tokio::test]
    async fn clear_drops_the_frame_and_every_payload() {
        let stream = FrameStream::default();
        let counter = stream.begin_frame();
        for kind in StreamKind::all() {
            stream.set_payload(kind, counter, vec![1]);
        }

        stream.clear().await;

        assert!(stream.get_latest_raw_frame().await.is_none());
        for kind in StreamKind::all() {
            assert!(stream.payload(kind, counter).is_none());
        }
    }

    /// Storing payloads must not advance the counter — only `begin_frame` does, so
    /// clients cannot wake on a frame whose payloads are still being written.
    #[test]
    fn storing_a_payload_does_not_advance_the_counter() {
        let stream = FrameStream::default();
        stream.set_payload(StreamKind::Jpeg, 1, vec![1]);
        assert_eq!(stream.frame_counter(), 0);
        assert_eq!(stream.begin_frame(), 1);
    }
}
