//! The `auto_crop` setting's session side: one crop decision per stack snapshot, made in the
//! stacking task, which owns the stack's lifecycle and its export.
//!
//! The rect then travels with the frame (`StackedFrame::crop`), so the render task, both
//! eyepiece sockets and the saved FITS/PNG apply the same one and cannot disagree. It is
//! applied *before* the render pipeline: a thin border inflated the whole-frame sky MAD the
//! stretch is solved from by 15-20 % on two drifting sessions. Plate solving keeps the
//! uncropped frame, whose field of view its hint describes.

use std::sync::Arc;

use tracing::{debug, warn};

use night_amplifier_core::error::Result;
use night_amplifier_core::frame::{Frame, NoiseField};
use night_amplifier_core::stacking::{CropDecision, CropRect, CropTracker};

/// One stack's crop, or none while the setting is off.
#[derive(Debug, Default)]
pub struct StackCrop {
    tracker: CropTracker,
    too_thin: bool,
}

impl StackCrop {
    pub fn new() -> Self {
        Self::default()
    }

    /// The crop for a stack snapshot of `depth` subs whose coverage map is `coverage`
    /// (`None`: every sub covered all of it). `enabled` is `auto_crop` for a frame that shows
    /// the stack; while it is false nothing is cropped and the next enable starts afresh.
    pub fn decide(&mut self, enabled: bool, depth: usize, coverage: Option<&NoiseField>) -> Option<CropRect> {
        if !enabled {
            self.reset();
            return None;
        }
        let previous = self.tracker.shown();
        let decision = self.tracker.observe(depth, coverage);
        let too_thin = decision == CropDecision::TooThin;
        if too_thin && !self.too_thin {
            warn!(depth, "Auto crop: too little of the stack is well covered to crop to, showing it whole");
        }
        self.too_thin = too_thin;
        let rect = decision.rect();
        if rect != previous {
            debug!(?rect, depth, "Auto crop changed");
        }
        rect
    }

    pub fn reset(&mut self) {
        self.tracker.reset();
        self.too_thin = false;
    }
}

/// Crops a stack snapshot to `rect` in place, and hands back its coverage map cropped alike
/// (see [`crop_coverage`]). On an error neither is touched.
pub fn crop_stack(frame: &mut Frame, coverage: Option<&NoiseField>, rect: &CropRect) -> Result<Option<NoiseField>> {
    let coverage = crop_coverage(coverage, rect)?;
    frame.crop_in_place(rect)?;
    Ok(coverage)
}

/// A stack's coverage map cut to `rect`: `None` when nothing in the crop is thinner than the
/// rest, as a complete stack carries none.
pub fn crop_coverage(coverage: Option<&NoiseField>, rect: &CropRect) -> Result<Option<NoiseField>> {
    Ok(coverage.map(|field| field.cropped(rect)).transpose()?.filter(NoiseField::is_usable))
}

/// The render task's display frame cut to `rect` and binned by `bin` in one pass, returning
/// whether it was binned. The bin reads the window in place, and an unbinned cut copies only
/// the crop when another holder (an in-flight solve) shares the frame: `make_mut` before the
/// cut copied all of it, 4.1 -> 42.7 ms per 3008² bin-2 frame. On an error the frame is
/// untouched; a failed bin is not one, the pipeline just runs at full resolution.
pub fn cut_display_frame(frame: &mut Arc<Frame>, rect: Option<&CropRect>, bin: usize) -> Result<bool> {
    let full = CropRect { x: 0, y: 0, width: frame.width(), height: frame.height() };
    let window = rect.copied().unwrap_or(full);
    window.check(frame.width(), frame.height())?;
    if bin > 1 {
        match frame.downsample_region(&window, bin) {
            Ok(binned) => {
                *frame = Arc::new(binned);
                return Ok(true);
            }
            Err(e) => warn!(error = %e, factor = bin, "Preview binning failed, rendering at full resolution"),
        }
    }
    let Some(rect) = rect else {
        return Ok(false);
    };
    match Arc::get_mut(frame) {
        Some(owned) => owned.crop_in_place(rect)?,
        None => *frame = Arc::new(frame.cropped(rect)?),
    }
    Ok(false)
}

/// Rows 0, `stride`, ... of `frame` cut to `rect` and binned by `bin`, each as
/// [`cut_display_frame`] cuts it, without cutting the rest: a moved crop's probe reads the
/// noise of the crop it left on a fraction of its rows (`PreviewAnalysis::note_left_window`).
pub fn cut_rows(frame: &Frame, rect: &CropRect, bin: usize, stride: usize) -> Result<Frame> {
    let bin = bin.max(1);
    rect.check(frame.width(), frame.height())?;
    let (width, channels) = (rect.width / bin, frame.channels());
    let rows: Vec<usize> = (0..rect.height / bin).step_by(stride.max(1)).collect();
    let plane = width * rows.len();
    let mut data = vec![0.0; plane * channels];
    for (i, &r) in rows.iter().enumerate() {
        let row = frame.downsample_region(&CropRect { y: rect.y + r * bin, height: bin, ..*rect }, bin)?;
        for c in 0..channels {
            data[c * plane + i * width..][..width].copy_from_slice(row.channel_data(c));
        }
    }
    Frame::from_f32_vec(data, width, rows.len(), channels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use night_amplifier_core::frame::NOISE_REDUCTION;

    /// 64x64 px with the leftmost 8 px column half covered.
    fn thin_left_column() -> NoiseField {
        let cells = 64 / NOISE_REDUCTION;
        let coverage = (0..cells * cells).map(|i| if i % cells == 0 { 0.5 } else { 1.0 }).collect();
        NoiseField::coverage_only(coverage, cells, cells, 3, 64, 64).unwrap()
    }

    const CROPPED: CropRect = CropRect { x: 8, y: 0, width: 56, height: 64 };
    /// A depth the tracker crops at.
    const DEEP: usize = night_amplifier_core::stacking::MIN_CROP_DEPTH;

    /// A crop's rows cut alone are the render task's cut's rows, bit for bit, binned or not.
    #[test]
    fn rows_cut_alone_are_the_rows_of_the_whole_cut() {
        let mut frame = Frame::zeros(64, 48, 3).unwrap();
        for c in 0..3 {
            for y in 0..48 {
                for x in 0..64 {
                    frame.set_pixel(x, y, c, ((x * 7 + y * 13 + c * 5) % 17) as f32 / 17.0);
                }
            }
        }
        let rect = CropRect { x: 6, y: 4, width: 50, height: 41 };
        for bin in [1, 2, 3] {
            let mut whole = Arc::new(frame.clone());
            cut_display_frame(&mut whole, Some(&rect), bin).unwrap();
            for stride in [1, 3] {
                let rows = cut_rows(&frame, &rect, bin, stride).unwrap();
                assert_eq!((rows.width(), rows.height()), (whole.width(), whole.height().div_ceil(stride)), "bin {bin} stride {stride}");
                for (i, y) in (0..whole.height()).step_by(stride).enumerate() {
                    for c in 0..3 {
                        let (got, want) = (&rows.channel_data(c)[i * rows.width()..][..rows.width()], &whole.channel_data(c)[y * whole.width()..][..whole.width()]);
                        assert_eq!(got, want, "bin {bin} stride {stride}: row {y} channel {c}");
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_is_cropped_while_the_setting_is_off() {
        let mut crop = StackCrop::new();
        for depth in 1..10 {
            assert_eq!(crop.decide(false, depth, Some(&thin_left_column())), None);
        }
    }

    #[test]
    fn a_thin_border_is_cropped_while_the_setting_is_on() {
        let mut crop = StackCrop::new();
        assert_eq!(crop.decide(true, DEEP, Some(&thin_left_column())), Some(CROPPED));
        assert_eq!(crop.decide(true, DEEP + 1, None), None, "a complete stack was cropped");
    }

    /// Off, then on again, is a fresh start: the tracker's rect does not survive the gap.
    #[test]
    fn switching_off_forgets_the_crop() {
        let mut crop = StackCrop::new();
        crop.decide(true, DEEP, Some(&thin_left_column()));
        crop.decide(false, DEEP + 1, Some(&thin_left_column()));
        assert_eq!(crop.tracker.shown(), None);
        assert_eq!(crop.decide(true, DEEP + 2, Some(&thin_left_column())), Some(CROPPED));
    }

    #[test]
    fn a_crop_takes_the_frame_and_its_map_together() {
        let mut frame = Frame::filled(64, 64, 3, 0.25).unwrap();
        let coverage = crop_stack(&mut frame, Some(&thin_left_column()), &CROPPED).unwrap();
        assert_eq!((frame.width(), frame.height()), (56, 64));
        assert!(coverage.is_none(), "a crop left with no thin cell still carried its map");

        let mut frame = Frame::filled(64, 64, 3, 0.25).unwrap();
        let wider = CropRect { x: 0, y: 0, width: 32, height: 64 };
        let coverage = crop_stack(&mut frame, Some(&thin_left_column()), &wider).unwrap().unwrap();
        assert_eq!((coverage.source_width(), coverage.source_height()), (32, 64));
        assert_eq!(coverage.sample_coverage(0, 32), 0.5);
    }

    /// Every sample names its own position, so a cut from the wrong origin shows.
    fn labelled(width: usize, height: usize) -> Frame {
        let mut frame = Frame::zeros(width, height, 3).unwrap();
        for c in 0..3 {
            for y in 0..height {
                for x in 0..width {
                    frame.set_pixel(x, y, c, (c * 10_000 + y * 100 + x) as f32);
                }
            }
        }
        frame
    }

    /// A frame another holder shares (an in-flight solve) is cut into a frame of its own,
    /// binned or not, and the holder's copy stays whole; owned, the same samples come out.
    #[test]
    fn a_shared_display_frame_is_cut_without_touching_its_other_holder() {
        let whole = labelled(64, 48);
        let rect = CropRect { x: 8, y: 8, width: 48, height: 40 };
        let mut reference = whole.clone();
        reference.crop_in_place(&rect).unwrap();
        for bin in [1, 2, 3] {
            let expected = reference.downsample(bin).unwrap();
            for shared in [true, false] {
                let mut frame = Arc::new(whole.clone());
                let holder = shared.then(|| Arc::clone(&frame));
                let binned = cut_display_frame(&mut frame, Some(&rect), bin).unwrap();
                assert_eq!(binned, bin > 1);
                assert!(frame.data() == expected.data(), "bin {bin}, shared {shared}");
                if let Some(holder) = holder {
                    assert!(!Arc::ptr_eq(&holder, &frame));
                    assert!(holder.data() == whole.data(), "bin {bin}: the solve's frame changed");
                }
            }
        }
    }

    #[test]
    fn an_uncut_display_frame_is_only_binned() {
        let whole = labelled(64, 48);
        let mut frame = Arc::new(whole.clone());
        assert!(!cut_display_frame(&mut frame, None, 1).unwrap());
        assert!(frame.data() == whole.data());
        assert!(cut_display_frame(&mut frame, None, 2).unwrap());
        assert!(frame.data() == whole.downsample(2).unwrap().data());
    }

    #[test]
    fn a_display_cut_that_does_not_fit_leaves_the_frame_whole() {
        let mut frame = Arc::new(labelled(64, 48));
        let outside = CropRect { x: 32, y: 0, width: 40, height: 48 };
        for bin in [1, 2] {
            assert!(cut_display_frame(&mut frame, Some(&outside), bin).is_err());
            assert_eq!((frame.width(), frame.height()), (64, 48));
        }
    }

    #[test]
    fn a_crop_that_does_not_fit_leaves_the_frame_whole() {
        let mut frame = Frame::filled(64, 64, 3, 0.25).unwrap();
        let outside = CropRect { x: 32, y: 0, width: 40, height: 64 };
        assert!(crop_stack(&mut frame, Some(&thin_left_column()), &outside).is_err());
        assert_eq!((frame.width(), frame.height()), (64, 64));
    }
}
