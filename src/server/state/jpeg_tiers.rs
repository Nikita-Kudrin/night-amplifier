//! Demand-driven resolution tiers for the live image streams.
//!
//! Every streaming client is mapped onto one of a small, fixed set of bounding
//! boxes. The render task encodes a payload once per tier that has at least one
//! client and caches it, so WebSocket handlers never encode on their own — they
//! copy a pointer and write it to the socket.
//!
//! Both stream families use the same tiers, for the same reason: resampling to
//! the size a client will actually display is where most of a noisy frame's
//! grain goes away, and letting the browser minify instead throws that
//! averaging out. The GPU minifies with a four-tap bilinear filter and no
//! mipmaps, which is capped around 1.45x of noise reduction however far it is
//! shrinking, where an area average delivers the full factor.
//!
//! `JpegTier` keeps its name because that is where it started; `StreamKind`
//! selects which stream's client counters a tier registration lands in.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::AppState;
use crate::server::encoding::{JPEG_MAX_BOUNDING_BOX, JPEG_MIN_BOUNDING_BOX};

/// Smallest and largest display-resolution class a client can ask for, i.e. the
/// short edges of the streamable bounding boxes (1080 … 2160).
const MIN_RESOLUTION_CLASS: u32 = JPEG_MIN_BOUNDING_BOX.1;
const MAX_RESOLUTION_CLASS: u32 = JPEG_MAX_BOUNDING_BOX.1;

/// Which stream family a tier registration belongs to.
///
/// The two differ only in how a payload is stored: JPEG keeps one per tier in
/// `JpegTierCache`, while the lossless stream keeps a single payload and so is
/// served at the largest tier any of its clients asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    /// Dynamic JPEG (SA10) — `/ws/stream` and `/ws/eyepiece`.
    Jpeg,
    /// Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`.
    Lossless,
}

impl StreamKind {
    pub const COUNT: usize = 2;

    pub const fn all() -> [StreamKind; Self::COUNT] {
        [Self::Jpeg, Self::Lossless]
    }
}

/// Fixed resolution tiers, ordered by ascending bounding-box size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JpegTier {
    /// 1920x1080 bounding box — phones and the eyepiece overlay.
    Hd1080,
    /// 2560x1440 bounding box.
    Qhd1440,
    /// 3840x2160 bounding box — 4K tablets.
    Uhd2160,
    /// Native sensor resolution, no downsampling.
    Original,
}

impl JpegTier {
    /// Number of tiers; also the length of the per-tier client counters and
    /// cache slots.
    pub const COUNT: usize = 4;

    /// All tiers in ascending bounding-box order.
    pub const fn all() -> [JpegTier; Self::COUNT] {
        [Self::Hd1080, Self::Qhd1440, Self::Uhd2160, Self::Original]
    }

    /// Tier a lossless client holds until it reports a viewport.
    ///
    /// The 4K cap, not the 1080 floor: this stream shipped a hardcoded
    /// 3840x2160 box before tiers reached it, and a client that never reports —
    /// an older frontend, or one whose report is still in flight — must not be
    /// silently *downgraded* by a change that exists to improve it. The JPEG
    /// path defaults to the floor instead, because there a wrong guess costs an
    /// entire wasted encode rather than a slightly larger payload.
    pub const LOSSLESS_DEFAULT: Self = Self::Uhd2160;

    /// Maximum output dimensions for the tier. `Original` is unbounded so the
    /// frame is encoded at its native size.
    pub const fn bounding_box(self) -> (u32, u32) {
        match self {
            Self::Hd1080 => (1920, 1080),
            Self::Qhd1440 => (2560, 1440),
            Self::Uhd2160 => (3840, 2160),
            Self::Original => (u32::MAX, u32::MAX),
        }
    }

    /// Smallest tier that can serve a viewport, selected by its shorter edge.
    ///
    /// Frames are always fitted to the viewport with their aspect ratio intact,
    /// so the pixels a client can actually use are bounded by its *shorter* edge
    /// — a landscape frame in a portrait phone is limited by the phone's width,
    /// and in a landscape phone by its height. Comparing both edges against a
    /// 16:9 box instead would force a portrait 1080x2220 phone into the 4K tier,
    /// making it download a full-resolution frame to display 1080 px of it.
    pub fn from_client_resolution(width: u32, height: u32) -> Self {
        Self::from_resolution_class(width.min(height))
    }

    /// Tier for a client's requested viewport, clamped to the streamable range.
    pub fn for_request(req_w: Option<u32>, req_h: Option<u32>) -> Self {
        let requested = match (req_w, req_h) {
            (Some(w), Some(h)) => w.min(h),
            (Some(edge), None) | (None, Some(edge)) => edge,
            (None, None) => MIN_RESOLUTION_CLASS,
        };
        Self::from_resolution_class(requested.clamp(MIN_RESOLUTION_CLASS, MAX_RESOLUTION_CLASS))
    }

    /// Smallest tier whose short edge covers the requested resolution class.
    fn from_resolution_class(class: u32) -> Self {
        Self::all()
            .into_iter()
            .find(|tier| class <= tier.bounding_box().1)
            .unwrap_or(Self::Original)
    }

    /// Stable label for telemetry attributes.
    pub const fn metric_label(self) -> &'static str {
        match self {
            Self::Hd1080 => "hd1080",
            Self::Qhd1440 => "qhd1440",
            Self::Uhd2160 => "uhd2160",
            Self::Original => "original",
        }
    }

    /// True when encoding at this tier shrinks the frame. Tiers that do not
    /// downsample all produce the same native-resolution payload, which lets
    /// the render task encode once and share the result.
    pub fn would_downsample(self, frame_w: usize, frame_h: usize) -> bool {
        let (max_w, max_h) = self.bounding_box();
        frame_w > max_w as usize || frame_h > max_h as usize
    }
}

/// Pre-encoded JPEG payloads for the frame currently being streamed, one slot
/// per tier.
///
/// Slots are dropped as soon as the frame counter advances: a client that wakes
/// late has no use for a superseded frame.
#[derive(Default)]
pub struct JpegTierCache {
    frame_counter: u64,
    entries: [Option<bytes::Bytes>; JpegTier::COUNT],
}

impl JpegTierCache {
    /// Look up the payload for a tier at the given frame counter. Returns
    /// `None` once the frame has advanced or if the tier was not encoded.
    pub fn get(&self, tier: JpegTier, counter: u64) -> Option<bytes::Bytes> {
        if counter != self.frame_counter {
            return None;
        }
        self.entries[tier as usize].clone()
    }

    /// Store a payload for a tier and hand back a shareable handle to it.
    ///
    /// A counter newer than the cached one clears the stale slots. A payload
    /// encoded for an already-superseded frame is returned untouched so its
    /// encoder can still send it, without rolling the cache backwards.
    pub fn insert(
        &mut self,
        tier: JpegTier,
        counter: u64,
        data: impl Into<bytes::Bytes>,
    ) -> bytes::Bytes {
        let data = data.into();
        if counter < self.frame_counter {
            return data;
        }
        if counter > self.frame_counter {
            self.entries = Default::default();
            self.frame_counter = counter;
        }
        self.entries[tier as usize] = Some(data.clone());
        data
    }
}

/// Registers a client against a resolution tier for as long as it is alive.
///
/// The render task only encodes tiers with a non-zero count, so the decrement
/// has to happen even when a handler exits early or panics — hence a `Drop`
/// guard rather than manual bookkeeping.
pub struct TierClientGuard {
    state: Arc<AppState>,
    kind: StreamKind,
    tier: JpegTier,
}

impl TierClientGuard {
    pub fn new(state: Arc<AppState>, kind: StreamKind, tier: JpegTier) -> Self {
        state.tier_clients(kind)[tier as usize].fetch_add(1, Ordering::SeqCst);
        Self { state, kind, tier }
    }

    pub fn tier(&self) -> JpegTier {
        self.tier
    }

    /// Move this client to another tier. The new tier is claimed before the old
    /// one is released so a tier the client still needs is never seen idle.
    pub fn set_tier(&mut self, tier: JpegTier) {
        if tier == self.tier {
            return;
        }
        let counters = self.state.tier_clients(self.kind);
        counters[tier as usize].fetch_add(1, Ordering::SeqCst);
        counters[self.tier as usize].fetch_sub(1, Ordering::SeqCst);
        self.tier = tier;
    }
}

impl Drop for TierClientGuard {
    fn drop(&mut self) {
        self.state.tier_clients(self.kind)[self.tier as usize].fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMX464: (usize, usize) = (2712, 1538);

    #[test]
    fn test_tier_bounding_box_values() {
        assert_eq!(JpegTier::Hd1080.bounding_box(), (1920, 1080));
        assert_eq!(JpegTier::Qhd1440.bounding_box(), (2560, 1440));
        assert_eq!(JpegTier::Uhd2160.bounding_box(), (3840, 2160));
        assert_eq!(JpegTier::Original.bounding_box(), (u32::MAX, u32::MAX));
    }

    #[test]
    fn test_tier_from_resolution_1080p() {
        assert_eq!(
            JpegTier::from_client_resolution(1920, 1080),
            JpegTier::Hd1080
        );
    }

    #[test]
    fn test_tier_from_resolution_below_1080p() {
        // A 720p phone viewport clamps up to the 1080p floor.
        assert_eq!(
            JpegTier::for_request(Some(1280), Some(720)),
            JpegTier::Hd1080
        );
    }

    #[test]
    fn test_tier_from_resolution_1440p() {
        assert_eq!(
            JpegTier::from_client_resolution(2560, 1440),
            JpegTier::Qhd1440
        );
    }

    #[test]
    fn test_tier_from_resolution_between_tiers() {
        assert_eq!(
            JpegTier::from_client_resolution(2200, 1200),
            JpegTier::Qhd1440
        );
    }

    #[test]
    fn test_tier_from_resolution_4k() {
        assert_eq!(
            JpegTier::from_client_resolution(3840, 2160),
            JpegTier::Uhd2160
        );
    }

    #[test]
    fn test_tier_from_resolution_over_4k() {
        // Requests above 4K clamp down, so no client selects `Original`.
        assert_eq!(
            JpegTier::for_request(Some(5000), Some(3000)),
            JpegTier::Uhd2160
        );
    }

    #[test]
    fn test_tier_from_resolution_defaults_to_1080p() {
        assert_eq!(JpegTier::for_request(None, None), JpegTier::Hd1080);
    }

    /// A Samsung S22 (1080x2340, DPR 3) reports roughly 1080x2220 upright and
    /// 2340x1080 sideways. Both need ~1080 usable pixels, so both must land on
    /// the cheapest tier — rotating the phone must not change its bandwidth.
    #[test]
    fn test_tier_for_1080p_phone_is_orientation_independent() {
        assert_eq!(
            JpegTier::for_request(Some(1080), Some(2220)),
            JpegTier::Hd1080
        );
        assert_eq!(
            JpegTier::for_request(Some(2340), Some(1080)),
            JpegTier::Hd1080
        );
    }

    #[test]
    fn test_tier_for_4k_tablet_is_orientation_independent() {
        assert_eq!(
            JpegTier::for_request(Some(3840), Some(2160)),
            JpegTier::Uhd2160
        );
        assert_eq!(
            JpegTier::for_request(Some(2160), Some(3840)),
            JpegTier::Uhd2160
        );
    }

    #[test]
    fn test_tier_for_request_with_single_edge() {
        assert_eq!(JpegTier::for_request(Some(1440), None), JpegTier::Qhd1440);
        assert_eq!(JpegTier::for_request(None, Some(2160)), JpegTier::Uhd2160);
    }

    #[test]
    fn test_tier_would_downsample() {
        let (w, h) = IMX464;
        assert!(JpegTier::Hd1080.would_downsample(w, h));
        assert!(!JpegTier::Uhd2160.would_downsample(w, h));
        assert!(!JpegTier::Original.would_downsample(w, h));
    }

    #[test]
    fn test_jpeg_tier_cache_stores_and_retrieves() {
        let mut cache = JpegTierCache::default();
        let stored = cache.insert(JpegTier::Hd1080, 7, vec![1, 2, 3]);

        assert_eq!(stored.as_ref(), &[1, 2, 3]);
        assert_eq!(cache.get(JpegTier::Hd1080, 7).unwrap().as_ref(), &[1, 2, 3]);
    }

    #[test]
    fn test_jpeg_tier_cache_clears_on_new_frame() {
        let mut cache = JpegTierCache::default();
        cache.insert(JpegTier::Hd1080, 7, vec![1, 2, 3]);
        cache.insert(JpegTier::Qhd1440, 8, vec![4, 5, 6]);

        assert!(cache.get(JpegTier::Hd1080, 7).is_none());
        assert!(cache.get(JpegTier::Hd1080, 8).is_none());
        assert_eq!(
            cache.get(JpegTier::Qhd1440, 8).unwrap().as_ref(),
            &[4, 5, 6]
        );
    }

    #[test]
    fn test_jpeg_tier_cache_miss_on_wrong_tier() {
        let mut cache = JpegTierCache::default();
        cache.insert(JpegTier::Hd1080, 1, vec![9]);

        assert!(cache.get(JpegTier::Qhd1440, 1).is_none());
    }

    #[test]
    fn test_jpeg_tier_cache_ignores_stale_counter() {
        let mut cache = JpegTierCache::default();
        cache.insert(JpegTier::Hd1080, 5, vec![1]);

        // A late encode for frame 4 must not evict frame 5, but is still
        // returned so its encoder can send it.
        let late = cache.insert(JpegTier::Qhd1440, 4, vec![2]);
        assert_eq!(late.as_ref(), &[2]);
        assert!(cache.get(JpegTier::Qhd1440, 4).is_none());
        assert_eq!(cache.get(JpegTier::Hd1080, 5).unwrap().as_ref(), &[1]);
    }

    #[test]
    fn test_jpeg_tier_cache_shares_payload_between_tiers() {
        let mut cache = JpegTierCache::default();
        let native = cache.insert(JpegTier::Uhd2160, 3, vec![1, 2, 3]);
        cache.insert(JpegTier::Original, 3, native.clone());

        let uhd = cache.get(JpegTier::Uhd2160, 3).unwrap();
        let original = cache.get(JpegTier::Original, 3).unwrap();
        assert_eq!(uhd.as_ptr(), original.as_ptr());
    }

    #[tokio::test]
    async fn test_jpeg_tier_client_increment_decrement() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let guard = TierClientGuard::new(Arc::clone(&state), StreamKind::Jpeg, JpegTier::Qhd1440);
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Qhd1440), 1);
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Hd1080), 0);

        drop(guard);
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Qhd1440), 0);
    }

    #[tokio::test]
    async fn test_jpeg_tier_client_guard_drop() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let handle = {
            let state = Arc::clone(&state);
            std::thread::spawn(move || {
                let _guard = TierClientGuard::new(state, StreamKind::Jpeg, JpegTier::Hd1080);
                panic!("intentional: checking the guard releases the tier while unwinding");
            })
        };
        assert!(handle.join().is_err());

        // Unwinding through the guard still releases the tier.
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Hd1080), 0);
    }

    /// With nobody connected the lossless stream keeps its historical 4K cap,
    /// so a client that never reports a viewport is served exactly as before.
    #[tokio::test]
    async fn lossless_target_falls_back_to_the_4k_cap() {
        let (state, _disk_writer) = AppState::new_for_testing();
        assert_eq!(state.lossless_target_box(), JPEG_MAX_BOUNDING_BOX);
    }

    #[tokio::test]
    async fn lossless_target_follows_the_connected_client() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let guard =
            TierClientGuard::new(Arc::clone(&state), StreamKind::Lossless, JpegTier::Qhd1440);
        assert_eq!(state.lossless_target_box(), (2560, 1440));

        drop(guard);
        assert_eq!(
            state.lossless_target_box(),
            JPEG_MAX_BOUNDING_BOX,
            "a disconnect must release the tier, not strand the stream at it"
        );
    }

    /// The lossless stream keeps one payload, not one per tier, so it has to be
    /// encoded for the largest viewport anyone asked for — a client on a smaller
    /// tier then receives more pixels than it needs, which is what every client
    /// got before tiers reached this path.
    #[tokio::test]
    async fn lossless_target_serves_the_largest_connected_tier() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let _small =
            TierClientGuard::new(Arc::clone(&state), StreamKind::Lossless, JpegTier::Hd1080);
        let large =
            TierClientGuard::new(Arc::clone(&state), StreamKind::Lossless, JpegTier::Uhd2160);
        assert_eq!(state.lossless_target_box(), (3840, 2160));

        // Dropping the larger client must fall back to the one still watching,
        // not to the cap.
        drop(large);
        assert_eq!(state.lossless_target_box(), (1920, 1080));
    }

    /// The two stream families keep separate counters: a JPEG client must not
    /// change what the lossless stream encodes, or one browser tab would resize
    /// another's.
    #[tokio::test]
    async fn stream_kinds_do_not_share_tier_counters() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let _jpeg = TierClientGuard::new(Arc::clone(&state), StreamKind::Jpeg, JpegTier::Uhd2160);
        assert_eq!(
            state.lossless_target_box(),
            JPEG_MAX_BOUNDING_BOX,
            "a JPEG client must not register against the lossless stream"
        );
        assert_eq!(
            state.tier_client_count(StreamKind::Lossless, JpegTier::Uhd2160),
            0
        );

        let _lossless =
            TierClientGuard::new(Arc::clone(&state), StreamKind::Lossless, JpegTier::Hd1080);
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Hd1080), 0);
        assert_eq!(state.lossless_target_box(), (1920, 1080));
    }

    /// `Original` is unbounded; the lossless stream must still cap at 4K rather
    /// than trying to ship a native frame of any size.
    #[tokio::test]
    async fn lossless_target_caps_an_unbounded_tier() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let _guard =
            TierClientGuard::new(Arc::clone(&state), StreamKind::Lossless, JpegTier::Original);
        assert_eq!(state.lossless_target_box(), JPEG_MAX_BOUNDING_BOX);
    }

    #[tokio::test]
    async fn test_jpeg_tier_client_guard_set_tier_moves_count() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);

        let mut guard = TierClientGuard::new(Arc::clone(&state), StreamKind::Jpeg, JpegTier::Hd1080);
        guard.set_tier(JpegTier::Uhd2160);

        assert_eq!(guard.tier(), JpegTier::Uhd2160);
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Hd1080), 0);
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Uhd2160), 1);

        // Re-selecting the same tier must not double-count.
        guard.set_tier(JpegTier::Uhd2160);
        assert_eq!(state.jpeg_tier_client_count(JpegTier::Uhd2160), 1);
    }
}
