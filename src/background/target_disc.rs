//! Keeping a frame-filling target out of the background model.
//!
//! Brightness pruning measures its thresholds on the nodes themselves, so when a halo
//! covers most of the frame the halo *is* the reference: on the 2026-09-14 globular
//! (IMX533) the spline carried a 1.6 sigma plateau and took 49-56 % of the glow at
//! 256 px. No brightness rule separates a halo from a gradient — a quadratic baseline
//! took 96 %. Crowding does: halo nodes sit on unresolved stars and read 11-39 % rougher
//! than open sky, which a gradient never does (`NodeSample::scatter` fits a plane out).
//!
//! So crowding only *finds* the target; a disc around it, sized by where the node excess
//! over an outer surface falls to noise, is refilled from that surface. The spline can then
//! neither bend into the target nor extrapolate the rim's inward slope. Halo taken at
//! 256 px: 49 % -> 11 % (2048 crop), 56 % -> 18 % (full frame); no disc on 12 synthetic
//! linear/horizon/dome/bowl gradients.
//!
//! The surface is a plane unless the outer nodes are clearly curved. Uncalibrated
//! vignetting is a dome, which a plane reads as ring excess (the disc ran to [`MAX_RADIUS`])
//! and cuts out of the model: 2.78 of a 7.66 sigma rise followed under a centred cluster.
//! A quadratic always, though, follows a halo's own wing where it fills the frame (field
//! crop: 5/10 % -> 18/36 % taken), so it must earn its place — [`MAX_CURVED_RESIDUAL`].
//!
//! Shared by both extractors: the RBF spline (Pro) and the bilinear grid, each refilling
//! the disc's nodes before its own interpolation.

use nalgebra::{DMatrix, DVector};
use rayon::prelude::*;

use super::grid::{extract_node_sample, median, GridNode, NodeSample};
use crate::frame::Frame;

/// Relative scatter excess, in robust sigmas of that excess across nodes, marking a seed.
const SEED_SIGMA: f32 = 3.0;

/// Ring excess, in its own standard errors, that still counts as target.
const EDGE_SIGMA: f32 = 3.0;

/// Outermost share of nodes (by distance from the target) the reference plane is fit on.
const OUTER_SHARE: f32 = 0.3;

/// Largest disc radius, in normalised frame units. Beyond it the excess measured about a
/// cluster and about the sensor centre were indistinguishable (session 1 vs 2 of the
/// field night) — vignetting, which the model *should* remove.
const MAX_RADIUS: f32 = 0.45;

/// Nodes that must stay outside the disc for the spline to have a frame to fit.
const MIN_OUTSIDE: usize = 24;

/// Outer-node residual RMS a quadratic must get under, as a share of the plane's, to be
/// used (it explains >= 3/4 of the plane's residual variance). Measured: vignetting 0.09,
/// horizon glow 0.29, the field globular's halo wing 0.83, flat sky 0.97.
const MAX_CURVED_RESIDUAL: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetDisc {
    pub cx: f32,
    pub cy: f32,
    pub radius: f32,
    /// The outer nodes are curved (vignetting), so the refill is quadratic, not a plane.
    pub curved: bool,
}

impl TargetDisc {
    /// Every node's [`NodeSample`] on `channel` (the one pruning reads), for [`Self::find`].
    /// Its `value` is that channel's node value, so extractors don't clip it twice.
    pub fn sample_nodes(frame: &Frame, nodes: &[GridNode], box_size: usize, channel: usize) -> Vec<Option<NodeSample>> {
        let _span = tracing::info_span!("target_disc_samples").entered();
        nodes
            .par_iter()
            .map(|node| extract_node_sample(frame, node, box_size, channel))
            .collect()
    }

    /// The crowded target in this grid, if there is one. `samples[i]` belongs to
    /// `nodes[i]`, extracted from the pruning channel before any pruning.
    pub fn find(
        nodes: &[GridNode],
        samples: &[Option<NodeSample>],
        grid_cols: usize,
        grid_rows: usize,
        width: usize,
        height: usize,
    ) -> Option<Self> {
        let valid: Vec<usize> = (0..nodes.len())
            .filter(|&i| samples[i].is_some_and(|s| s.samples > 0 && s.scatter.is_finite()))
            .collect();
        if valid.len() < MIN_OUTSIDE {
            return None;
        }
        let sample = |i: usize| samples[i].expect("filtered to valid");

        let mut scatters: Vec<f32> = valid.iter().map(|&i| sample(i).scatter).collect();
        let typical = median(&mut scatters);
        if typical.is_nan() || typical <= 0.0 {
            return None;
        }
        let excess = |i: usize| sample(i).scatter / typical - 1.0;
        let excesses: Vec<f32> = valid.iter().map(|&i| excess(i)).collect();
        let spread = robust_sigma(&excesses);
        if spread.is_nan() || spread <= 0.0 {
            return None;
        }

        let crowded: Vec<bool> = (0..nodes.len())
            .map(|i| samples[i].is_some() && excess(i) > SEED_SIGMA * spread)
            .collect();
        let seeds: Vec<usize> = (0..nodes.len())
            .filter(|&i| crowded[i] && has_crowded_neighbour(&nodes[i], &crowded, grid_cols, grid_rows))
            .collect();
        if seeds.len() < 2 {
            return None;
        }

        let pos = |n: &GridNode| (n.x as f32 / width as f32, n.y as f32 / height as f32);
        let weight: f32 = seeds.iter().map(|&i| excess(i)).sum();
        let cx = seeds.iter().map(|&i| excess(i) * pos(&nodes[i]).0).sum::<f32>() / weight;
        let cy = seeds.iter().map(|&i| excess(i) * pos(&nodes[i]).1).sum::<f32>() / weight;
        let r: Vec<f32> = nodes
            .iter()
            .map(|n| (pos(n).0 - cx).hypot(pos(n).1 - cy))
            .collect();

        let mut sorted: Vec<f32> = valid.iter().map(|&i| r[i]).collect();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let outer_from = sorted[((1.0 - OUTER_SHARE) * (sorted.len() - 1) as f32) as usize];
        let outer: Vec<usize> = valid.iter().copied().filter(|&i| r[i] >= outer_from).collect();
        let points = |set: &[usize]| -> Vec<(f32, f32, f32)> {
            set.iter()
                .map(|&i| (pos(&nodes[i]).0, pos(&nodes[i]).1, sample(i).value))
                .collect()
        };
        let outer_points = points(&outer);
        let plane = Surface::fit(&outer_points, false)?;
        let quadratic = Surface::fit(&outer_points, true)
            .filter(|q| q.rms(&outer_points) < MAX_CURVED_RESIDUAL * plane.rms(&outer_points));
        let curved = quadratic.is_some();
        let reference = quadratic.unwrap_or(plane);
        let residual = |i: usize| {
            let (x, y) = pos(&nodes[i]);
            sample(i).value - reference.at(x, y)
        };
        let outer_residuals: Vec<f32> = outer.iter().map(|&i| residual(i)).collect();
        let outer_spread = robust_sigma(&outer_residuals);

        let step = 1.0 / grid_cols.max(grid_rows) as f32;
        let mut radius = seeds.iter().map(|&i| r[i]).fold(0.0, f32::max);
        let mut lo = 0.0;
        while lo < sorted.last().copied().unwrap_or(0.0) {
            let ring: Vec<usize> = valid.iter().copied().filter(|&i| r[i] >= lo && r[i] < lo + step).collect();
            lo += step;
            if ring.len() < 3 || lo <= radius {
                continue;
            }
            let n = (ring.len() as f32).sqrt();
            let mut errors: Vec<f32> = ring
                .iter()
                .map(|&i| 1.2533 * sample(i).scatter / (sample(i).samples as f32).sqrt())
                .collect();
            let noise = (median(&mut errors) / n).max(outer_spread / n);
            let mut ring_residuals: Vec<f32> = ring.iter().map(|&i| residual(i)).collect();
            if median(&mut ring_residuals) > EDGE_SIGMA * noise {
                radius = lo;
            } else {
                break;
            }
        }

        radius = radius.min(MAX_RADIUS);
        while radius > 0.0 && r.iter().filter(|&&d| d >= radius).count() < MIN_OUTSIDE {
            radius -= step / 2.0;
        }
        (radius > 0.0).then_some(Self {
            cx,
            cy,
            radius,
            curved,
        })
    }

    pub fn contains(&self, node: &GridNode, width: usize, height: usize) -> bool {
        let (x, y) = (node.x as f32 / width as f32, node.y as f32 / height as f32);
        (x - self.cx).hypot(y - self.cy) < self.radius
    }

    /// Replace every node inside the disc with this channel's [`Surface`] through the
    /// surviving nodes outside it. Returns false, leaving `grid` untouched, when too
    /// few survive to fit one.
    pub fn fill(&self, grid: &mut [GridNode], width: usize, height: usize) -> bool {
        let outside: Vec<(f32, f32, f32)> = grid
            .iter()
            .filter(|n| !self.contains(n, width, height))
            .filter_map(|n| n.value.map(|v| (n.x as f32 / width as f32, n.y as f32 / height as f32, v)))
            .collect();
        let Some(surface) = Surface::fit(&outside, self.curved) else {
            return false;
        };
        for node in grid.iter_mut().filter(|n| self.contains(n, width, height)) {
            node.value = Some(surface.at(node.x as f32 / width as f32, node.y as f32 / height as f32));
        }
        true
    }
}

/// 1.4826 x MAD.
fn robust_sigma(values: &[f32]) -> f32 {
    let centre = median(&mut values.to_vec());
    1.4826 * median(&mut values.iter().map(|v| (v - centre).abs()).collect::<Vec<_>>())
}

fn has_crowded_neighbour(node: &GridNode, crowded: &[bool], cols: usize, rows: usize) -> bool {
    (-1i32..=1)
        .flat_map(|dr| (-1i32..=1).map(move |dc| (dr, dc)))
        .filter(|&d| d != (0, 0))
        .any(|(dr, dc)| {
            let (r, c) = (node.row as i32 + dr, node.col as i32 + dc);
            r >= 0 && c >= 0 && (r as usize) < rows && (c as usize) < cols && crowded[r as usize * cols + c as usize]
        })
}

/// Points below which the quadratic terms are not fitted: six unknowns need a margin over
/// node noise, or a sparse rim bends the refill.
const MIN_QUADRATIC_POINTS: usize = 12;

/// `v = a + b u + c w` (+ `d u² + e uw + f w²` when quadratic) about the frame centre
/// (`u = x - 0.5`), least squares.
#[derive(Debug, Clone, Copy)]
struct Surface([f64; 6]);

impl Surface {
    /// `None` below 3 points. A quadratic asked of fewer than [`MIN_QUADRATIC_POINTS`] is a
    /// plane, so it can never beat one by [`MAX_CURVED_RESIDUAL`].
    fn fit(points: &[(f32, f32, f32)], quadratic: bool) -> Option<Self> {
        let terms = match points.len() {
            n if quadratic && n >= MIN_QUADRATIC_POINTS => 6,
            n if n >= 3 => 3,
            _ => return None,
        };
        let a = DMatrix::from_fn(points.len(), terms, |i, j| Self::basis(points[i].0, points[i].1)[j]);
        let b = DVector::from_iterator(points.len(), points.iter().map(|p| p.2 as f64));
        let solution = a.svd(true, true).solve(&b, 1e-12).ok()?;
        let mut coefficients = [0.0; 6];
        coefficients[..terms].copy_from_slice(solution.as_slice());
        Some(Self(coefficients))
    }

    fn basis(x: f32, y: f32) -> [f64; 6] {
        let (u, w) = (x as f64 - 0.5, y as f64 - 0.5);
        [1.0, u, w, u * u, u * w, w * w]
    }

    fn at(&self, x: f32, y: f32) -> f32 {
        Self::basis(x, y).iter().zip(&self.0).map(|(b, c)| b * c).sum::<f64>() as f32
    }

    fn rms(&self, points: &[(f32, f32, f32)]) -> f64 {
        let sum: f64 = points.iter().map(|&(x, y, v)| ((v - self.at(x, y)) as f64).powi(2)).sum();
        (sum / points.len().max(1) as f64).sqrt()
    }
}

#[cfg(test)]
mod tests {
    include!("target_disc_tests.rs");
}
