//! One detection per bright patch, for the triangle ladder's input.
//!
//! Noise on a bright extended object breaks it into dozens of local maxima, and being
//! on the object they are the brightest detections in the list: on the Cat's Eye
//! fixture (0.7 s subs) 27 of the ladder's 30 stars were nebula, whose positions change
//! sub to sub, and every fit it made was a coincidence. Keeping only the brightest
//! detection within [`CLUTTER_RADIUS_PX`] collapses such a patch to a few points while
//! leaving a star field alone.

use crate::detection::Star;

/// Radius within which a fainter detection is read as part of a brighter one's patch.
///
/// Measured as the ladder's plausible fits per fixture, unsuppressed → 15 px → 25 px:
/// Cat's Eye 0 → 20 → 23 of 26, M101 34 → 37 → 37, Orion unchanged at either radius
/// (its M42 fragments collapse from 200 to 41 detections without losing a fit). A
/// double closer than this keeps only its brighter member, which costs the ladder
/// nothing: it needs well-spread triangles, not every star.
pub(super) const CLUTTER_RADIUS_PX: f32 = 25.0;

/// `stars` without any detection lying within [`CLUTTER_RADIUS_PX`] of a brighter one
/// that was kept, brightest first.
pub(super) fn suppress_clutter(stars: &[Star]) -> Vec<Star> {
    let mut by_flux: Vec<&Star> = stars.iter().collect();
    by_flux.sort_by(|a, b| b.flux.total_cmp(&a.flux));

    let limit = CLUTTER_RADIUS_PX * CLUTTER_RADIUS_PX;
    let mut kept: Vec<Star> = Vec::with_capacity(stars.len());
    for star in by_flux {
        let crowded = kept
            .iter()
            .any(|k| (k.x - star.x).powi(2) + (k.y - star.y).powi(2) < limit);
        if !crowded {
            kept.push(*star);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn star(x: f32, y: f32, flux: f32) -> Star {
        Star::new(x, y, flux, 0.5, 30.0)
    }

    #[test]
    fn a_patch_of_maxima_collapses_to_its_brightest() {
        let mut stars: Vec<Star> = (0..40)
            .map(|i| star(500.0 + (i % 7) as f32 * 6.0, 500.0 + (i / 7) as f32 * 6.0, 10.0 + i as f32))
            .collect();
        stars.push(star(1500.0, 200.0, 5.0));

        let kept = suppress_clutter(&stars);

        assert!(kept.len() <= 4, "a 40-maximum patch kept {} detections", kept.len());
        assert_eq!(kept[0].flux, 49.0, "the brightest of the patch survives first");
        assert!(kept.iter().any(|s| s.x == 1500.0), "an isolated faint star survives");
    }

    #[test]
    fn a_star_field_is_left_alone() {
        let stars: Vec<Star> =
            (0..64).map(|i| star((i % 8) as f32 * 300.0, (i / 8) as f32 * 300.0, i as f32)).collect();

        assert_eq!(suppress_clutter(&stars).len(), stars.len());
    }

    #[test]
    fn the_result_is_brightest_first() {
        let stars = vec![star(0.0, 0.0, 1.0), star(100.0, 0.0, 3.0), star(200.0, 0.0, 2.0)];
        let fluxes: Vec<f32> = suppress_clutter(&stars).iter().map(|s| s.flux).collect();

        assert_eq!(fluxes, vec![3.0, 2.0, 1.0]);
    }
}
