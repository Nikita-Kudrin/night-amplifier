//! One independent image stream: the payloads, the counter that versions them, and the
//! client census that decides which of them are worth producing.
//!
//! There is one per camera role. Two producers sharing a counter is not a small
//! inefficiency but a correctness bug: [`JpegTierCache`] serves a tier only while its
//! `frame_counter` matches, so guide frames advancing the main counter would invalidate
//! every main-stream payload (and vice versa) and wake both sets of clients on every
//! frame from either camera.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::sync::{watch, RwLock};

use super::{JpegTier, JpegTierCache, RenderReadyFrame, StreamKind};
use crate::telemetry::metrics as telemetry_metrics;

/// Everything singular about one stream of rendered frames.
pub struct FrameStream {
    /// Latest rendered frame, LZ4-compressed for the lossless stream, tagged with the
    /// counter it was encoded from. The tag is what lets a newly-connected client tell
    /// this payload apart from one left over by a previous session — see
    /// [`Self::get_latest_frame`].
    latest_frame: RwLock<Option<(u64, bytes::Bytes)>>,
    /// Latest frame in linear form, for encoding a tier on demand.
    latest_raw_frame: RwLock<Option<Arc<RenderReadyFrame>>>,
    /// Versions every payload above. Claimed by [`Self::begin_frame`] before the
    /// payloads are stored and published by [`Self::publish_frame`] once they are, so a
    /// woken client never observes a counter whose payloads are still missing.
    frame_counter: AtomicU64,
    /// Carries the counter of the most recently published frame.
    ///
    /// A `watch` channel rather than a `Notify`: `Notify::notify_waiters` wakes only
    /// the tasks registered at that instant and stores no permit, so every frame
    /// published while a handler sat in `socket.send().await` was lost. A watch
    /// receiver latches the version instead, so a send that lands between two polls
    /// makes the next `changed()` return immediately.
    frame_ready: watch::Sender<u64>,
    /// Number of clients per resolution tier. The producer only encodes tiers somebody
    /// is watching — and, for the guide stream, only renders at all when somebody is.
    jpeg_tier_clients: [AtomicUsize; JpegTier::COUNT],
    /// Per-tier client counts for the lossless stream.
    lz4_tier_clients: [AtomicUsize; JpegTier::COUNT],
    /// Payloads pre-encoded by the producer, one slot per tier.
    jpeg_tier_cache: StdRwLock<JpegTierCache>,
}

impl Default for FrameStream {
    fn default() -> Self {
        Self {
            latest_frame: RwLock::new(None),
            latest_raw_frame: RwLock::new(None),
            frame_counter: AtomicU64::new(0),
            frame_ready: watch::channel(0).0,
            jpeg_tier_clients: std::array::from_fn(|_| AtomicUsize::new(0)),
            lz4_tier_clients: std::array::from_fn(|_| AtomicUsize::new(0)),
            jpeg_tier_cache: StdRwLock::new(JpegTierCache::default()),
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

    /// Store the LZ4-encoded payload for the lossless stream, tagged with the counter
    /// [`Self::begin_frame`] claimed for it.
    ///
    /// Storing does not advance the frame counter — see [`Self::begin_frame`].
    pub async fn set_latest_frame(&self, counter: u64, frame_data: Vec<u8>) {
        let frame_size = frame_data.len() as u64;
        *self.latest_frame.write().await = Some((counter, bytes::Bytes::from(frame_data)));
        telemetry_metrics::record_latest_frame_size(frame_size);
    }

    /// The stored lossless payload and the frame it was encoded from.
    ///
    /// The producer writes this only while a lossless client is registered, so it is
    /// routinely older than [`Self::frame_counter`] — callers must compare the two
    /// rather than assume the payload is current.
    pub async fn get_latest_frame(&self) -> Option<(u64, bytes::Bytes)> {
        self.latest_frame.read().await.clone()
    }

    /// Set the latest linear frame for dynamic encoding.
    pub async fn set_latest_raw_frame(&self, frame: Arc<RenderReadyFrame>) {
        *self.latest_raw_frame.write().await = Some(frame);
    }

    /// Retrieve the most recently rendered linear frame, if any.
    ///
    /// This is not the stacked FITS (which is linear 32-bit), but the snapshot of the
    /// live preview pipeline immediately before compression. Used by newly-connected
    /// WebSocket clients to encode an initial payload for their specific tier without
    /// waiting for the next camera exposure.
    pub async fn get_latest_raw_frame(&self) -> Option<Arc<RenderReadyFrame>> {
        self.latest_raw_frame.read().await.clone()
    }

    /// Per-tier client counters for one stream family.
    pub fn tier_clients(&self, kind: StreamKind) -> &[AtomicUsize; JpegTier::COUNT] {
        match kind {
            StreamKind::Jpeg => &self.jpeg_tier_clients,
            StreamKind::Lossless => &self.lz4_tier_clients,
        }
    }

    pub fn tier_client_count(&self, kind: StreamKind, tier: JpegTier) -> usize {
        self.tier_clients(kind)[tier as usize].load(Ordering::SeqCst)
    }

    /// Number of clients currently watching a JPEG resolution tier.
    pub fn jpeg_tier_client_count(&self, tier: JpegTier) -> usize {
        self.tier_client_count(StreamKind::Jpeg, tier)
    }

    /// How many clients the lossless stream currently has, across every tier.
    ///
    /// The producer encodes the LZ4 payload only when this is non-zero. Every lossless
    /// connection holds a `TierClientGuard` for its whole life — a client that has not
    /// reported a viewport holds one at [`JpegTier::LOSSLESS_DEFAULT`] — so this is a
    /// complete count and needs no counter of its own.
    pub fn lossless_client_count(&self) -> usize {
        self.lz4_tier_clients
            .iter()
            .map(|c| c.load(Ordering::SeqCst))
            .sum()
    }

    /// Whether anyone at all is watching this stream, on either family.
    ///
    /// The guide loop's render gate: with nobody watching there is no reason to pay for
    /// background extraction, the stretch solve and an encode, so it does none of them.
    /// The solver is fed either way — see `capture::guide_task`.
    pub fn has_viewers(&self) -> bool {
        JpegTier::all()
            .into_iter()
            .any(|tier| self.jpeg_tier_client_count(tier) > 0)
            || self.lossless_client_count() > 0
    }

    /// Bounding box the lossless stream should encode into.
    ///
    /// The stream keeps a single payload rather than one per tier, so it is served at
    /// the largest tier any connected client asked for: a client on a smaller tier then
    /// receives more pixels than it needs, which is what every client got before tiers
    /// reached this path.
    ///
    /// Falls back to the 4K cap when no client has reported a viewport, so a client that
    /// never sends one is served exactly as it was before.
    pub fn lossless_target_box(&self) -> (u32, u32) {
        let (cap_w, cap_h) = crate::server::encoding::JPEG_MAX_BOUNDING_BOX;
        let largest = JpegTier::all()
            .into_iter()
            .rfind(|&tier| self.tier_client_count(StreamKind::Lossless, tier) > 0);
        match largest {
            Some(tier) => tier.lossless_box(),
            None => (cap_w, cap_h),
        }
    }

    /// Look up the pre-encoded JPEG for a tier at the given frame.
    pub fn get_tier_jpeg(&self, tier: JpegTier, counter: u64) -> Option<bytes::Bytes> {
        self.jpeg_tier_cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(tier, counter)
    }

    /// Publish a pre-encoded JPEG for a tier and return a shareable handle, which the
    /// producer reuses for tiers that resolve to the same output size.
    pub fn set_tier_jpeg(
        &self,
        tier: JpegTier,
        counter: u64,
        data: impl Into<bytes::Bytes>,
    ) -> bytes::Bytes {
        self.jpeg_tier_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tier, counter, data)
    }

    /// Drop every payload and forget the frame. Called when a stream's producer stops,
    /// so a reconnecting client is not served a frame from a camera that has gone.
    pub async fn clear(&self) {
        *self.latest_frame.write().await = None;
        *self.latest_raw_frame.write().await = None;
        self.jpeg_tier_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
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

    /// The payload tag is what tells a connecting client whether the stored frame
    /// belongs to the counter it just read, rather than to a previous session.
    #[tokio::test]
    async fn the_stored_payload_carries_the_counter_it_was_encoded_from() {
        let stream = FrameStream::default();
        let counter = stream.begin_frame();
        stream.set_latest_frame(counter, vec![1, 2, 3]).await;

        // A later frame the producer skipped, because nobody was watching losslessly.
        stream.begin_frame();

        let (tag, _) = stream.get_latest_frame().await.unwrap();
        assert_eq!(tag, counter);
        assert!(tag < stream.frame_counter());
    }
}
