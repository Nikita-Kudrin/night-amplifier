//! The render has to hold still when the sky does.
//!
//! A static synthetic sky — gradient, colour cast, stars and a scatter of faint flat
//! patches — goes frame after frame through the production preview path and encoder,
//! with one [`PreviewAnalysis`] held across frames as the render task holds it. The
//! patches sit a few black-point gaps above the sky, where the tone curve's response to
//! the gap is steepest: that is where a stale noise figure shows, and where it showed —
//! the render sagged between cache refreshes and jumped at each.

use super::{AnalysisContext, PreviewAnalysis};
use crate::frame::Frame;
use crate::server::capture::pipeline::process_preview_frame_with_analysis;
use crate::server::encoding::frame_to_rgb8_downsampled;
use crate::server::state::{CaptureSettings, RenderReadyFrame};

const SIZE: usize = 256;
/// One sub's noise, in linear units.
const SIGMA: f32 = 0.003;
/// How far the patches sit above the sky: about one black-point gap at depth 8 and four
/// by depth 100, so they cross the curve's steep part as the stack deepens.
const PATCH: f32 = 0.003;
const PATCH_SIDE: usize = 8;

/// What the observer sees of one frame, in output levels.
#[derive(Debug, Clone, Copy)]
struct Seen {
    sky: [f64; 3],
    patches: f64,
}

struct Scene {
    /// The noiseless sky at unit exposure, planar RGB.
    signal: Vec<f32>,
    sky: Vec<usize>,
    patches: Vec<usize>,
}

impl Scene {
    fn new() -> Self {
        let plane = SIZE * SIZE;
        let cast = [1.2f32, 1.0, 0.85];
        let mut signal = vec![0.0f32; plane * 3];
        let mut in_patch = vec![false; plane];
        let mut near_star = vec![false; plane];

        for y in 0..SIZE {
            for x in 0..SIZE {
                let sky = 0.02 + 0.006 * x as f32 / SIZE as f32 + 0.003 * y as f32 / SIZE as f32;
                for c in 0..3 {
                    signal[c * plane + y * SIZE + x] = sky * cast[c];
                }
            }
        }
        // A 6x6 lattice of patches, off the background model's nodes.
        for gy in 0..6 {
            for gx in 0..6 {
                let (x0, y0) = (22 + gx * 38, 22 + gy * 38);
                for y in y0..y0 + PATCH_SIDE {
                    for x in x0..x0 + PATCH_SIDE {
                        in_patch[y * SIZE + x] = true;
                        for c in 0..3 {
                            signal[c * plane + y * SIZE + x] += PATCH;
                        }
                    }
                }
            }
        }
        // Stars on their own lattice, between the patches.
        for sy in (41..SIZE - 8).step_by(38) {
            for sx in (41..SIZE - 8).step_by(38) {
                let peak = 0.05 + 0.4 * ((sx * 7 + sy * 13) % 10) as f32 / 10.0;
                for y in sy - 6..=sy + 6 {
                    for x in sx - 6..=sx + 6 {
                        let d2 = ((x as f32 - sx as f32).powi(2) + (y as f32 - sy as f32).powi(2))
                            / (2.0 * 1.5f32.powi(2));
                        near_star[y * SIZE + x] = true;
                        for c in 0..3 {
                            signal[c * plane + y * SIZE + x] += peak * (-d2).exp();
                        }
                    }
                }
            }
        }

        let margin = 12;
        let mut sky = Vec::new();
        let mut patches = Vec::new();
        for y in margin..SIZE - margin {
            for x in margin..SIZE - margin {
                let i = y * SIZE + x;
                if in_patch[i] {
                    patches.push(i);
                } else if !near_star[i] {
                    sky.push(i);
                }
            }
        }
        Self { signal, sky, patches }
    }

    /// One sub at `exposure`: the signal scaled, and noise that scales as shot noise does.
    fn sub(&self, exposure: f32, rng: &mut Rng) -> Vec<f32> {
        let sigma = SIGMA * exposure.sqrt();
        self.signal
            .iter()
            .map(|&s| (s * exposure + sigma * rng.gauss()).clamp(0.0, 1.0))
            .collect()
    }

    fn see(&self, rgb8: &[u8]) -> Seen {
        let mean = |idx: &[usize], c: usize| {
            idx.iter().map(|&i| rgb8[i * 3 + c] as f64).sum::<f64>() / idx.len() as f64
        };
        Seen {
            sky: [mean(&self.sky, 0), mean(&self.sky, 1), mean(&self.sky, 2)],
            patches: mean(&self.patches, 1),
        }
    }
}

/// Deterministic noise: xorshift, and a Gaussian from four uniforms — cheap enough for a
/// debug build, and close enough to normal for a MAD.
struct Rng(u64);

impl Rng {
    fn uniform(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn gauss(&mut self) -> f32 {
        (self.uniform() + self.uniform() + self.uniform() + self.uniform() - 2.0) * 3.0f32.sqrt()
    }
}

/// Render one frame the way the render task does, with `analysis` held across frames.
fn render(
    data: Vec<f32>,
    ctx: AnalysisContext,
    analysis: &mut PreviewAnalysis,
    settings: &CaptureSettings,
) -> Vec<u8> {
    let mut frame = Frame::from_f32_vec(data, SIZE, SIZE, 3).unwrap();
    let rendered =
        process_preview_frame_with_analysis(&mut frame, settings, ctx, analysis).unwrap();
    let ready = RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config: rendered.pipeline_config,
        stretch_result: rendered.stretch_result,
        noise: None,
    };
    let (rgb8, w, h) = frame_to_rgb8_downsampled(&ready, SIZE as u32, SIZE as u32).unwrap();
    assert_eq!((w as usize, h as usize), (SIZE, SIZE));
    rgb8
}

/// Live view: every frame a new sub, `exposure(i)` for the `i`-th.
fn live_view(frames: usize, exposure: impl Fn(usize) -> f32) -> Vec<Seen> {
    let scene = Scene::new();
    let settings = CaptureSettings::default();
    let mut analysis = PreviewAnalysis::new();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    (0..frames)
        .map(|i| {
            let rgb8 = render(
                scene.sub(exposure(i), &mut rng),
                AnalysisContext::ONE_SHOT,
                &mut analysis,
                &settings,
            );
            scene.see(&rgb8)
        })
        .collect()
}

/// Stacked view: the running mean of `frames` subs, rendered at every depth twice —
/// through the analysis held across frames, and through a fresh one.
fn stacked_view(frames: usize) -> Vec<(Seen, Seen)> {
    let scene = Scene::new();
    let settings = CaptureSettings::default();
    let mut analysis = PreviewAnalysis::new();
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut sum = vec![0.0f64; scene.signal.len()];
    (1..=frames)
        .map(|depth| {
            for (s, v) in sum.iter_mut().zip(scene.sub(1.0, &mut rng)) {
                *s += v as f64;
            }
            let mean: Vec<f32> = sum.iter().map(|s| (s / depth as f64) as f32).collect();
            let ctx = AnalysisContext {
                showing_stack: true,
                stack_depth: depth as u32,
            };
            let held = scene.see(&render(mean.clone(), ctx, &mut analysis, &settings));
            let fresh = scene.see(&render(mean, ctx, &mut PreviewAnalysis::new(), &settings));
            (held, fresh)
        })
        .collect()
}

/// The plan's bar: one output level, frame to frame, for a sky that is not moving.
const ONE_LEVEL: f64 = 1.0;

fn assert_sky_holds(seen: &[Seen], from: usize, what: &str) {
    for (i, pair) in seen.windows(2).enumerate().skip(from) {
        for c in 0..3 {
            let step = pair[1].sky[c] - pair[0].sky[c];
            assert!(
                step.abs() < ONE_LEVEL,
                "{what}: channel {c}'s sky moved {step:+.2} output levels at frame {} \
                 ({:.2} -> {:.2})",
                i + 1,
                pair[0].sky[c],
                pair[1].sky[c]
            );
        }
    }
}

/// The black point and white balance are re-solved for every sub in live view, and that
/// is what keeps the sky still: the solve pins it to the target background, so what
/// remains is the estimators' own jitter. Holding either one steady instead would let
/// the sky ride the per-frame pedestal and model offsets the black point absorbs.
#[test]
fn a_static_sky_holds_still_in_live_view() {
    let seen = live_view(40, |_| 1.0);
    assert_sky_holds(&seen, 0, "live view");
    for (i, pair) in seen.windows(2).enumerate() {
        let step = pair[1].patches - pair[0].patches;
        // The patches are 2304 pixels of single-sub noise; their mean alone wanders 0.35
        // levels a frame (RMS; 0.8 at worst over these 40), so the bound leaves room for it.
        assert!(
            step.abs() < 1.5,
            "live view: the faint patches moved {step:+.2} output levels at frame {}",
            i + 1
        );
    }
}

/// Reusing the analysis must not change what a deepening stack looks like.
///
/// The cache used to hold the MAD between refreshes while the depth gain advanced every
/// frame, so the faint patches sagged below what a fresh analysis rendered — up to 1.5
/// output levels here, 5 on M27 — and jumped back at each refresh. Carried along the
/// stack's noise trend, the held analysis stays within ~0.3 of a fresh one.
#[test]
fn a_reused_analysis_renders_what_a_fresh_one_would() {
    let views = stacked_view(80);
    let held: Vec<Seen> = views.iter().map(|v| v.0).collect();
    assert_sky_holds(&held, 4, "stacked view");

    for (i, (held, fresh)) in views.iter().enumerate() {
        let gap = held.patches - fresh.patches;
        assert!(
            gap.abs() < 0.6,
            "depth {}: the reused analysis rendered the faint patches at {:.2} output \
             levels, a fresh one at {:.2}",
            i + 1,
            held.patches,
            fresh.patches
        );
    }
}

/// A deliberate change has to be followed at once, not faded into: with the exposure
/// doubled, the very next frame renders the sky where it will stay.
#[test]
fn an_exposure_change_is_followed_at_once() {
    const CHANGE: usize = 20;
    let seen = live_view(40, |i| if i < CHANGE { 1.0 } else { 2.0 });
    let settled = &seen[CHANGE + 5..];
    let settled_mean = |f: &dyn Fn(&Seen) -> f64| {
        settled.iter().map(|s| f(s)).sum::<f64>() / settled.len() as f64
    };
    let first = seen[CHANGE];
    for c in 0..3 {
        let target = settled_mean(&|s: &Seen| s.sky[c]);
        assert!(
            (first.sky[c] - target).abs() < ONE_LEVEL,
            "channel {c}'s sky rendered {:.2} on the first frame at the new exposure and \
             settled at {target:.2}",
            first.sky[c]
        );
    }
    let target = settled_mean(&|s: &Seen| s.patches);
    assert!(
        (first.patches - target).abs() < 1.5,
        "the faint patches rendered {:.2} on the first frame at the new exposure and \
         settled at {target:.2}",
        first.patches
    );
}
