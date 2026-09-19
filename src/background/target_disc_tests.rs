use super::*;
use crate::background::{BackgroundConfig, BackgroundExtractionAlgorithm, BackgroundExtractor};

const SIGMA: f32 = 3.4e-5;
const SKY: f32 = 0.0028;
const SIZE: usize = 1024;

fn lcg(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / (1u32 << 24) as f32
}

/// Flat sky at deep-stack noise plus `extra(x, y)`.
fn noisy_frame(extra: impl Fn(usize, usize) -> f32) -> Frame {
    let mut seed = 0x2545_f491_u32;
    let mut frame = Frame::zeros(SIZE, SIZE, 1).unwrap();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (u1, u2) = (lcg(&mut seed) + 1e-7, lcg(&mut seed));
            let z = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
            frame.set_pixel(x, y, 0, SKY + SIGMA * z + extra(x, y));
        }
    }
    frame
}

/// Faint unresolved stars placed with probability `density(x, y)`.
fn crowded_field(stars_per_pixel: f32, density: impl Fn(f32, f32) -> f32) -> Vec<f32> {
    let mut seed = 0x1234_5678_u32;
    let mut glow = vec![0.0f32; SIZE * SIZE];
    for _ in 0..((SIZE * SIZE) as f32 * stars_per_pixel) as usize {
        let (x, y) = (lcg(&mut seed) * SIZE as f32, lcg(&mut seed) * SIZE as f32);
        if lcg(&mut seed) >= density(x, y) {
            continue;
        }
        for py in (y as usize).saturating_sub(4)..(y as usize + 5).min(SIZE) {
            for px in (x as usize).saturating_sub(4)..(x as usize + 5).min(SIZE) {
                let r2 = (px as f32 + 0.5 - x).powi(2) + (py as f32 + 0.5 - y).powi(2);
                glow[py * SIZE + px] += 1.5 * SIGMA * (-r2 / 4.0).exp();
            }
        }
    }
    glow
}

/// Cells-centred 16x16 grid, as the RBF extractor lays it.
fn disc_in(frame: &Frame) -> Option<TargetDisc> {
    let step = SIZE / 16;
    let nodes: Vec<GridNode> = (0..16)
        .flat_map(|row| (0..16).map(move |col| GridNode::new(col * step + step / 2, row * step + step / 2, col, row)))
        .collect();
    let samples = TargetDisc::sample_nodes(frame, &nodes, crate::background::compute_box_size(SIZE), 0);
    TargetDisc::find(&nodes, &samples, 16, 16, SIZE, SIZE)
}

/// Gradients carry no crowding: a disc would stop the model following them.
#[test]
fn gradients_alone_never_find_a_target() {
    let s = SIZE as f32;
    type Gradient = Box<dyn Fn(usize, usize) -> f32>;
    let cases: [(&str, Gradient); 4] = [
        ("linear", Box::new(move |x, _| 30.0 * SIGMA * x as f32 / s)),
        ("horizon", Box::new(move |_, y| 40.0 * SIGMA * (-(1.0 - y as f32 / s) * 4.0).exp())),
        ("dome", Box::new(move |x, y| {
            -40.0 * SIGMA * ((x as f32 / s - 0.5).powi(2) + (y as f32 / s - 0.5).powi(2))
        })),
        ("corner", Box::new(move |x, y| 25.0 * SIGMA * (-(x as f32).hypot(y as f32) / s * 2.5).exp())),
    ];
    for (name, extra) in cases {
        let disc = disc_in(&noisy_frame(extra));
        assert!(disc.is_none(), "{name}: found a target {disc:?} in a bare gradient");
    }
}

/// A satellite trail is bright and linear, not crowded.
#[test]
fn a_satellite_trail_is_not_a_target() {
    let s = SIZE as f32;
    let frame = noisy_frame(|x, y| {
        let d = (x as f32 - y as f32).abs() / 2f32.sqrt();
        30.0 * SIGMA * x as f32 / s + if d < 1.5 { 30.0 * SIGMA } else { 0.0 }
    });
    assert_eq!(disc_in(&frame), None);
}

/// A Milky Way band crowds half the frame; it must not read as one target.
#[test]
fn a_milky_way_band_is_not_a_target() {
    let s = SIZE as f32;
    let band = crowded_field(0.08, |_, y| if y > s * 0.5 { 1.0 } else { 0.05 });
    let frame = noisy_frame(|x, y| band[y * SIZE + x] + 30.0 * SIGMA * x as f32 / s);
    assert_eq!(disc_in(&frame), None);
}

/// The bilinear extractor had the same halo defect as RBF; it shares the disc.
#[test]
fn the_bilinear_model_keeps_a_crowded_cluster_out() {
    let (centre, core) = (SIZE as f32 / 2.0, SIZE as f32 * 0.06);
    let glow = crowded_field(0.6, |x, y| {
        1.0 / (1.0 + ((x - centre).powi(2) + (y - centre).powi(2)) / (core * core))
    });
    let frame = noisy_frame(|x, y| glow[y * SIZE + x]);
    let config = BackgroundConfig::for_star_field().with_algorithm(BackgroundExtractionAlgorithm::GridBilinear);
    let model = BackgroundExtractor::new(config).estimate(&frame).unwrap();
    let rise = (model.get_background(SIZE / 2, SIZE / 2, 0) - model.get_background(SIZE / 16, SIZE / 16, 0)) / SIGMA;
    assert!(rise < 0.5, "bilinear model rose {rise:.2} sigma under the cluster");
}

/// A noisy `w` x `h` sky plus a round crowded cluster of core radius `core` px at the
/// centre, plus `extra(x, y)`.
fn wide_cluster(w: usize, h: usize, core: f32, extra: impl Fn(usize, usize) -> f32) -> Frame {
    let mut seed = 0x2545_f491_u32;
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let mut glow = vec![0.0f32; w * h];
    for _ in 0..((w * h) as f32 * 0.6) as usize {
        let (x, y) = (lcg(&mut seed) * w as f32, lcg(&mut seed) * h as f32);
        if lcg(&mut seed) >= 1.0 / (1.0 + ((x - cx).powi(2) + (y - cy).powi(2)) / (core * core)) {
            continue;
        }
        for py in (y as usize).saturating_sub(4)..(y as usize + 5).min(h) {
            for px in (x as usize).saturating_sub(4)..(x as usize + 5).min(w) {
                let r2 = (px as f32 + 0.5 - x).powi(2) + (py as f32 + 0.5 - y).powi(2);
                glow[py * w + px] += 1.5 * SIGMA * (-r2 / 4.0).exp();
            }
        }
    }
    let mut frame = Frame::zeros(w, h, 1).unwrap();
    for y in 0..h {
        for x in 0..w {
            let (u1, u2) = (lcg(&mut seed) + 1e-7, lcg(&mut seed));
            let z = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
            frame.set_pixel(x, y, 0, SKY + SIGMA * z + glow[y * w + x] + extra(x, y));
        }
    }
    frame
}

fn bilinear(frame: &Frame) -> crate::background::BackgroundModel {
    let config = BackgroundConfig::for_star_field().with_algorithm(BackgroundExtractionAlgorithm::GridBilinear);
    BackgroundExtractor::new(config).estimate(frame).unwrap()
}

/// Uncalibrated vignetting (flats are not wired in) under a centred globular. With a
/// plane the dome read as ring excess, the disc ran to `MAX_RADIUS` and the refill
/// followed 2.78 of a 7.66 sigma rise: a bright disc left in the image.
#[test]
fn vignetting_under_a_centred_cluster_is_still_modelled() {
    let s = SIZE as f32;
    // 10 sigma brighter at the centre than the corners: ~12 % of the sky.
    let dome = move |x: usize, y: usize| {
        10.0 * SIGMA * (1.0 - 2.0 * ((x as f32 / s - 0.5).powi(2) + (y as f32 / s - 0.5).powi(2)))
    };
    let frame = wide_cluster(SIZE, SIZE, SIZE as f32 * 0.06, dome);
    let model = bilinear(&frame);
    let (c, corner) = (SIZE / 2, SIZE / 16);
    let true_rise = (dome(c, c) - dome(corner, corner)) / SIGMA;
    let model_rise = (model.get_background(c, c, 0) - model.get_background(corner, corner, 0)) / SIGMA;
    let disc = disc_in(&frame);
    println!("disc {disc:?}: vignetting rise {true_rise:.2} sigma, modelled {model_rise:.2}");
    assert!(disc.is_some_and(|d| d.curved), "vignetting did not select the quadratic: {disc:?}");
    // Quadratic: 8.82 modelled, the excess over 7.66 being halo the smaller disc leaves.
    assert!(
        (true_rise - model_rise).abs() < 1.5,
        "the model follows {model_rise:.2} of a {true_rise:.2} sigma vignetting rise under the cluster"
    );
}

/// The disc lives in normalised units, an ellipse 1.78x wider than tall on a 16:9
/// sensor; a round halo must still stay out above and below the cluster.
#[test]
fn a_round_cluster_on_a_wide_sensor_is_kept_out_on_both_axes() {
    let (w, h) = (1536usize, 864usize);
    let frame = wide_cluster(w, h, h as f32 * 0.06, |_, _| 0.0);
    let model = bilinear(&frame);
    let far = model.get_background(w / 32, h / 32, 0);
    let offset = h * 3 / 10;
    let rise = |x: usize, y: usize| (model.get_background(x, y, 0) - far) / SIGMA;
    let (horizontal, vertical) = (rise(w / 2 + offset, h / 2), rise(w / 2, h / 2 + offset));
    let centre = rise(w / 2, h / 2);
    println!("rise at centre {centre:.2}, {offset} px right {horizontal:.2}, {offset} px down {vertical:.2} sigma");
    assert!(centre < 0.5, "the model rose {centre:.2} sigma under the cluster");
    assert!(
        (vertical - horizontal).abs() < 0.5,
        "at {offset} px from the cluster the model rose {vertical:.2} sigma vertically and {horizontal:.2} horizontally"
    );
}

/// Two clusters in one field (h and chi Persei): the disc centres between them and
/// must still cover both.
#[test]
fn two_clusters_in_one_field_are_both_kept_out() {
    let core = SIZE as f32 * 0.05;
    let centres = [(SIZE as f32 * 0.3, SIZE as f32 * 0.45), (SIZE as f32 * 0.7, SIZE as f32 * 0.55)];
    let glow = crowded_field(0.6, |x, y| {
        centres
            .iter()
            .map(|&(cx, cy)| 1.0 / (1.0 + ((x - cx).powi(2) + (y - cy).powi(2)) / (core * core)))
            .fold(0.0, f32::max)
    });
    let frame = noisy_frame(|x, y| glow[y * SIZE + x]);
    let model = bilinear(&frame);
    let far = model.get_background(SIZE / 16, SIZE * 15 / 16, 0);
    let rises: Vec<f32> = centres
        .iter()
        .map(|&(cx, cy)| (model.get_background(cx as usize, cy as usize, 0) - far) / SIGMA)
        .collect();
    let disc = disc_in(&frame);
    println!("disc {disc:?}: model rise under the clusters {rises:?} sigma");
    // Flat sky: a quadratic would follow the halo wings instead.
    assert!(disc.is_some_and(|d| !d.curved), "flat sky selected the quadratic: {disc:?}");
    assert!(rises.iter().all(|&r| r < 0.5), "the model rose {rises:?} sigma under the two clusters");
}

/// A quadratic refill extrapolates where the rim is one-sided: a cluster off-centre over
/// horizon glow (exponential, not quadratic). Worst error 1.69 sigma (plane 1.30), but
/// mostly halo past the disc edge — 0.61 at six core radii, outside it — plus 0.45 the
/// bilinear grid misses with no cluster at all. Bounds a bent refill, not that leak.
#[test]
fn an_off_centre_cluster_over_horizon_glow_keeps_the_gradient() {
    let s = SIZE as f32;
    let horizon = move |_: usize, y: usize| 40.0 * SIGMA * (-(1.0 - y as f32 / s) * 4.0).exp();
    let (cx, cy, core) = (s * 0.3, s * 0.65, s * 0.05);
    let glow = crowded_field(0.6, |x, y| 1.0 / (1.0 + ((x - cx).powi(2) + (y - cy).powi(2)) / (core * core)));
    let frame = noisy_frame(|x, y| glow[y * SIZE + x] + horizon(x, y));
    let model = bilinear(&frame);
    let errors: Vec<f32> = [(cx, cy), (cx + core * 3.0, cy), (cx, cy - core * 3.0), (cx, cy + core * 3.0)]
        .iter()
        .map(|&(x, y)| {
            let (x, y) = (x as usize, y as usize);
            (model.get_background(x, y, 0) - SKY - horizon(x, y)) / SIGMA
        })
        .collect();
    let worst = errors.iter().copied().fold(0.0f32, |a, e| if e.abs() > a.abs() { e } else { a });
    println!("disc {:?}: worst model error under the cluster {worst:.2} sigma", disc_in(&frame));
    assert!(worst.abs() < 2.0, "the model missed the horizon glow by {worst:.2} sigma under the cluster");
}
