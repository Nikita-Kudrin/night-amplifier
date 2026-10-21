//! A deep-sky stack's first subs, held until the session has enough of itself to judge
//! them by.
//!
//! Every safeguard judges a sub against the session so far, so the first subs pass on
//! trust and sub 1 becomes the reference unjudged: a bumped first sub left every star of
//! a stack ~40 % larger, trails in subs 1-3 let 2 of 26 M33 subs stack, and Fireworks'
//! smeared sub 1 (too wide to measure) could never be re-based away from. The held subs
//! stream into the stack as before; once [`PROBATION_SUBS`] are in, the sharpest clean
//! one is elected reference, every held sub is judged by the gate's own rules against
//! the batch, and the stack is rebuilt from the survivors in one pass, where the
//! rejection plugin sees each pixel's held samples side by side (`MasterStack::restack`).
//!
//! Costs a half-precision copy of each held sub (54 MB at 3008²x3, 324 MB for six,
//! released at the commit; fewer where memory is short, [`affordable`]) and one warp
//! and blend per survivor, once.

use night_amplifier_core::detection::{doubled_star_share, Star};
use night_amplifier_core::frame::Frame;
use night_amplifier_core::registration::{AdaptiveRegistration, AdaptiveRegistrationResult as Fit, AffineTransform};
use rayon::prelude::*;
use tracing::info;
use night_amplifier_core::stacking::{FrameQuality, HeldFrame};

use crate::capture::frame_gate::{FrameGate, RejectionReason, StarField, DOUBLED_MARGIN, WARMUP_FRAMES};

/// Subs held before the stack commits: the reference plus the warm-up the gate needs to
/// judge by its medians, so the batch is judged as a warmed-up gate judges later subs.
pub const PROBATION_SUBS: usize = WARMUP_FRAMES + 1;

/// Share of the best-pairing sub's stars a held sub must pair on the reference to stay.
/// Clean subs pair alike (Fireworks 160-180, M27 140-156), clouded and bumped ones far
/// fewer. Stricter than mid-session, which keeps registered doubled subs: here a sub is
/// a sixth of the stack, and dropping one costs a sixth of a session's first minute.
const AGREEMENT: f32 = 0.5;

/// Share of the best consensus the first sub needs to stay the reference: clean subs
/// pair within a few stars of each other, and swapping among them buys nothing.
const INCUMBENT_SHARE: f32 = 0.9;

/// Offered subs, held or not, after which a probation commits on what it holds: under
/// cloud most subs have no stars to hold, and the stack must not wait for clear sky.
const PATIENCE_FACTOR: usize = 2;

/// Divisor of the memory available at a stack's start that its held subs and the commit's
/// f32 scratch frame may take: the render task, encoders and disk queue share the rest,
/// whose frame queues alone may take a fifth of RAM (`frame_queue_budget_bytes`).
const MEMORY_SHARE: usize = 2;

/// Subs a probation of `limit` can afford for a stack of `samples`, out of `available`
/// bytes (`None`, unknown: `limit`). Six subs of a 26 MP colour sensor are 940 MB beside
/// its 1.25 GB accumulator, more than a 4 GB board has left; below two, the stack streams.
pub(super) fn affordable(limit: usize, samples: usize, available: Option<usize>) -> usize {
    let Some(available) = available else {
        return limit;
    };
    let scratch = samples * std::mem::size_of::<f32>();
    let budget = (available / MEMORY_SHARE).saturating_sub(scratch);
    limit.min(budget / (samples * HeldFrame::BYTES_PER_SAMPLE).max(1))
}

/// `MemAvailable`, or `None` where `/proc/meminfo` cannot say.
fn available_memory() -> Option<usize> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    crate::capture::channel::parse_meminfo_bytes(&meminfo, "MemAvailable")
}

pub(super) struct HeldSub {
    frame: HeldFrame,
    pub(super) stars: Vec<Star>,
    pub(super) field: StarField,
    pub(super) quality: FrameQuality,
    /// Whether the sub is in the stack as it streamed.
    pub(super) stacked: bool,
}

pub(super) struct Probation {
    limit: usize,
    held: Vec<HeldSub>,
    offered: usize,
}

/// What a probation decided: the reference, and every held sub's fate against it.
pub(super) struct Commit {
    /// Index of the elected reference among the held subs.
    pub(super) reference: usize,
    /// Per held sub: its transform onto the reference, `Some` exactly for subs kept.
    pub(super) placed: Vec<Option<AffineTransform>>,
    /// Per held sub: why it is out, `None` for subs kept.
    pub(super) reasons: Vec<Option<RejectionReason>>,
    /// The gate as the batch left it: its history is the held subs' measurements.
    pub(super) gate: FrameGate,
}

impl Probation {
    /// `None` below two subs: one sub is no batch, and the stack streams as it always did.
    pub(super) fn new(limit: usize) -> Option<Self> {
        (limit >= 2).then(|| Self { limit, held: Vec::with_capacity(limit), offered: 0 })
    }

    /// [`Self::new`] for a stack of `samples`, held to what memory affords right now.
    pub(super) fn for_stack(limit: usize, samples: usize) -> Option<Self> {
        let subs = affordable(limit, samples, available_memory());
        if subs < limit {
            info!(asked = limit, held = subs, "Probation held to what memory affords");
        }
        Self::new(subs)
    }

    pub(super) fn offered(&mut self) {
        self.offered += 1;
    }

    pub(super) fn hold(&mut self, frame: &Frame, stars: Vec<Star>, quality: FrameQuality, stacked: bool) {
        let field = StarField::of(&stars);
        self.held.push(HeldSub { frame: HeldFrame::of(frame), stars, field, quality, stacked });
    }

    pub(super) fn is_due(&self) -> bool {
        self.held.len() >= self.limit || self.offered >= PATIENCE_FACTOR * self.limit
    }

    pub(super) fn held(&self) -> &[HeldSub] {
        &self.held
    }

    /// The kept subs in stacking order (the reference first, then as they arrived), with
    /// their transforms and qualities, ready for `Stacker::restack`.
    pub(super) fn into_kept(self, commit: &Commit) -> (Vec<HeldFrame>, Vec<AffineTransform>, Vec<FrameQuality>) {
        let mut order: Vec<usize> = (0..self.held.len()).filter(|&i| commit.placed[i].is_some()).collect();
        order.sort_by_key(|&i| i != commit.reference);
        let transforms = order.iter().map(|&i| commit.placed[i].expect("kept subs are placed")).collect();
        let qualities = order.iter().map(|&i| self.held[i].quality).collect();
        let mut held: Vec<Option<HeldSub>> = self.held.into_iter().map(Some).collect();
        let frames = order.iter().map(|&i| held[i].take().expect("each sub once").frame).collect();
        (frames, transforms, qualities)
    }

    /// Elects the reference and judges every held sub against it. `None` when no
    /// candidate registers with most of the batch: the stack then stays as it streamed.
    ///
    /// Elected by consensus — the median count of stars the others line up on it within
    /// 1.5 px — not by FWHM: a reference serves registration, and the area estimator reads
    /// a smeared sub *sharper* (Fireworks: 2.5 px against 5 px clean), while a bumped,
    /// trailed or smeared sub pairs 7-120 stars where its clean neighbours pair 150-180.
    pub(super) fn decide(&self, registration: &AdaptiveRegistration, frames_seen: usize) -> Option<Commit> {
        let needed = (self.held.len() - 1).div_ceil(2);
        let trials: Vec<(usize, Vec<Option<Fit>>)> = self
            .candidates()
            .into_par_iter()
            .map(|candidate| (candidate, self.register_against(registration, candidate)))
            .collect();
        let mut trials: Vec<(usize, Vec<Option<Fit>>)> = trials
            .into_iter()
            .filter(|(_, fits)| fits.iter().flatten().filter(|fit| fit.is_credible()).count() >= needed)
            .collect();
        let best = trials.iter().map(|(_, fits)| consensus(fits)).max()?;
        // The sub the stack streamed on keeps the job unless another is clearly better:
        // moving the reference moves every pixel, and Pro's field solve starts over.
        let incumbent = trials
            .iter()
            .position(|(i, fits)| *i == 0 && consensus(fits) as f32 >= INCUMBENT_SHARE * best as f32);
        let chosen = incumbent.unwrap_or_else(|| {
            let width = |i: usize| self.held[i].field.fwhm.unwrap_or(f32::INFINITY);
            (0..trials.len())
                .filter(|&t| consensus(&trials[t].1) == best)
                .min_by(|&a, &b| width(trials[a].0).total_cmp(&width(trials[b].0)))
                .expect("the best is one of them")
        });
        let (reference, fits) = trials.swap_remove(chosen);
        Some(self.judge(reference, fits, frames_seen))
    }

    /// Subs worth trying as the reference: not doubled against the batch (a bump's
    /// companions pull its centroids), or every sub if all of them are.
    fn candidates(&self) -> Vec<usize> {
        let doubling = lower_quartile(self.held.iter().map(|sub| sub.field.doubling).collect());
        let single = |sub: &HeldSub| sub.field.doubling < doubling + DOUBLED_MARGIN * (1.0 - doubling);
        let order: Vec<usize> = (0..self.held.len()).filter(|&i| single(&self.held[i])).collect();
        if order.is_empty() {
            return (0..self.held.len()).collect();
        }
        order
    }

    fn register_against(&self, registration: &AdaptiveRegistration, reference: usize) -> Vec<Option<Fit>> {
        let stars = &self.held[reference].stars;
        self.held
            .iter()
            .enumerate()
            .map(|(i, sub)| (i != reference).then(|| registration.register(stars, &sub.stars).ok()).flatten())
            .collect()
    }

    /// The gate's verdicts on the batch: a sub pairing under [`AGREEMENT`] of the
    /// best-pairing sub's stars is out, and the rest are judged by the gate's own rules
    /// against a yardstick the agreeing subs set. The whole batch's medians fail once half
    /// of it is bad — three clouded subs of six pulled the median FWHM down until the
    /// three clear ones read as bloated and were dropped instead.
    fn judge(&self, reference: usize, fits: Vec<Option<Fit>>, frames_seen: usize) -> Commit {
        let mut gate = FrameGate::restarted(frames_seen);
        let elected = &self.held[reference];
        gate.set_reference(elected.field.fwhm, doubled_star_share(&elected.stars));
        let best = fits.iter().flatten().filter(|fit| fit.is_credible()).map(|fit| fit.support.pairs).max().unwrap_or(0);
        let agrees = |fit: &Fit| fit.is_credible() && fit.support.pairs as f32 >= AGREEMENT * best as f32;
        for (sub, fit) in self.held.iter().zip(&fits) {
            if let Some(fit) = fit.as_ref().filter(|fit| agrees(fit)) {
                gate.record(fit, sub.field);
            }
        }

        let mut placed = vec![None; self.held.len()];
        let mut reasons = vec![None; self.held.len()];
        placed[reference] = Some(AffineTransform::identity());
        for (i, (sub, fit)) in self.held.iter().zip(fits).enumerate() {
            if i == reference {
                continue;
            }
            let verdict = match &fit {
                None => Some(gate.explain_unregistered(sub.field, || false)),
                Some(fit) if !agrees(fit) => Some(RejectionReason::Outvoted),
                Some(fit) => gate.verdict(fit, sub.field.fwhm),
            };
            match (verdict, fit) {
                (None, Some(fit)) => placed[i] = Some(fit.transform),
                (reason, _) => reasons[i] = reason.or(Some(RejectionReason::RegistrationFailed)),
            }
        }
        Commit { reference, placed, reasons, gate }
    }
}

/// Median stars the batch lines up on a candidate; a fit chance explains counts none.
fn consensus(fits: &[Option<Fit>]) -> usize {
    let mut pairs: Vec<usize> = fits
        .iter()
        .flatten()
        .map(|fit| if fit.is_credible() { fit.support.pairs } else { 0 })
        .collect();
    // Subs that did not register at all count as none, too.
    pairs.resize(fits.len().saturating_sub(1).max(pairs.len()), 0);
    pairs.sort_unstable();
    pairs.get(pairs.len() / 2).copied().unwrap_or(0)
}

fn lower_quartile(mut values: Vec<f32>) -> f32 {
    quantile(&mut values, 4)
}

fn quantile(values: &mut [f32], divisor: usize) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / divisor]
}

#[cfg(test)]
#[path = "probation_tests.rs"]
mod tests;
