//! Which grid nodes are sky, shared by the bilinear and RBF extractors.
//!
//! A target is an excess that peaks *inside* the frame; a glow (moon, light pollution,
//! horizon) peaks at its *edge*. So: fit a robust polynomial ladder to the nodes and
//! reject those well above it, readmit rejected nodes whose climb up the excess ends
//! within [`EDGE_NODES`] of the border, grow what is left through its wing and by its own
//! size, and refill it from the fit. Replaced brightness pruning and a crowding disc
//! that, on ten real stacks, threw away corner glows and edge bands (Orion 35 mm: 70 ->
//! 4 output levels of sky range, M57 22 -> 4) while keeping fireworks' arms 0.81 -> 0.90
//! and M33's 0.53 -> 0.78. Known limit: a faint nebula overlapping a glow from the same
//! edge can partly read as glow; catalog protection ([`FieldProtection`]) is the backstop.

use super::grid::{layout_nodes, median, node_axis, GridNode};
use super::poly::{robust_sigma, terms, Point, Poly, DEGREE_GAIN, MAX_DEGREE};
use super::protection::FieldProtection;

/// Residual, in robust sigmas of the survivors', above which a node is not sky.
const K_ABOVE: f64 = 3.0;

/// Below which it is not sky either: looser, since a dark node is rarely a target.
const K_BELOW: f64 = 6.0;

/// A target grows from its [`K_ABOVE`] seeds through connected nodes whose 3x3 mean
/// residual (sky neighbours only) is still this many sigmas up: a halo's wing sits under
/// any single cut and, left in the sky, is inpainted under the core (M33 kept 0.59 of its
/// excess without growth, 0.64 with, before size-proportional dilation). The mean, not
/// the node: grown on single nodes at 1 sigma, a sixth of plain sky qualified and the mask wandered from depth to depth (a
/// held model and a fresh one rendered faint patches 2.2 output levels apart).
const K_GROW: f64 = 1.0;

/// Fit -> grow rounds: each regrowth refits without the wing it just found.
const GROW_ROUNDS: usize = 3;

/// Node lines a target may grow past its seeds, in all rounds together. Unbounded, it
/// ran through any coherent faint texture (a lattice of faint patches: 9 nodes at one
/// depth, 40 at the next, the render jumping 2.5 output levels between them).
const GROW_LINES: usize = 2;

/// Fit -> reject rounds per degree; the mask settles in 2-4 on real stacks.
const ROUNDS: usize = 10;

/// A rejected component peaking this close to the border (in node lines) is glow. 2, not
/// 1: with the inset lines a glow's peak can sit on the second line in, behind an edge
/// darkened by vignetting (M57: 0.9 of the width, then a drop in the last 4 %).
const EDGE_NODES: usize = 2;

/// Node lines a target is grown by, so its skirt does not pull the model up into it:
/// its own radius (a disc of its node count), at least [`DILATE_MIN`], at most
/// [`DILATE_MAX`]. A halo scales with its object, and a broad smooth wing reads as sky
/// to any fit that can bend: a fixed one line left the RBF model 39 % of a globular's
/// glow at 256-384 px.
const DILATE_MIN: usize = 1;
const DILATE_MAX: usize = 3;

/// Below this share of the frame under the sky nodes' bounding box, the fill is their level,
/// not a surface: a target filling the frame (M42 at 250 mm: 12 sky nodes in one corner)
/// left a plane extrapolated from the corner, which took the nebula's wings as sky.
const MIN_SKY_SPREAD: f64 = 0.25;

/// A catalog footprint covering more of the nodes than this is ignored, as are all of
/// them if together they still do: an object larger than the field would leave no sky
/// to fit. Judged per footprint first, or M31 filling a 1 deg frame unprotected M32.
const MAX_PROTECTED_SHARE: f32 = 0.5;

/// Floor on the residual sigma, relative to the sky level, so a noiseless synthetic
/// frame does not reject nodes for float rounding.
const SIGMA_FLOOR: f64 = 1e-4;

/// Where a frame's nodes are: [`node_axis`] on both axes.
#[derive(Debug, Clone)]
pub struct NodeLayout {
    pub xs: Vec<usize>,
    pub ys: Vec<usize>,
    pub width: usize,
    pub height: usize,
}

impl NodeLayout {
    pub fn new(width: usize, height: usize, cols: usize, rows: usize, inset: f32) -> Self {
        Self {
            xs: node_axis(width, cols, inset),
            ys: node_axis(height, rows, inset),
            width,
            height,
        }
    }

    pub fn cols(&self) -> usize {
        self.xs.len()
    }

    pub fn rows(&self) -> usize {
        self.ys.len()
    }

    pub fn nodes(&self) -> Vec<GridNode> {
        layout_nodes(&self.xs, &self.ys)
    }

    /// Node `i` in frame fractions, 0..=1 on both axes.
    fn position(&self, i: usize) -> (f64, f64) {
        let (c, r) = (i % self.cols(), i / self.cols());
        (
            self.xs[c] as f64 / (self.width - 1).max(1) as f64,
            self.ys[r] as f64 / (self.height - 1).max(1) as f64,
        )
    }

    fn border_distance(&self, i: usize) -> usize {
        let (c, r) = (i % self.cols(), i / self.cols());
        c.min(r).min(self.cols() - 1 - c).min(self.rows() - 1 - r)
    }
}

/// The classification and the completed grids.
#[derive(Debug, Clone)]
pub struct Classified {
    /// Node `i` is sky.
    pub sky: Vec<bool>,
    /// Per channel, every node: its own value where sky, the fit plus the survivors'
    /// residual inpainted across it where not. What both models interpolate.
    pub values: Vec<Vec<f32>>,
}

/// Classify `samples[channel][node]` (`None`: no pixels) on `layout`, deciding on
/// `stats_channel` and applying that mask to every channel. `None` when too few nodes
/// survive to fit even a plane.
pub fn classify(
    layout: &NodeLayout,
    samples: &[Vec<Option<f32>>],
    stats_channel: usize,
    protection: Option<&FieldProtection>,
) -> Option<Classified> {
    let n = layout.cols() * layout.rows();
    let pos: Vec<Point> = (0..n).map(|i| Point::new(layout.position(i))).collect();
    let stats: Vec<Option<f64>> = samples[stats_channel].iter().map(|v| v.map(f64::from)).collect();
    let protected = protected_nodes(layout, protection);

    let none = vec![false; n];
    let mut sky = reject_targets(layout, &pos, &stats, &protected, &none)?;
    let glow = edge_peaked(layout, &pos, &stats, &sky, &protected);
    if glow.iter().any(|&g| g) {
        sky = reject_targets(layout, &pos, &stats, &protected, &glow)?;
    }
    dilate(layout, &mut sky, &glow, &protected);
    if sky.iter().filter(|&&s| s).count() < terms(1) + 2 {
        return None;
    }

    let values = samples
        .iter()
        .map(|channel| {
            let v: Vec<Option<f64>> = channel.iter().map(|x| x.map(f64::from)).collect();
            complete(layout, &pos, &v, &sky)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Classified { sky, values })
}

fn protected_nodes(layout: &NodeLayout, protection: Option<&FieldProtection>) -> Vec<bool> {
    let nodes = layout.nodes();
    let n = nodes.len();
    let Some(protection) = protection else {
        return vec![false; n];
    };
    let share = |mask: &[bool]| mask.iter().filter(|&&m| m).count() as f32 / n.max(1) as f32;
    let mut mask = vec![false; n];
    for region in &protection.regions {
        let inside: Vec<bool> = nodes.iter().map(|node| region.contains(node.x as f32, node.y as f32)).collect();
        if share(&inside) > MAX_PROTECTED_SHARE {
            tracing::debug!(share = share(&inside), "A catalog footprint covers most of the field; ignored");
            continue;
        }
        mask.iter_mut().zip(inside).for_each(|(m, i)| *m |= i);
    }
    if share(&mask) > MAX_PROTECTED_SHARE {
        tracing::debug!(share = share(&mask), "Catalog protection covers most of the field; ignored");
        return vec![false; n];
    }
    mask
}

/// [`robust_mask`], then each target grown through its wing ([`K_GROW`]).
fn reject_targets(
    layout: &NodeLayout,
    pos: &[Point],
    v: &[Option<f64>],
    protected: &[bool],
    forced_in: &[bool],
) -> Option<Vec<bool>> {
    let mut sky = robust_mask(pos, v, protected, forced_in)?;
    let level = sky_level(v);
    let seeds: Vec<bool> = sky.iter().zip(forced_in).map(|(&s, &f)| !s && !f).collect();
    let reach = lines_from(layout, &seeds);
    for _ in 0..GROW_ROUNDS {
        let on: Vec<usize> = (0..v.len()).filter(|&i| sky[i]).collect();
        let Some(fit) = Poly::best(pos, v, &on) else { break };
        let residuals: Vec<f64> = on.iter().map(|&i| v[i].unwrap() - fit.at(&pos[i])).collect();
        let sigma = robust_sigma(&residuals).max(SIGMA_FLOOR * level).max(f64::MIN_POSITIVE);
        let residual = |i: usize| v[i].map(|x| x - fit.at(&pos[i]));
        let local = |i: usize| {
            let around: Vec<f64> = std::iter::once(i)
                .chain(neighbours(layout, i))
                .filter(|&j| sky[j])
                .filter_map(residual)
                .collect();
            around.iter().sum::<f64>() / around.len().max(1) as f64
        };
        let wing = |i: usize| {
            sky[i] && !forced_in[i] && reach[i] <= GROW_LINES && v[i].is_some() && local(i) > K_GROW * sigma
        };
        let mut grown = sky.clone();
        let mut frontier: Vec<usize> = (0..v.len()).filter(|&i| !sky[i] && !forced_in[i]).collect();
        while let Some(i) = frontier.pop() {
            for j in neighbours(layout, i).collect::<Vec<_>>() {
                if grown[j] && wing(j) {
                    grown[j] = false;
                    frontier.push(j);
                }
            }
        }
        if grown == sky || grown.iter().filter(|&&s| s).count() < terms(1) + 2 {
            break;
        }
        sky = grown;
    }
    Some(sky)
}

/// Chebyshev distance, in node lines, from every node to the nearest `from` node.
fn lines_from(layout: &NodeLayout, from: &[bool]) -> Vec<usize> {
    let mut distance: Vec<usize> = from.iter().map(|&f| if f { 0 } else { usize::MAX }).collect();
    let mut frontier: Vec<usize> = (0..from.len()).filter(|&i| from[i]).collect();
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for i in frontier {
            for j in neighbours(layout, i).collect::<Vec<_>>() {
                if distance[j] == usize::MAX {
                    distance[j] = distance[i] + 1;
                    next.push(j);
                }
            }
        }
        frontier = next;
    }
    distance
}

fn sky_level(v: &[Option<f64>]) -> f64 {
    let mut all: Vec<f32> = v.iter().flatten().map(|&x| x as f32).collect();
    median(&mut all).abs() as f64
}

/// Sky nodes by the degree ladder: per degree, fit the current sky and re-decide every
/// node against it, so a node rejected by a plane comes back once a curve explains it.
/// Starts from the darker half, since a target only ever adds light: a first fit over
/// every node is lifted by the target and can put the whole sky below its own cut.
/// `forced_in` (glow) always counts as sky. A `protected` node (inside a catalog
/// footprint) is target from [`K_GROW`] up rather than [`K_ABOVE`]: the catalog says an
/// object is there, but a footprint is a generous ellipse (a circle where OpenNGC gives
/// no angle), and carving all of it out left a fill extrapolated across plain sky.
fn robust_mask(
    pos: &[Point],
    v: &[Option<f64>],
    protected: &[bool],
    forced_in: &[bool],
) -> Option<Vec<bool>> {
    let n = v.len();
    let level = sky_level(v);
    let mut values: Vec<f32> = v.iter().flatten().map(|&x| x as f32).collect();
    let half = median(&mut values) as f64;
    let mut sky: Vec<bool> = (0..n)
        .map(|i| forced_in[i] || v[i].is_some_and(|x| x <= half))
        .collect();
    let mut previous_rms = f64::MAX;
    for degree in 1..=MAX_DEGREE {
        let mut trial = sky.clone();
        let mut fit = None;
        for _ in 0..ROUNDS {
            let on: Vec<usize> = (0..n).filter(|&i| trial[i]).collect();
            let Some(poly) = Poly::fit(pos, v, &on, degree) else { break };
            let residuals: Vec<f64> = on.iter().map(|&i| v[i].unwrap() - poly.at(&pos[i])).collect();
            let sigma = robust_sigma(&residuals).max(SIGMA_FLOOR * level).max(f64::MIN_POSITIVE);
            let next: Vec<bool> = (0..n)
                .map(|i| {
                    let above = if protected[i] { K_GROW } else { K_ABOVE };
                    forced_in[i]
                        || v[i].is_some_and(|x| {
                            let d = x - poly.at(&pos[i]);
                            d < above * sigma && d > -K_BELOW * sigma
                        })
                })
                .collect();
            fit = Some(poly);
            if next == trial || next.iter().filter(|&&s| s).count() < terms(degree) + 2 {
                break;
            }
            trial = next;
        }
        let Some(poly) = fit else { break };
        let on: Vec<usize> = (0..n).filter(|&i| trial[i]).collect();
        let rms = poly.rms(pos, v, &on);
        if degree > 1 && rms > DEGREE_GAIN * previous_rms {
            break;
        }
        previous_rms = rms;
        sky = trial;
    }
    (previous_rms < f64::MAX).then_some(sky)
}

/// Rejected nodes (protected aside) whose climb up the residual over the sky's fit ends
/// within [`EDGE_NODES`] of the border. Per node, not per component: a strong glow and a
/// target join into one rejected patch, and its single peak would decide for both. The
/// climb runs on the 3x3 mean residual so node noise does not stop it on a glow's flank.
fn edge_peaked(
    layout: &NodeLayout,
    pos: &[Point],
    v: &[Option<f64>],
    sky: &[bool],
    protected: &[bool],
) -> Vec<bool> {
    let n = sky.len();
    let on: Vec<usize> = (0..n).filter(|&i| sky[i]).collect();
    let Some(fit) = Poly::best(pos, v, &on) else {
        return vec![false; n];
    };
    let candidate: Vec<bool> = (0..n).map(|i| !sky[i] && !protected[i] && v[i].is_some()).collect();
    let residual: Vec<f64> = (0..n).map(|i| v[i].map_or(0.0, |x| x - fit.at(&pos[i]))).collect();
    let smoothed: Vec<f64> = (0..n)
        .map(|i| {
            let around: Vec<f64> = std::iter::once(i)
                .chain(neighbours(layout, i))
                .filter(|&j| candidate[j])
                .map(|j| residual[j])
                .collect();
            around.iter().sum::<f64>() / around.len().max(1) as f64
        })
        .collect();
    (0..n)
        .map(|start| {
            if !candidate[start] {
                return false;
            }
            let mut at = start;
            loop {
                let up = neighbours(layout, at)
                    .filter(|&j| candidate[j])
                    .max_by(|&a, &b| smoothed[a].total_cmp(&smoothed[b]));
                match up {
                    Some(j) if smoothed[j] > smoothed[at] => at = j,
                    _ => break,
                }
            }
            layout.border_distance(at) <= EDGE_NODES
        })
        .collect()
}

fn neighbours(layout: &NodeLayout, i: usize) -> impl Iterator<Item = usize> + '_ {
    let (cols, rows) = (layout.cols() as i64, layout.rows() as i64);
    let (c, r) = ((i % layout.cols()) as i64, (i / layout.cols()) as i64);
    (-1..=1)
        .flat_map(move |dr| (-1..=1).map(move |dc| (r + dr, c + dc)))
        .filter(move |&(nr, nc)| (nr, nc) != (r, c) && nr >= 0 && nc >= 0 && nr < rows && nc < cols)
        .map(move |(nr, nc)| (nr * cols + nc) as usize)
}

/// Grow each target the classifier found by its own size (see [`DILATE_MIN`]). Catalog
/// footprints carry their own margin and are not grown: grown, IC 63's left the fit a
/// third of the sky to extrapolate across the rest.
fn dilate(layout: &NodeLayout, sky: &mut [bool], glow: &[bool], protected: &[bool]) {
    let n = sky.len();
    let target: Vec<bool> = (0..n).map(|i| !sky[i] && !glow[i] && !protected[i]).collect();
    let mut grow = vec![false; n];
    let mut seen = vec![false; n];
    for start in 0..n {
        if seen[start] || !target[start] {
            continue;
        }
        seen[start] = true;
        let mut component = vec![start];
        let mut next = 0;
        while next < component.len() {
            for j in neighbours(layout, component[next]).collect::<Vec<_>>() {
                if !seen[j] && target[j] {
                    seen[j] = true;
                    component.push(j);
                }
            }
            next += 1;
        }
        let radius = (component.len() as f64 / std::f64::consts::PI).sqrt().round() as usize;
        let lines = radius.clamp(DILATE_MIN, DILATE_MAX);
        let mut members = vec![false; n];
        for &i in &component {
            members[i] = true;
        }
        for (i, d) in lines_from(layout, &members).into_iter().enumerate() {
            grow[i] |= d <= lines;
        }
    }
    for i in 0..n {
        if grow[i] && !glow[i] {
            sky[i] = false;
        }
    }
}

/// One channel's grid: sky nodes keep their value; the rest take the sky's fit plus the
/// sky's residual inpainted across them, so the fill carries both the trend and the
/// local level around the hole. Two bounds, both because a target only adds light: the
/// inpainted residual never lifts a hole (at its rim it is the target's own wing — a
/// globular's model took 27 % of the glow at 128-256 px, 2 % bounded), and no fill
/// exceeds the node's own measurement, since a background above it would subtract more
/// than the node holds (a fit extrapolated into a large hole at the edge does overshoot).
fn complete(layout: &NodeLayout, pos: &[Point], v: &[Option<f64>], sky: &[bool]) -> Option<Vec<f32>> {
    let on: Vec<usize> = (0..v.len()).filter(|&i| sky[i] && v[i].is_some()).collect();
    let fit = if spread(pos, &on) < MIN_SKY_SPREAD {
        Poly::fit(pos, v, &on, 0)?
    } else {
        without_overshoot(pos, v, &on, sky, Poly::best(pos, v, &on)?)
    };
    let mut residual: Vec<Option<f32>> = (0..v.len())
        .map(|i| (sky[i] && v[i].is_some()).then(|| (v[i].unwrap() - fit.at(&pos[i])) as f32))
        .collect();
    inpaint(&mut residual, layout.rows(), layout.cols());
    Some(
        (0..v.len())
            .map(|i| match (sky[i], v[i]) {
                (true, Some(x)) => x as f32,
                (false, Some(x)) => (fit.at(&pos[i]) as f32 + residual[i].unwrap_or(0.0).min(0.0)).min(x as f32),
                _ => fit.at(&pos[i]) as f32 + residual[i].unwrap_or(0.0).min(0.0),
            })
            .collect(),
    )
}

/// Share of the frame the sky nodes' bounding box covers.
fn spread(pos: &[Point], on: &[usize]) -> f64 {
    let (mut x0, mut y0, mut x1, mut y1) = (1.0f64, 1.0f64, 0.0f64, 0.0f64);
    for &i in on {
        let Point { x, y, .. } = pos[i];
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    ((x1 - x0) * (y1 - y0)).max(0.0)
}

/// `fit`, or the highest lower degree whose fill does not rise above the data it
/// replaces: a background above a target's own measurement is the fit swinging through
/// a hole it has no samples in (M27 filling a 1024 px crop rendered black under the
/// quartic, whose fill the node bound then pinned to the data). Never below a plane.
fn without_overshoot(pos: &[Point], v: &[Option<f64>], on: &[usize], sky: &[bool], fit: Poly) -> Poly {
    let residuals: Vec<f64> = on.iter().map(|&i| v[i].unwrap() - fit.at(&pos[i])).collect();
    let tolerance = K_ABOVE * robust_sigma(&residuals);
    let holes: Vec<usize> = (0..v.len()).filter(|&i| !sky[i] && v[i].is_some()).collect();
    let overshoots = |poly: &Poly| holes.iter().any(|&i| poly.at(&pos[i]) - v[i].unwrap() > tolerance);
    let mut fit = fit;
    while fit.degree > 1 && overshoots(&fit) {
        match Poly::fit(pos, v, on, fit.degree - 1) {
            Some(lower) => fit = lower,
            None => break,
        }
    }
    fit
}

/// Fill `None` by repeated 4-neighbour averaging. Sequential: a few hundred nodes.
pub(crate) fn inpaint(grid: &mut [Option<f32>], rows: usize, cols: usize) {
    loop {
        let mut fills = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                if grid[r * cols + c].is_some() {
                    continue;
                }
                let around = [
                    (r > 0).then(|| grid[(r - 1) * cols + c]).flatten(),
                    (r + 1 < rows).then(|| grid[(r + 1) * cols + c]).flatten(),
                    (c > 0).then(|| grid[r * cols + c - 1]).flatten(),
                    (c + 1 < cols).then(|| grid[r * cols + c + 1]).flatten(),
                ];
                let known: Vec<f32> = around.into_iter().flatten().collect();
                if !known.is_empty() {
                    fills.push((r * cols + c, known.iter().sum::<f32>() / known.len() as f32));
                }
            }
        }
        if fills.is_empty() {
            return;
        }
        for (i, v) in fills {
            grid[i] = Some(v);
        }
    }
}

#[cfg(test)]
mod tests {
    include!("classify_tests.rs");
}
