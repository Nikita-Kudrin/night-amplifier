//! Catalog objects the background model must not follow.
//!
//! Morphology alone cannot tell a faint nebula from a glow entering the same edge; the
//! catalog can (IC 63: the model lifts 2.4 sky sigmas under its core by shape alone, 2.0
//! with its footprint, the rest being the glow itself). Once a session's stack is plate
//! solved, the catalog knows: every object in the field becomes an ellipse here, and the
//! nodes inside it are never sky. Coordinates are pixels of the frame the regions were
//! solved on; [`FieldProtection::mapped`] carries them to a cropped, binned display frame.

use crate::stacking::CropRect;

/// Each footprint is grown by this factor: catalog axes trace a surface-brightness
/// isophote, and the skirt beyond it still lifts a node's median.
const MARGIN: f32 = 1.3;

/// One object's footprint: an ellipse, `angle` (radians) from +x towards +y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProtectedRegion {
    pub cx: f32,
    pub cy: f32,
    pub semi_major: f32,
    pub semi_minor: f32,
    pub angle: f32,
}

impl ProtectedRegion {
    /// Whether `(x, y)` lies inside the footprint grown by its margin.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (dx, dy) = (x - self.cx, y - self.cy);
        let (sin, cos) = self.angle.sin_cos();
        let (u, v) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        let (a, b) = self.axes();
        (u / a).powi(2) + (v / b).powi(2) <= 1.0
    }

    /// The box `(x0, y0, x1, y1)` holding the grown footprint.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let (a, b) = self.axes();
        let (sin, cos) = self.angle.sin_cos();
        let hx = ((a * cos).powi(2) + (b * sin).powi(2)).sqrt();
        let hy = ((a * sin).powi(2) + (b * cos).powi(2)).sqrt();
        (self.cx - hx, self.cy - hy, self.cx + hx, self.cy + hy)
    }

    fn axes(&self) -> (f32, f32) {
        (self.semi_major * MARGIN, self.semi_minor.max(1.0) * MARGIN)
    }
}

/// Every protected region of one frame geometry. `generation` changes with each new
/// solve, so a cache keyed on it notices one arriving mid-session.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldProtection {
    pub generation: u64,
    pub width: usize,
    pub height: usize,
    pub regions: Vec<ProtectedRegion>,
}

impl FieldProtection {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.regions.iter().any(|r| r.contains(x, y))
    }

    /// The same regions on `source` cut to `crop` (in its pixels, `None` for the whole
    /// frame) and resampled to `width` pixels wide — the render task's display frame.
    /// Binning is uniform, so one scale serves both axes. `None` when `source` is not the
    /// size they were solved on: a stack replaced since, whose footprints these are not.
    pub fn mapped(&self, source: (usize, usize), crop: Option<&CropRect>, width: usize, height: usize) -> Option<Self> {
        if source != (self.width, self.height) {
            return None;
        }
        let (x0, y0, crop_width) = crop.map_or((0, 0, self.width), |c| (c.x, c.y, c.width));
        let scale = width as f32 / crop_width.max(1) as f32;
        Some(Self {
            generation: self.generation,
            width,
            height,
            regions: self
                .regions
                .iter()
                .map(|r| ProtectedRegion {
                    cx: (r.cx - x0 as f32) * scale,
                    cy: (r.cy - y0 as f32) * scale,
                    semi_major: r.semi_major * scale,
                    semi_minor: r.semi_minor * scale,
                    angle: r.angle,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(cx: f32, cy: f32, r: f32) -> ProtectedRegion {
        ProtectedRegion { cx, cy, semi_major: r, semi_minor: r, angle: 0.0 }
    }

    #[test]
    fn a_rotated_ellipse_contains_along_its_major_axis_only() {
        let region = ProtectedRegion {
            cx: 100.0,
            cy: 100.0,
            semi_major: 40.0,
            semi_minor: 5.0,
            angle: std::f32::consts::FRAC_PI_4,
        };
        let field = FieldProtection { generation: 1, width: 200, height: 200, regions: vec![region] };
        assert!(field.contains(125.0, 125.0), "on the major axis, 35 px out");
        assert!(!field.contains(125.0, 75.0), "on the minor axis, 35 px out");
    }

    /// The edge profiles test only the footprints whose box reaches their band, so the box
    /// must hold every point `contains` accepts, and no more than the ellipse needs.
    #[test]
    fn the_bounds_hold_a_rotated_footprint_tightly() {
        let region = ProtectedRegion { cx: 100.0, cy: 80.0, semi_major: 40.0, semi_minor: 8.0, angle: 0.6 };
        let (x0, y0, x1, y1) = region.bounds();
        let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for y in 0..200 {
            for x in 0..200 {
                let (fx, fy) = (x as f32, y as f32);
                if region.contains(fx, fy) {
                    assert!(fx >= x0 && fx <= x1 && fy >= y0 && fy <= y1, "({fx}, {fy}) outside {:?}", region.bounds());
                    (lo_x, lo_y, hi_x, hi_y) = (lo_x.min(fx), lo_y.min(fy), hi_x.max(fx), hi_y.max(fy));
                }
            }
        }
        for (bound, reached) in [(x0, lo_x), (y0, lo_y), (x1, hi_x), (y1, hi_y)] {
            assert!((bound - reached).abs() < 1.5, "bound {bound} beyond the footprint's {reached}");
        }
    }

    #[test]
    fn the_margin_grows_every_footprint() {
        let field = FieldProtection { generation: 1, width: 100, height: 100, regions: vec![disc(50.0, 50.0, 10.0)] };
        assert!(field.contains(62.0, 50.0), "inside 1.3x the radius");
        assert!(!field.contains(64.0, 50.0), "outside 1.3x the radius");
    }

    #[test]
    fn mapping_follows_the_crop_and_the_bin() {
        let field = FieldProtection { generation: 7, width: 3008, height: 3008, regions: vec![disc(1000.0, 600.0, 100.0)] };
        let crop = CropRect { x: 200, y: 100, width: 2000, height: 2000 };
        let shown = field.mapped((3008, 3008), Some(&crop), 1000, 1000).unwrap();
        assert_eq!(shown.generation, 7);
        let r = shown.regions[0];
        assert_eq!((r.cx, r.cy, r.semi_major), (400.0, 250.0, 50.0));
        assert_eq!(field.mapped((3008, 3008), None, 1504, 1504).unwrap().regions[0].cx, 500.0);
    }

    /// Footprints solved on one stack say nothing about a stack of another size.
    #[test]
    fn a_frame_of_another_size_is_not_mapped() {
        let field = FieldProtection { generation: 1, width: 3008, height: 3008, regions: vec![disc(1000.0, 600.0, 100.0)] };
        assert!(field.mapped((1504, 1504), None, 1504, 1504).is_none());
        assert!(field.mapped((3008, 2000), None, 1504, 1000).is_none());
    }
}
