//! Correlation-aligned planetary live stacking.

use tracing::{debug, field, info, instrument, warn, Span};

use super::{LiveStackError, LiveStacker, StackSettings};
use night_amplifier_core::frame::{Frame, NoiseField};
use night_amplifier_core::planetary::{
    align, compute_centroid, Alignment, AlignmentReference, AlignmentRoi, Fit, LuminancePyramid, Search,
};
use night_amplifier_core::registration::AffineTransform;
use night_amplifier_core::plugins::Plugins;
use crate::capture::frame_gate::{FrameAdmission, RejectionReason};
use night_amplifier_core::stacking::{FrameQuality, Stacker, StackingType};

/// Holds state for planetary-based live stacking pipeline
pub struct PlanetaryStackingContext {
    pub stacker: Stacker,
    pub is_initialized: bool,
    /// The reference's luminance levels and centroid, built once per reference.
    reference_pyramid: Option<LuminancePyramid>,
    reference_centroid: (f32, f32),
    /// The reference's window and the one asked for: rebuilt only when that changes.
    alignment: Option<(AlignmentRoi, AlignmentReference)>,
    /// The last correlated shift: the target drifts (2026-10-24's Moon 2.5 px/s, 230 px in
    /// 80 s), so each search starts where the previous frame was found.
    last_shift: Option<(f32, f32)>,
    /// Multi-point alignment comes from here, checked frame by frame.
    plugins: Plugins,
}

impl PlanetaryStackingContext {
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        settings: &StackSettings,
    ) -> Result<Self, LiveStackError> {
        let config = settings.config.clone();
        let stacker =
            Stacker::with_plugins(width, height, channels, config, settings.plugins.clone())?;

        Ok(Self {
            stacker,
            is_initialized: false,
            reference_pyramid: None,
            reference_centroid: (0.0, 0.0),
            alignment: None,
            last_shift: None,
            plugins: settings.plugins.clone(),
        })
    }

    #[instrument(skip(self, frame))]
    pub fn initialize_with_reference(&mut self, frame: &Frame) -> Result<(), LiveStackError> {
        let pyramid = LuminancePyramid::of(frame);
        self.reference_centroid = compute_centroid(pyramid.luminance(), frame.width(), frame.height());
        self.reference_pyramid = Some(pyramid);
        self.alignment = None;
        self.last_shift = None;

        if let Some(plugin) = self.plugins.planetary() {
            plugin.clear_cache();
        }

        // Add reference frame with default quality
        let quality = FrameQuality::default();

        self.stacker.add_reference_with_quality(frame, quality)?;

        self.is_initialized = true;
        Ok(())
    }

    /// Whether the frame joined the stack, where it landed (target -> reference), and why
    /// not when it did not.
    #[instrument(skip(self, frame, settings), fields(
        dx = field::Empty,
        dy = field::Empty,
        ncc = field::Empty,
        registered = field::Empty,
    ))]
    pub fn add_frame(&mut self, frame: &Frame, settings: &StackSettings) -> Result<FrameAdmission, LiveStackError> {
        let tracking = settings.planetary_auto_tracking.then_some(self.reference_centroid);
        let roi = AlignmentRoi::for_alignment(frame.width(), frame.height(), settings.planetary_roi, tracking);
        let pyramid = LuminancePyramid::of(frame);
        let (reference_centroid, last_shift) = (self.reference_centroid, self.last_shift);
        let found = find_shift(self.reference_for(roi)?, &pyramid, reference_centroid, last_shift);
        debug!(dx = found.dx, dy = found.dy, ncc = found.ncc, fit = ?found.fit, "Planetary alignment results");
        Span::current().record("dx", found.dx);
        Span::current().record("dy", found.dy);
        Span::current().record("ncc", found.ncc);
        if let Some(reason) = refusal(found.fit) {
            debug!(fit = ?found.fit, "Planetary frame not found near the reference, not added to stack");
            Span::current().record("registered", false);
            return Ok(FrameAdmission::refused(reason));
        }
        if found.fit == Fit::Correlated {
            self.last_shift = Some((found.dx, found.dy));
        }

        let warped_frame = self.multi_point_warp(frame, &pyramid, &roi, &found, settings);
        let landed = AffineTransform::from_translation(found.dx, found.dy);
        let transform = match warped_frame {
            Some(_) => AffineTransform::from_translation(0.0, 0.0),
            None => landed,
        };
        let final_frame = warped_frame.as_ref().unwrap_or(frame);

        // Compute quality (standard FWHM/SNR or planetary-specific)
        let quality = FrameQuality::default();

        match self
            .stacker
            .add_frame_with_quality(final_frame, &transform, quality)
        {
            Ok(()) => {
                Span::current().record("registered", true);
                Ok(FrameAdmission::correlated(landed))
            }
            Err(e) => {
                debug!(error = %e, "Failed to add frame to planetary stack");
                Span::current().record("registered", false);
                Ok(FrameAdmission::refused(RejectionReason::StackerError))
            }
        }
    }

    /// The reference's window for `roi`, built on the first frame that asks for it.
    fn reference_for(&mut self, roi: AlignmentRoi) -> Result<&AlignmentReference, LiveStackError> {
        let pyramid = self.reference_pyramid.as_ref().ok_or(LiveStackError::NoReference)?;
        if self.alignment.as_ref().is_some_and(|(asked, _)| *asked != roi) {
            self.alignment = None;
        }
        let (_, built) = self.alignment.get_or_insert_with(|| (roi, AlignmentReference::new(pyramid, &roi)));
        Ok(built)
    }

    /// The frame warped by Pro's multi-point alignment, each point searching around the
    /// global shift; `None` without the plugin, with the setting off, or on its failure.
    fn multi_point_warp(
        &self,
        frame: &Frame,
        pyramid: &LuminancePyramid,
        roi: &AlignmentRoi,
        global: &Alignment,
        settings: &StackSettings,
    ) -> Option<Frame> {
        if !settings.planetary_multi_point_alignment {
            return None;
        }
        let (plugin, reference) = (settings.plugins.planetary()?, self.reference_pyramid.as_ref()?);
        let search = Search { expected: (global.dx, global.dy), radius: LOCAL_SEARCH_RADIUS };
        plugin
            .warp_frame(frame, pyramid, reference, roi, search)
            .inspect_err(|e| debug!(error = %e, "Multi-point planetary alignment failed, falling back"))
            .ok()
    }

    #[instrument(skip(self), fields(frame_count = self.frame_count()))]
    pub fn compute(&self) -> Result<Frame, LiveStackError> {
        Ok(self.stacker.compute()?)
    }

    pub fn frame_count(&self) -> usize {
        self.stacker.frame_count()
    }

    pub fn width(&self) -> usize {
        self.stacker.width()
    }

    pub fn height(&self) -> usize {
        self.stacker.height()
    }

    pub fn channels(&self) -> usize {
        self.stacker.channels()
    }

    /// Update stacking parameters from current settings dynamically
    pub fn update_from_settings(&mut self, settings: &StackSettings) {
        self.stacker.update_config(settings.config.clone());
    }
}

/// How far a frame may have moved from where its search starts.
const SEARCH_RADIUS: usize = 50;
/// The retry around the centroids' estimate, whose error grows with the drift.
const WIDE_SEARCH_RADIUS: usize = 200;
/// How far seeing may move one alignment point's patch from the frame's global shift.
const LOCAL_SEARCH_RADIUS: usize = 8;
const SUBPIXEL_FACTOR: usize = 2;

/// Searches around the last correlated shift, then widely around the frames' centroid
/// shift, which a bump the last shift cannot follow still lands near: on the Moon, which
/// fills its frame, it read 0.82-0.91 of the true drift past 40 px (208 of 255 by the end).
fn find_shift(
    reference: &AlignmentReference,
    frame: &LuminancePyramid,
    reference_centroid: (f32, f32),
    last_shift: Option<(f32, f32)>,
) -> Alignment {
    let (cx, cy) = compute_centroid(frame.luminance(), frame.width(), frame.height());
    let from_centroids = (reference_centroid.0 - cx, reference_centroid.1 - cy);
    let around = |expected, radius| align(reference, frame, Search { expected, radius }, SUBPIXEL_FACTOR);
    let found = around(last_shift.unwrap_or(from_centroids), SEARCH_RADIUS);
    if found.fit == Fit::Correlated {
        return found;
    }
    around(from_centroids, WIDE_SEARCH_RADIUS)
}

/// Why a frame found at `fit` is not stacked: a wrong shift is worse than a frame left out.
fn refusal(fit: Fit) -> Option<RejectionReason> {
    match fit {
        Fit::Correlated | Fit::Featureless => None,
        Fit::SearchEdge => Some(RejectionReason::TargetOutOfReach),
        Fit::Uncorrelated => Some(RejectionReason::SurfaceNotFound),
    }
}

impl LiveStacker for PlanetaryStackingContext {
    fn kind(&self) -> StackingType {
        StackingType::Planetary
    }

    fn geometry(&self) -> (usize, usize, usize) {
        (self.width(), self.height(), self.channels())
    }

    fn depth(&self) -> usize {
        self.frame_count()
    }

    fn has_reference(&self) -> bool {
        self.is_initialized
    }

    fn apply_settings(&mut self, settings: &StackSettings) {
        self.update_from_settings(settings);
    }

    fn set_reference(&mut self, frame: &Frame) -> Result<(), LiveStackError> {
        self.initialize_with_reference(frame)?;
        info!("Planetary stacking initialized with reference frame");
        Ok(())
    }

    fn offer(
        &mut self,
        frame: &Frame,
        settings: &StackSettings,
    ) -> Result<FrameAdmission, LiveStackError> {
        // An error leaves the accumulated stack on screen, as a failed alignment does.
        match self.add_frame(frame, settings) {
            Ok(admission) if admission.added => {
                info!(frame_count = self.frame_count(), "Frame added to planetary stack");
                Ok(admission)
            }
            Ok(admission) => {
                info!(
                    frame_count = self.frame_count(),
                    reason = admission.rejected_because.map(|r| r.describe()),
                    "Planetary alignment failed, frame not added to stack"
                );
                Ok(admission)
            }
            Err(e) => {
                warn!(error = %e, "Error adding frame to planetary stack");
                Ok(FrameAdmission::unreasoned(false))
            }
        }
    }

    fn snapshot(&self) -> Result<(Frame, Option<NoiseField>), LiveStackError> {
        self.compute().map(|frame| (frame, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use night_amplifier_core::frame::Frame;
    use crate::state::CaptureSettings;

    fn defaults() -> StackSettings {
        StackSettings::of(&CaptureSettings::default(), &Plugins::none())
    }

    /// A textured disc filling most of a 1000 px frame (a Moon-like target the centroid
    /// can roughly follow), moved by `shift`. One seed is one surface.
    fn moon(shift: (f32, f32)) -> Frame {
        let size = 1000usize;
        let mut plane = vec![0.0f32; size * size];
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 40) as f32 / (1u64 << 24) as f32
        };
        let craters: Vec<(f32, f32, f32, f32)> =
            (0..700).map(|_| (next() * 1000.0, next() * 1000.0, 3.0 + next() * 20.0, (next() - 0.5) * 0.3)).collect();
        let (cx, cy) = (500.0 + shift.0, 500.0 + shift.1);
        for y in 0..size {
            for x in 0..size {
                let (fx, fy) = (x as f32 - shift.0, y as f32 - shift.1);
                let dist = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if dist > 380.0 {
                    continue;
                }
                let texture: f32 = craters
                    .iter()
                    .filter(|c| (c.0 - fx).abs() < 4.0 * c.2 && (c.1 - fy).abs() < 4.0 * c.2)
                    .map(|c| c.3 * (-((c.0 - fx).powi(2) + (c.1 - fy).powi(2)) / (2.0 * c.2 * c.2)).exp())
                    .sum();
                plane[y * size + x] = 0.5 + texture;
            }
        }
        Frame::from_f32_vec(plane, size, size, 1).unwrap()
    }

    fn moon_context(settings: &StackSettings) -> PlanetaryStackingContext {
        let mut ctx = PlanetaryStackingContext::new(1000, 1000, 1, settings).unwrap();
        ctx.initialize_with_reference(&moon((0.0, 0.0))).unwrap();
        ctx
    }

    fn assert_shift(ctx: &PlanetaryStackingContext, truth: (f32, f32)) {
        let (dx, dy) = ctx.last_shift.expect("a correlated shift");
        assert!((dx + truth.0).abs() < 0.3 && (dy + truth.1).abs() < 0.3, "found ({dx}, {dy}) for {truth:?}");
    }

    /// 2026-10-24's Moon drifted 230 px in 80 s; searched around no shift, 77 % of its
    /// frames matched on the search's edge and were stacked at a wrong shift.
    #[test]
    fn a_drift_past_the_search_radius_is_followed() {
        let settings = defaults();
        let mut ctx = moon_context(&settings);
        for step in 1..=8 {
            let truth = (-3.0 * step as f32, 25.0 * step as f32 + 0.4);
            assert!(ctx.add_frame(&moon(truth), &settings).unwrap().added, "step {step} was refused");
            assert_shift(&ctx, truth);
        }
        assert_eq!(ctx.frame_count(), 9);
    }

    #[test]
    fn a_bump_the_last_shift_cannot_reach_is_found_from_the_centroids() {
        let settings = defaults();
        let mut ctx = moon_context(&settings);
        assert!(ctx.add_frame(&moon((1.0, 2.0)), &settings).unwrap().added);
        assert!(ctx.add_frame(&moon((-70.0, 135.0)), &settings).unwrap().added);
        assert_shift(&ctx, (-70.0, 135.0));
    }

    /// A field whose centroid stays put, moved past the wide search (230 px: its best match
    /// sits on the search's edge) and past where the frame holds the window (260 px:
    /// nothing correlates). A wrong shift is worse than a frame left out.
    #[test]
    fn a_frame_matching_only_on_the_search_edge_is_not_stacked() {
        let settings = defaults();
        let field = |shift: (f32, f32)| {
            let mut frame = moon(shift);
            frame.data_mut().iter_mut().filter(|v| **v == 0.0).for_each(|v| *v = 0.5);
            frame
        };
        let mut ctx = PlanetaryStackingContext::new(1000, 1000, 1, &settings).unwrap();
        ctx.initialize_with_reference(&field((0.0, 0.0))).unwrap();
        let mut refused = |shift| ctx.add_frame(&field(shift), &settings).unwrap().rejected_because;
        assert_eq!(refused((0.0, 230.0)), Some(RejectionReason::TargetOutOfReach));
        assert_eq!(refused((0.0, 260.0)), Some(RejectionReason::SurfaceNotFound));
        assert_eq!(ctx.frame_count(), 1);
        assert_eq!(ctx.last_shift, None);
    }

    /// A target off to one side (a planet left to drift across, the Moon's limb by the
    /// edge): auto-tracking centres the window on it, clamped against the frame's edge, so
    /// a frame that has not moved peaks where the window cannot step further out.
    #[test]
    fn a_target_by_the_frame_edge_stacks_where_it_has_not_moved() {
        let settings = defaults();
        let by_edge = |shift: (f32, f32)| moon((-380.0 + shift.0, shift.1));
        let reference = by_edge((0.0, 0.0));
        let mut ctx = PlanetaryStackingContext::new(1000, 1000, 1, &settings).unwrap();
        ctx.initialize_with_reference(&reference).unwrap();
        let asked = AlignmentRoi::for_alignment(1000, 1000, None, Some(ctx.reference_centroid));
        assert_eq!(asked.x, 0, "the window is not at the edge");
        let shifts = [(0.0, 0.0), (-0.4, 0.3), (0.3, -2.0), (2.0, 1.0)];
        let refused: Vec<_> =
            shifts.into_iter().filter(|&shift| !ctx.add_frame(&by_edge(shift), &settings).unwrap().added).collect();
        assert!(refused.is_empty(), "refused {refused:?} of {shifts:?}");
    }

    /// The reference's window is built once and rebuilt when the window asked for changes
    /// (a setting mid-session): a stale one would correlate another part of the reference.
    #[test]
    fn the_reference_window_follows_the_window_asked_for() {
        let tracked = defaults();
        let mut ctx = moon_context(&tracked);
        let window = |ctx: &PlanetaryStackingContext| ctx.alignment.as_ref().map(|(_, built)| built.window());
        assert!(ctx.add_frame(&moon((3.0, 1.0)), &tracked).unwrap().added);
        let centred = window(&ctx).expect("built by the first frame");
        assert_eq!(centred.width, 500, "{centred:?}");

        let chosen = AlignmentRoi::new(300, 350, 400, 300);
        let capture = CaptureSettings { planetary_auto_tracking: false, planetary_roi: Some(chosen), ..CaptureSettings::default() };
        let fixed = StackSettings::of(&capture, &Plugins::none());
        assert!(ctx.add_frame(&moon((4.0, -2.0)), &fixed).unwrap().added);
        assert_eq!(window(&ctx), Some(chosen));
        assert_shift(&ctx, (4.0, -2.0));
    }

    /// Pro's points search a few pixels around the global shift the session found, so a
    /// drifted frame's points are not each searched ±50 px around nothing.
    #[test]
    fn multi_point_alignment_is_handed_the_global_shift() {
        use night_amplifier_core::planetary::PlanetaryStackerPlugin;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Recording(Mutex<Vec<Search>>);
        impl PlanetaryStackerPlugin for Recording {
            fn warp_frame(
                &self,
                frame: &Frame,
                _: &LuminancePyramid,
                _: &LuminancePyramid,
                _: &AlignmentRoi,
                search: Search,
            ) -> night_amplifier_core::error::Result<Frame> {
                self.0.lock().unwrap().push(search);
                Ok(frame.clone())
            }
            fn clear_cache(&self) {}
        }

        let plugin = Arc::new(Recording::default());
        let plugins = Plugins::none().with_planetary(plugin.clone()).always_licensed();
        let capture = CaptureSettings { planetary_multi_point_alignment: true, ..CaptureSettings::default() };
        let settings = StackSettings::of(&capture, &plugins);
        let mut ctx = moon_context(&settings);
        assert!(ctx.add_frame(&moon((12.0, -30.0)), &settings).unwrap().added);
        let searches = plugin.0.lock().unwrap();
        let [search] = searches.as_slice() else { panic!("{searches:?}") };
        assert_eq!(search.radius, LOCAL_SEARCH_RADIUS);
        assert!((search.expected.0 + 12.0).abs() < 0.3 && (search.expected.1 - 30.0).abs() < 0.3, "{search:?}");
    }

    #[test]
    fn a_frame_before_the_reference_is_refused() {
        let settings = defaults();
        let mut ctx = PlanetaryStackingContext::new(100, 100, 1, &settings).unwrap();
        let frame = Frame::zeros(100, 100, 1).unwrap();
        assert!(matches!(ctx.add_frame(&frame, &settings), Err(LiveStackError::NoReference)));
    }

    #[test]
    fn test_planetary_stacking_context_initialization() {
        let settings = defaults();
        let mut ctx = PlanetaryStackingContext::new(100, 100, 3, &settings).unwrap();

        let frame = Frame::zeros(100, 100, 3).unwrap();
        ctx.initialize_with_reference(&frame).unwrap();

        assert!(ctx.is_initialized);
        assert_eq!(ctx.frame_count(), 1);
    }

    #[test]
    fn test_planetary_stacking_context_add_frame() {
        let settings = defaults();
        let mut ctx = PlanetaryStackingContext::new(100, 100, 1, &settings).unwrap();

        // Create a reference frame with a "planet" (a square)
        let mut ref_frame = Frame::zeros(100, 100, 1).unwrap();
        for y in 40..60 {
            for x in 40..60 {
                ref_frame.set_pixel(x, y, 0, 1.0);
            }
        }
        ctx.initialize_with_reference(&ref_frame).unwrap();

        // Create a second frame shifted by (5, 3)
        let mut next_frame = Frame::zeros(100, 100, 1).unwrap();
        for y in 43..63 {
            for x in 45..65 {
                next_frame.set_pixel(x, y, 0, 1.0);
            }
        }

        let added = ctx.add_frame(&next_frame, &settings).unwrap();
        assert!(added.added);
        assert_eq!(ctx.frame_count(), 2);

        let stacked = ctx.compute().unwrap();
        // The stacked frame should have the square back at (40, 40)
        assert!(stacked.get_pixel(40, 40, 0) > 0.0);
    }
}
