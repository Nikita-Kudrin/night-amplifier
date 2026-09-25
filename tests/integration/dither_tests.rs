//! The dither at the 8-bit boundary, judged on what it leaves in the output bytes.
//!
//! A dither's own threshold spectrum is only half the story: what the eye sees is the
//! *quantised* output, and on a flat field an ordered matrix quantises into a lattice —
//! one dot per tile at the extreme levels, a crosshatch in between. The blue-noise mask
//! replaced the 8x8 Bayer matrix on exactly that measurement (see
//! `render::output::quantize`), so the guard here rebuilds the matrix it replaced and
//! requires the instrument to see its lines: a guard that is only quiet proves nothing.
//!
//! What reaches `/eyepiece` is a JPEG of those bytes, which can take back what the dither
//! put between two levels; the second guard is why a denoised frame is encoded at a higher
//! quality (`server::encoding::jpeg_quality`).

use serial_test::serial;

use crate::integration::instruments::{
    anchors, block_mean_error, cached_stack, format_octaves, green_region, lattice_lines,
    octave_band_sigma, render_with, requested_sessions, stream, LatticeLines, OCTAVE_LABELS,
};

const SIDE: usize = 512;

/// Standard-normal samples from a fixed xorshift, so the guard is reproducible.
fn gaussian(state: &mut u64) -> f64 {
    let mut uniform = || {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        ((*state >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    };
    let (u1, u2) = (uniform(), uniform());
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// A flat sky `level` output levels high with `sigma` levels of noise, in normalised units.
fn flat_sky(level: f64, sigma: f64, seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..SIDE * SIDE)
        .map(|_| ((level + sigma * gaussian(&mut state)) / 255.0) as f32)
        .collect()
}

/// The sky through the production conversion, as the green plane in output levels.
fn production(sky: &[f32]) -> Vec<f64> {
    let output = night_amplifier::render::DisplayOutput::default().with_dither(true);
    let mut plane = Vec::with_capacity(sky.len());
    let mut row_out = vec![0u8; SIDE * 3];
    for (y, row) in sky.chunks_exact(SIDE).enumerate() {
        let row_in: Vec<f32> = row.iter().flat_map(|&v| [v, v, v]).collect();
        night_amplifier::render::output::write_row_rgb8(&mut row_out, &row_in, y, output);
        plane.extend(row_out.iter().skip(1).step_by(3).map(|&v| v as f64));
    }
    plane
}

/// The sky through the 8x8 Bayer matrix the mask replaced, rounded the way
/// `sample_to_u8` rounds. Built from the matrix's bit-interleave definition.
fn bayer(sky: &[f32]) -> Vec<f64> {
    let cell = |x: usize, y: usize| {
        let mut rank = 0;
        for bit in 0..3 {
            let (bx, by) = ((x >> bit) & 1, (y >> bit) & 1);
            rank |= (((bx ^ by) << 1) | by) << (4 - 2 * bit);
        }
        rank as f64
    };
    sky.iter()
        .enumerate()
        .map(|(i, &v)| {
            let offset = ((cell((i % SIDE) & 7, (i / SIDE) & 7) + 0.5) / 64.0 - 0.5) / 255.0;
            ((v as f64 + offset).clamp(0.0, 1.0) * 255.0 + 0.5).floor()
        })
        .collect()
}

fn undithered(sky: &[f32]) -> Vec<f64> {
    sky.iter().map(|&v| ((v as f64).clamp(0.0, 1.0) * 255.0 + 0.5).floor()).collect()
}

fn lines(plane: &[f64], period: usize) -> LatticeLines {
    lattice_lines(plane, SIDE, SIDE, (SIDE / 2, SIDE / 2), period)
}

/// Realizations per level: one fundamental is eight DFT bins, and eight exponentials
/// average to above 2 about 7 % of the time with no dither at all.
const REALIZATIONS: u64 = 8;

/// [`LatticeLines`] averaged over [`REALIZATIONS`] independent noise fields.
fn mean_lines(level: f64, period: usize, quantise: impl Fn(&[f32]) -> Vec<f64>) -> (f64, f64) {
    let (mut fundamental, mut below) = (0.0, 0.0);
    for seed in 0..REALIZATIONS {
        let sky = flat_sky(level, 0.3, 0x2545_F491_4F6C_DD1D ^ (seed << 32) ^ level.to_bits());
        let found = lines(&quantise(&sky), period);
        fundamental += found.fundamental;
        below += found.below_half_nyquist;
    }
    (fundamental / REALIZATIONS as f64, below / REALIZATIONS as f64)
}

/// No lattice line from the dither in a flat, nearly noiseless sky — the only place a
/// dither is all there is to see, and where the 8x8 matrix left one at every level.
///
/// 0.3 output levels of noise: below that a denoised sky is rare, above it the noise
/// itself breaks up the rounding and no dither is visible. Checked at the mask's own 64 px
/// tile too, or a mask that moved the structure there rather than removing it would pass.
#[test]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn the_dither_leaves_no_lattice_in_a_flat_sky() {
    println!("level   Bayer @8 fund / <Nyq/2   mask @8 fund / <Nyq/2   mask @64 fund / <Nyq/2   none @8 fund");
    let mut worst: f64 = 0.0;
    for level in [40.06, 40.25, 40.5, 40.75, 40.94] {
        let reference = mean_lines(level, 8, bayer);
        let at_8 = mean_lines(level, 8, production);
        let at_64 = mean_lines(level, 64, production);
        let none = mean_lines(level, 8, undithered);
        println!(
            "{level:>5.2}   {:>8.1} / {:<8.1}      {:>6.2} / {:<6.2}         {:>6.2} / {:<6.2}          {:>5.2}",
            reference.0, reference.1, at_8.0, at_8.1, at_64.0, at_64.1, none.0,
        );

        assert!(
            reference.0 > 3.0 && reference.1 > 10.0,
            "level {level}: the instrument no longer sees the Bayer matrix's lattice \
             ({:.1}x / {:.1}x the spectrum beside it) — it cannot vouch for anything",
            reference.0,
            reference.1
        );
        worst = worst.max(at_8.0).max(at_8.1).max(at_64.0).max(at_64.1);
    }
    assert!(
        worst < 1.6,
        "the dither puts {worst:.2}x the surrounding power on its own or Bayer's lattice — \
         a periodic pattern the eye can pick out of a flat sky"
    );
}

/// A plane of output levels through the production JPEG encoder at `quality` and back,
/// with the payload's size.
fn through_jpeg(plane: &[f64], quality: i32) -> (Vec<f64>, usize) {
    use night_amplifier::server::encoding::{encode_rgb8_jpeg_bounded_from_u8, SA10_HEADER_SIZE};
    let rgb8: Vec<u8> = plane.iter().flat_map(|&v| [v as u8; 3]).collect();
    let payload = encode_rgb8_jpeg_bounded_from_u8(&rgb8, SIDE as u32, SIDE as u32, quality).unwrap();
    let decoded = turbojpeg::decompress(&payload[SA10_HEADER_SIZE..], turbojpeg::PixelFormat::RGB).unwrap();
    (decoded.pixels.iter().skip(1).step_by(3).map(|&v| v as f64).collect(), payload.len())
}

/// JPEG zeroes the fine detail of each 8x8 block first, and at q90 that is everything the
/// dither puts between two levels: after decode the sky's 8x8 block means miss the input by
/// 4-9x what they did before the encoder. `jpeg_quality` gives a denoised frame 95, which
/// keeps most of it, and this is the measurement that has to go on holding for the extra
/// payload to buy anything.
///
/// The lattice guard's sky (0.3 output levels of noise), flat at two levels and ramped
/// across four; a 512 px patch stands for a 1440p frame, since JPEG quantises each block on
/// its own.
#[test]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn the_denoised_jpeg_quality_keeps_what_the_dither_carries() {
    use night_amplifier::server::encoding::jpeg_quality;
    let (plain_q, denoised_q) = (jpeg_quality(1440, 1440, false), jpeg_quality(1440, 1440, true));
    let region = (0, 0, SIDE, SIDE);
    type Sky = (&'static str, fn(usize) -> f64);
    let skies: [Sky; 3] = [
        ("flat 40.25", |_| 40.25),
        ("flat 40.50", |_| 40.5),
        ("ramp 40-44", |x| 40.0 + 4.0 * x as f64 / SIDE as f64),
    ];
    println!("sky          before  q{plain_q:<3} q{denoised_q:<3}   bytes q{plain_q} / q{denoised_q}");
    let (mut before, mut plain, mut denoised) = (0.0, 0.0, 0.0);
    for (label, level) in skies {
        let truth: Vec<f64> = (0..SIDE * SIDE).map(|i| level(i % SIDE)).collect();
        let mut state = 0x2545_F491_4F6C_DD1D;
        let sky: Vec<f32> = truth.iter().map(|&t| ((t + 0.3 * gaussian(&mut state)) / 255.0) as f32).collect();
        let dithered = production(&sky);
        let (at_plain, plain_bytes) = through_jpeg(&dithered, plain_q);
        let (at_denoised, denoised_bytes) = through_jpeg(&dithered, denoised_q);
        let errors = [&dithered, &at_plain, &at_denoised].map(|p| block_mean_error(p, &truth, SIDE, region, 8));
        println!(
            "{label}   {:>6.3} {:>6.3} {:>6.3}   {plain_bytes} / {denoised_bytes}",
            errors[0], errors[1], errors[2]
        );
        before += errors[0];
        plain += errors[1];
        denoised += errors[2];
    }
    assert!(
        plain > 3.0 * before,
        "q{plain_q} no longer undoes the dither ({plain:.3} against {before:.3} before the encoder) \
         — the instrument cannot vouch for anything"
    );
    assert!(
        denoised < 0.6 * plain,
        "q{denoised_q} keeps no more of the dither than q{plain_q}: block means miss by \
         {denoised:.3} against {plain:.3} (summed over the skies)"
    );
}

/// Every session's sky, rendered with and without the dither: octave bands (which a
/// dither of the right amplitude must not move) and the lattice lines on a 248 px patch,
/// which with the dither on must read as they do with it off.
///
/// Diagnostic over the out-of-repo sessions (`RENDER_BRIGHTNESS_SETS`), no assertions.
/// The 2026-09-24 A/B against the Bayer matrix ran through this, recorded in
/// `render::output::quantize::dither_offset`.
#[test]
#[serial]
#[ignore = "diagnostic - run with: cargo test --release --test integration_pipeline dither -- --ignored --nocapture"]
fn measure_the_dither_on_real_sessions() {
    let settings = night_amplifier::server::state::CaptureSettings::default();
    for (name, dir) in requested_sessions() {
        let Some((stack, depth)) = cached_stack(name, dir) else {
            println!("{name}: not on this machine");
            continue;
        };
        let a = anchors(name, &stack, depth);
        let (x0, y0, x1, y1) = a.sky_box;
        let centre = ((x0 + x1) / 2, (y0 + y1) / 2);
        println!("\n=== {name} ({depth} subs), sky box {:?} ===", a.sky_box);
        println!("{:<8} octaves{}   @8 fund/<Nyq2   @64 fund/<Nyq2", "", OCTAVE_LABELS.map(|l| format!("{l:>7}")).concat());
        for (label, dither) in [("dither", true), ("off", false)] {
            let (rgb8, w, h) = render_with(stack.clone(), &settings, true, stream(), depth, None, |c| {
                c.display = c.display.with_dither(dither);
            });
            let (plane, pw, ph) = green_region(&rgb8, w, (0, 0, w, h));
            let (sky, sw, sh) = green_region(&rgb8, w, a.sky_box);
            let l8 = lattice_lines(&plane, pw, ph, centre, 8);
            let l64 = lattice_lines(&plane, pw, ph, centre, 64);
            println!(
                "{label:<8}        {}   {:>5.2}/{:<5.2}     {:>5.2}/{:<5.2}",
                format_octaves(&octave_band_sigma(&sky, sw, sh)),
                l8.fundamental,
                l8.below_half_nyquist,
                l64.fundamental,
                l64.below_half_nyquist
            );
        }
    }
}
