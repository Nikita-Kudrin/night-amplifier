use super::*;
use crate::background::protection::ProtectedRegion;

const SKY: f32 = 0.05;
/// Node-to-node scatter of a deep stack's star-clipped medians, as a share of the sky.
const NOISE: f32 = 0.002;

fn layout() -> NodeLayout {
    NodeLayout::new(1504, 1504, 12, 12, 0.035)
}

/// Deterministic noise in [-1, 1].
fn jitter(i: usize) -> f32 {
    let h = (i as u32).wrapping_mul(2_654_435_761).rotate_left(13).wrapping_mul(2_246_822_519);
    (h >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

/// One channel sampled at every node from `sky(x, y)` in frame fractions.
fn sample(layout: &NodeLayout, sky: impl Fn(f32, f32) -> f32) -> Vec<Vec<Option<f32>>> {
    let values = layout
        .nodes()
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (x, y) = (n.x as f32 / (layout.width - 1) as f32, n.y as f32 / (layout.height - 1) as f32);
            Some(sky(x, y) + NOISE * SKY * jitter(i))
        })
        .collect();
    vec![values]
}

fn galaxy(x: f32, y: f32, (cx, cy): (f32, f32), r: f32, peak: f32) -> f32 {
    peak * (-((x - cx).powi(2) + (y - cy).powi(2)) / (2.0 * r * r)).exp()
}

fn corner_glow(x: f32, y: f32) -> f32 {
    0.08 * (-((x).powi(2) + (1.0 - y).powi(2)).sqrt() / 0.2).exp()
}

/// Nodes whose centre is within `r` of `(cx, cy)`.
fn inside(layout: &NodeLayout, (cx, cy): (f32, f32), r: f32) -> Vec<usize> {
    layout
        .nodes()
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            let (x, y) = (n.x as f32 / (layout.width - 1) as f32, n.y as f32 / (layout.height - 1) as f32);
            (x - cx).hypot(y - cy) < r
        })
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn a_linear_gradient_is_all_sky() {
    let l = layout();
    let c = classify(&l, &sample(&l, |x, y| SKY + 0.02 * x + 0.01 * y), 0, None).unwrap();
    assert!(c.sky.iter().all(|&s| s), "{} nodes rejected", c.sky.iter().filter(|&&s| !s).count());
}

#[test]
fn a_noiseless_flat_frame_is_all_sky() {
    let l = layout();
    let samples = vec![vec![Some(0.3f32); l.cols() * l.rows()]];
    let c = classify(&l, &samples, 0, None).unwrap();
    assert!(c.sky.iter().all(|&s| s));
}

/// Orion 35 mm: a horizon glow filling a corner. Brightness pruning took it for
/// nebulosity and left 70 output levels of it on screen.
#[test]
fn a_corner_glow_is_sky_and_followed() {
    let l = layout();
    let c = classify(&l, &sample(&l, |x, y| SKY + corner_glow(x, y)), 0, None).unwrap();
    let corner = (l.rows() - 1) * l.cols();
    assert!(c.sky[corner], "the brightest corner node must be sky");
    let missed = c.values[0][corner] - (SKY + corner_glow(0.0, 1.0));
    assert!(missed.abs() < 0.002, "the model misses the corner by {missed}");
}

/// M57 / ring-2: a band rising toward one edge that then drops in the last 4 % behind
/// vignetting, so its peak sits one line in from the border.
#[test]
fn an_edge_band_behind_a_vignetted_edge_is_sky() {
    let l = layout();
    let band = |x: f32| 0.02 * (-((x - 0.92) / 0.12).powi(2)).exp() - if x > 0.96 { 0.006 } else { 0.0 };
    let c = classify(&l, &sample(&l, |x, _| SKY + band(x)), 0, None).unwrap();
    let rejected = c.sky.iter().filter(|&&s| !s).count();
    assert_eq!(rejected, 0, "an edge band is glow, not a target");
}

/// A centred galaxy is rejected, grown by a node, and refilled from the sky under it.
#[test]
fn a_centred_galaxy_is_kept_out_of_the_model() {
    let l = layout();
    let centre = (0.5, 0.45);
    let sky = |x: f32, _y: f32| SKY + 0.01 * x;
    let c = classify(&l, &sample(&l, |x, y| sky(x, y) + galaxy(x, y, centre, 0.08, 0.03)), 0, None).unwrap();
    for i in inside(&l, centre, 0.12) {
        assert!(!c.sky[i], "node {i} on the galaxy counted as sky");
    }
    let core = inside(&l, centre, 0.05)[0];
    let n = l.nodes()[core];
    let want = sky(n.x as f32 / 1503.0, n.y as f32 / 1503.0);
    let lifted = c.values[0][core] - want;
    assert!(lifted.abs() < 0.001, "the model under the core is lifted by {lifted} (galaxy peak 0.03)");
}

/// Both at once: the glow is followed, the galaxy is not.
#[test]
fn a_galaxy_beside_a_corner_glow_is_still_rejected() {
    let l = layout();
    let centre = (0.6, 0.4);
    let c = classify(&l, &sample(&l, |x, y| SKY + corner_glow(x, y) + galaxy(x, y, centre, 0.07, 0.03)), 0, None).unwrap();
    assert!(c.sky[(l.rows() - 1) * l.cols()], "the glow corner must be sky");
    for i in inside(&l, centre, 0.1) {
        assert!(!c.sky[i], "galaxy node {i} counted as sky");
    }
}

/// A nebula touching the frame edge reads as glow by shape; catalog protection keeps it.
#[test]
fn protection_keeps_an_edge_nebula_out_of_the_model() {
    let l = layout();
    let centre = (0.95, 0.8);
    let samples = sample(&l, |x, y| SKY + galaxy(x, y, centre, 0.1, 0.01));
    let unprotected = classify(&l, &samples, 0, None).unwrap();
    let nodes = inside(&l, centre, 0.08);
    assert!(nodes.iter().all(|&i| unprotected.sky[i]), "the premise: shape alone calls it glow");

    let protection = FieldProtection {
        generation: 1,
        width: 1504,
        height: 1504,
        regions: vec![ProtectedRegion { cx: 0.95 * 1503.0, cy: 0.8 * 1503.0, semi_major: 120.0, semi_minor: 120.0, angle: 0.0 }],
    };
    let protected = classify(&l, &samples, 0, Some(&protection)).unwrap();
    assert!(nodes.iter().all(|&i| !protected.sky[i]), "protected nodes must not be sky");
}

#[test]
fn protection_over_most_of_the_field_is_ignored() {
    let l = layout();
    let protection = FieldProtection {
        generation: 1,
        width: 1504,
        height: 1504,
        regions: vec![ProtectedRegion { cx: 752.0, cy: 752.0, semi_major: 1000.0, semi_minor: 1000.0, angle: 0.0 }],
    };
    let c = classify(&l, &sample(&l, |x, _| SKY + 0.01 * x), 0, Some(&protection)).unwrap();
    assert!(c.sky.iter().all(|&s| s));
}

/// The mask is decided on the stats channel and applied to the others.
#[test]
fn every_channel_shares_the_stats_channel_mask() {
    let l = layout();
    let centre = (0.5, 0.5);
    let red = sample(&l, |x, y| SKY + galaxy(x, y, centre, 0.08, 0.03)).remove(0);
    let flat = vec![Some(0.1f32); red.len()];
    let c = classify(&l, &[red, flat], 0, None).unwrap();
    let core = inside(&l, centre, 0.07)[0];
    assert!(!c.sky[core]);
    assert!((c.values[1][core] - 0.1).abs() < 1e-4, "a flat channel refills flat");
}

#[test]
fn too_few_nodes_with_values_is_none() {
    let l = NodeLayout::new(100, 100, 2, 2, 0.0);
    assert!(classify(&l, &[vec![Some(0.1), None, None, Some(0.1)]], 0, None).is_none());
}

/// An object larger than the field (M31 in a 1 deg frame) leaves no sky to fit, so its
/// footprint is ignored; M32 beside it is still a target, and must stay protected.
#[test]
fn an_oversized_footprint_does_not_unprotect_its_neighbours() {
    let l = layout();
    let disc = |cx: f32, cy: f32, r: f32| ProtectedRegion { cx, cy, semi_major: r, semi_minor: r, angle: 0.0 };
    let field = FieldProtection {
        generation: 1,
        width: l.width,
        height: l.height,
        regions: vec![disc(752.0, 752.0, 700.0), disc(1350.0, 1350.0, 60.0)],
    };
    let protected = protected_nodes(&l, Some(&field));
    let near_m32 = inside(&l, (1350.0 / 1503.0, 1350.0 / 1503.0), 0.04);
    assert!(!near_m32.is_empty());
    assert!(near_m32.iter().all(|&i| protected[i]), "the small footprint was dropped with the large one");
}

/// Footprints that each fit but together cover most of the field leave no sky either.
#[test]
fn footprints_covering_most_of_the_field_together_are_ignored() {
    let l = layout();
    let disc = |cx: f32, cy: f32| ProtectedRegion { cx, cy, semi_major: 420.0, semi_minor: 420.0, angle: 0.0 };
    let regions = vec![disc(376.0, 376.0), disc(1128.0, 376.0), disc(376.0, 1128.0), disc(1128.0, 1128.0)];
    let field = FieldProtection { generation: 1, width: l.width, height: l.height, regions };
    assert!(protected_nodes(&l, Some(&field)).iter().all(|&p| !p));
}
