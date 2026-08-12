//! Bilinear interpolation debayering algorithm
//!
//! Simple averaging of neighboring pixels. Fast and suitable for live stacking
//! where speed matters more than maximum quality.

use rayon::prelude::*;

use crate::debayer::CfaPattern;
use crate::error::Result;
use crate::frame::Frame;

use super::{
    get_rb_orientation, interpolate_blue_at_green, interpolate_diagonal,
    interpolate_green_cardinal, interpolate_red_at_green,
};

/// Channel indices as returned by [`CfaPattern::color_at`]
const RED: usize = 0;
const BLUE: usize = 2;

/// Rows handled by one rayon task: one full CFA period.
///
/// Two rows share their three input rows in cache, and the per-row CFA layout is derived twice per
/// task instead of once per pixel.
const ROWS_PER_TASK: usize = 2;

/// Smallest number of [`ROWS_PER_TASK`]-row chunks a rayon task may cover.
///
/// `with_min_len` counts *chunks*, not rows, and rayon only splits while `len / 2 >= min`, so an
/// oversized value silently caps the task count rather than merely raising the floor: at 32 chunks
/// a 1538-row frame gets at most ~16 tasks, which leaves cores idle on a 20-core box and costs 10 %.
///
/// `current_num_threads` rather than `available_parallelism` on purpose — the value has to describe
/// the pool that will actually run the work, so it adapts inside a scoped `install`.
fn chunk_min_len() -> usize {
    let threads = rayon::current_num_threads();
    if threads < 8 {
        4
    } else {
        threads / 2
    }
}

/// Perform bilinear debayering on a single-channel Bayer frame
pub fn debayer_bilinear(frame: &Frame, pattern: CfaPattern) -> Result<Frame> {
    let output = debayer_impl::<F32Output>(frame, pattern);
    Frame::from_f32_vec(output, frame.width(), frame.height(), 3)
}

/// Perform bilinear debayering directly to a 8-bit RGB vector
/// Bypasses intermediate f32 Frame allocations for encoding/streaming
pub fn debayer_bilinear_to_rgb8(frame: &Frame, pattern: CfaPattern) -> Result<Vec<u8>> {
    Ok(debayer_impl::<Rgb8Output>(frame, pattern))
}

/// Destination format of a debayer pass.
///
/// The f32 and 8-bit variants differ only in how a pixel is stored, so they share one traversal
/// rather than two copies of the interior/border split.
trait DebayerOutput: Send {
    type Elem: Copy + Default + Send;

    fn store(dst: &mut [Self::Elem], idx: usize, rgb: (f32, f32, f32));
}

struct F32Output;

impl DebayerOutput for F32Output {
    type Elem = f32;

    #[inline(always)]
    fn store(dst: &mut [f32], idx: usize, (r, g, b): (f32, f32, f32)) {
        dst[idx] = r;
        dst[idx + 1] = g;
        dst[idx + 2] = b;
    }
}

struct Rgb8Output;

impl DebayerOutput for Rgb8Output {
    type Elem = u8;

    #[inline(always)]
    fn store(dst: &mut [u8], idx: usize, (r, g, b): (f32, f32, f32)) {
        dst[idx] = to_u8(r);
        dst[idx + 1] = to_u8(g);
        dst[idx + 2] = to_u8(b);
    }
}

#[inline(always)]
fn to_u8(value: f32) -> u8 {
    (value.max(0.0).min(1.0) * 255.0 + 0.5) as u8
}

fn debayer_impl<O: DebayerOutput>(frame: &Frame, pattern: CfaPattern) -> Vec<O::Elem> {
    let width = frame.width();
    let height = frame.height();
    let input = frame.data();

    let mut output = vec![O::Elem::default(); width * height * 3];

    output
        .par_chunks_mut(width * 3 * ROWS_PER_TASK)
        .with_min_len(chunk_min_len())
        .enumerate()
        .for_each(|(task, rows)| {
            let y_start = task * ROWS_PER_TASK;
            for (offset, out_row) in rows.chunks_mut(width * 3).enumerate() {
                debayer_row::<O>(input, width, height, pattern, y_start + offset, out_row);
            }
        });

    output
}

/// Per-row CFA constants.
///
/// Both are properties of the row, not of the pixel: a row alternates between green and exactly one
/// of red/blue, and at a green pixel the axis carrying red is fixed by the row — an R row has red
/// left and right, a B row has blue left and right. Deriving these once per row is what keeps
/// `color_at` and `get_rb_orientation` out of the per-pixel path.
#[derive(Copy, Clone)]
struct RowLayout {
    role_at_even_x: usize,
    role_at_odd_x: usize,
    red_horizontal: bool,
}

impl RowLayout {
    #[inline]
    fn new(pattern: CfaPattern, y: usize) -> Self {
        Self {
            role_at_even_x: pattern.color_at(0, y),
            role_at_odd_x: pattern.color_at(1, y),
            red_horizontal: get_rb_orientation(pattern, y).0,
        }
    }
}

fn debayer_row<O: DebayerOutput>(
    input: &[f32],
    width: usize,
    height: usize,
    pattern: CfaPattern,
    y: usize,
    out_row: &mut [O::Elem],
) {
    // No row above or below, so every neighbour fetch has to clamp.
    if y == 0 || y + 1 == height {
        for x in 0..width {
            O::store(
                out_row,
                x * 3,
                bilinear_at(input, width, height, x, y, pattern),
            );
        }
        return;
    }

    let prev = &input[(y - 1) * width..y * width];
    let curr = &input[y * width..(y + 1) * width];
    let next = &input[(y + 1) * width..(y + 2) * width];

    let layout = RowLayout::new(pattern, y);
    let last_x = width.saturating_sub(1);

    // The first and last columns still clamp horizontally.
    O::store(out_row, 0, bilinear_at(input, width, height, 0, y, pattern));
    if last_x > 0 {
        O::store(
            out_row,
            last_x * 3,
            bilinear_at(input, width, height, last_x, y, pattern),
        );
    }

    // Interior: two pixels per step, one full CFA period, so each pixel's column parity — and
    // therefore its role — is known from `layout` without testing anything per pixel.
    let mut x = 1;
    while x + 1 < last_x {
        O::store(
            out_row,
            x * 3,
            interior_pixel(
                prev,
                curr,
                next,
                x,
                layout.role_at_odd_x,
                layout.red_horizontal,
            ),
        );
        O::store(
            out_row,
            (x + 1) * 3,
            interior_pixel(
                prev,
                curr,
                next,
                x + 1,
                layout.role_at_even_x,
                layout.red_horizontal,
            ),
        );
        x += 2;
    }
    if x < last_x {
        O::store(
            out_row,
            x * 3,
            interior_pixel(
                prev,
                curr,
                next,
                x,
                layout.role_at_odd_x,
                layout.red_horizontal,
            ),
        );
    }
}

/// Interpolate an interior pixel, where all eight neighbours are in range.
///
/// `role` and `red_horizontal` are row constants from [`RowLayout`], so nothing here consults the
/// CFA pattern and no index needs clamping.
#[inline(always)]
fn interior_pixel(
    prev: &[f32],
    curr: &[f32],
    next: &[f32],
    x: usize,
    role: usize,
    red_horizontal: bool,
) -> (f32, f32, f32) {
    let this = curr[x];

    match role {
        RED => (
            this,
            green_cardinal(prev, curr, next, x),
            diagonal(prev, next, x),
        ),
        BLUE => (
            diagonal(prev, next, x),
            green_cardinal(prev, curr, next, x),
            this,
        ),
        // Green. `color_at` only ever returns RED, GREEN or BLUE. Both two-tap averages are needed
        // either way — red comes from one axis and blue from the other — so only the pairing
        // depends on the row.
        _ => {
            let horizontal = (curr[x - 1] + curr[x + 1]) * 0.5;
            let vertical = (prev[x] + next[x]) * 0.5;
            if red_horizontal {
                (horizontal, this, vertical)
            } else {
                (vertical, this, horizontal)
            }
        }
    }
}

#[inline(always)]
fn green_cardinal(prev: &[f32], curr: &[f32], next: &[f32], x: usize) -> f32 {
    (curr[x - 1] + curr[x + 1] + prev[x] + next[x]) * 0.25
}

#[inline(always)]
fn diagonal(prev: &[f32], next: &[f32], x: usize) -> f32 {
    (prev[x - 1] + prev[x + 1] + next[x - 1] + next[x + 1]) * 0.25
}

/// Bilinear interpolation at a single pixel, clamping at the frame edge.
///
/// Correct anywhere, which is why the borders and `debayer_vng` use it; [`interior_pixel`] is the
/// fast path for everything else.
#[inline]
pub(crate) fn bilinear_at(
    data: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    pattern: CfaPattern,
) -> (f32, f32, f32) {
    let xi = x as isize;
    let yi = y as isize;
    let this = data[y * width + x];

    match pattern.color_at(x, y) {
        RED => (
            this,
            interpolate_green_cardinal(data, width, height, xi, yi),
            interpolate_diagonal(data, width, height, xi, yi),
        ),
        BLUE => (
            interpolate_diagonal(data, width, height, xi, yi),
            interpolate_green_cardinal(data, width, height, xi, yi),
            this,
        ),
        _ => {
            let (red_horizontal, blue_horizontal) = get_rb_orientation(pattern, y);
            (
                interpolate_red_at_green(data, width, height, xi, yi, red_horizontal),
                this,
                interpolate_blue_at_green(data, width, height, xi, yi, blue_horizontal),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Odd in both axes and tall enough that rayon actually splits the chunk iterator, so the
    /// tests cover a non-zero task index, the single-row tail chunk and the odd-width right border.
    const TEST_WIDTH: usize = 131;
    const TEST_HEIGHT: usize = 139;

    const R_LEVEL: f32 = 0.25;
    const G_LEVEL: f32 = 0.5;
    const B_LEVEL: f32 = 0.75;

    /// Deliberately not a linear ramp. A ramp whose period is commensurate with the row stride
    /// makes the horizontal and vertical two-tap averages equal across most of the frame, which
    /// hides orientation errors — the earlier `(i * 17) % 256` generator did exactly that on 56 %
    /// of pixels.
    fn mixed_value_at(x: usize, y: usize) -> f32 {
        let mut h = (x as u32)
            .wrapping_mul(0x9E37_79B1)
            .wrapping_add((y as u32).wrapping_mul(0x85EB_CA77));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2545_F491);
        h ^= h >> 13;
        (h >> 8) as f32 / 0x00FF_FFFF_u32 as f32
    }

    fn mixed_frame(width: usize, height: usize) -> Frame {
        let data = (0..width * height)
            .map(|i| mixed_value_at(i % width, i / width))
            .collect();
        Frame::from_f32_vec(data, width, height, 1).unwrap()
    }

    /// A mosaic where every red site holds `R_LEVEL`, every green site `G_LEVEL` and every blue
    /// site `B_LEVEL`.
    fn constant_plane_frame(pattern: CfaPattern, width: usize, height: usize) -> Frame {
        let data = (0..width * height)
            .map(|i| match pattern.color_at(i % width, i / width) {
                RED => R_LEVEL,
                BLUE => B_LEVEL,
                _ => G_LEVEL,
            })
            .collect();
        Frame::from_f32_vec(data, width, height, 1).unwrap()
    }

    fn scalar_reference(frame: &Frame, pattern: CfaPattern) -> Vec<f32> {
        let (width, height) = (frame.width(), frame.height());
        let input = frame.data();
        let mut output = vec![0.0f32; width * height * 3];

        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                let idx = (y * width + x) * 3;
                output[idx] = r;
                output[idx + 1] = g;
                output[idx + 2] = b;
            }
        }
        output
    }

    /// The correctness test: interpolating a frame of three constant colour planes must give those
    /// constants back, exactly, at every interior pixel.
    ///
    /// At a red or blue site the cardinal-4 and diagonal-4 averages each sample a single colour
    /// class, so both are exact. At a green site the two-tap averages sample a single class **only
    /// if the red/blue orientation is right** — reading the wrong axis returns the other class and
    /// misses by 0.5. Every level here is a dyadic rational, so the averages are exact in f32 and
    /// the assertion needs no tolerance.
    ///
    /// This is what catches an orientation bug. It answers "is the image right?", not "does it
    /// match what the code used to do", so it cannot be satisfied by preserving an old mistake.
    #[test]
    fn test_debayer_reproduces_constant_colour_planes() {
        for pattern in CfaPattern::all() {
            let frame = constant_plane_frame(pattern, TEST_WIDTH, TEST_HEIGHT);
            let rgb = debayer_bilinear(&frame, pattern).unwrap();
            let data = rgb.data();

            for y in 1..TEST_HEIGHT - 1 {
                for x in 1..TEST_WIDTH - 1 {
                    let idx = (y * TEST_WIDTH + x) * 3;
                    assert_eq!(
                        (data[idx], data[idx + 1], data[idx + 2]),
                        (R_LEVEL, G_LEVEL, B_LEVEL),
                        "{pattern:?} at ({x}, {y}), CFA role {}",
                        pattern.color_at(x, y)
                    );
                }
            }
        }
    }

    /// Same property through the 8-bit path, which applies its own clamp and rounding.
    #[test]
    fn test_debayer_to_rgb8_reproduces_constant_colour_planes() {
        let expected = (
            (R_LEVEL * 255.0 + 0.5) as u8,
            (G_LEVEL * 255.0 + 0.5) as u8,
            (B_LEVEL * 255.0 + 0.5) as u8,
        );

        for pattern in CfaPattern::all() {
            let frame = constant_plane_frame(pattern, TEST_WIDTH, TEST_HEIGHT);
            let rgb = debayer_bilinear_to_rgb8(&frame, pattern).unwrap();

            for y in 1..TEST_HEIGHT - 1 {
                for x in 1..TEST_WIDTH - 1 {
                    let idx = (y * TEST_WIDTH + x) * 3;
                    assert_eq!(
                        (rgb[idx], rgb[idx + 1], rgb[idx + 2]),
                        expected,
                        "{pattern:?} at ({x}, {y})"
                    );
                }
            }
        }
    }

    /// Guards the interior/border split and the per-task row indexing against the plain scalar
    /// `bilinear_at` walk — *not* against any older implementation. The interior kernel keeps the
    /// summation order of its clamping counterpart, so this is exact.
    #[test]
    fn test_parallel_interior_path_matches_scalar_walk() {
        let frame = mixed_frame(TEST_WIDTH, TEST_HEIGHT);

        for pattern in CfaPattern::all() {
            let fast = debayer_bilinear(&frame, pattern).unwrap();
            let reference = scalar_reference(&frame, pattern);

            assert_eq!(fast.data(), &reference[..], "f32 path, {pattern:?}");

            let fast_rgb8 = debayer_bilinear_to_rgb8(&frame, pattern).unwrap();
            let reference_rgb8: Vec<u8> = reference.iter().map(|&v| to_u8(v)).collect();
            assert_eq!(fast_rgb8, reference_rgb8, "rgb8 path, {pattern:?}");
        }
    }

    /// `chunk_min_len` scales with the pool, so the work split now depends on the thread count.
    /// Output must not.
    #[test]
    fn test_output_is_independent_of_thread_count() {
        let frame = mixed_frame(TEST_WIDTH, TEST_HEIGHT);
        let mut expected: Option<(Vec<f32>, Vec<u8>)> = None;

        for threads in [1usize, 3, 8] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let produced = pool.install(|| {
                (
                    debayer_bilinear(&frame, CfaPattern::Grbg)
                        .unwrap()
                        .data()
                        .to_vec(),
                    debayer_bilinear_to_rgb8(&frame, CfaPattern::Grbg).unwrap(),
                )
            });

            match &expected {
                None => expected = Some(produced),
                Some(first) => {
                    assert_eq!(&produced, first, "output changed with {threads} threads")
                }
            }
        }
    }

    /// A green pixel's red/blue axes depend on the row only. GRBG is the pattern that used to get
    /// this wrong at odd rows, where the vertical neighbours are red and the horizontal ones blue.
    #[test]
    fn test_green_orientation_matches_the_neighbouring_cfa_sites() {
        for pattern in CfaPattern::all() {
            for y in 2..4 {
                for x in 2..4 {
                    if pattern.color_at(x, y) != 1 {
                        continue;
                    }
                    let (red_horizontal, blue_horizontal) = get_rb_orientation(pattern, y);
                    let horizontal_site = pattern.color_at(x + 1, y);

                    assert_eq!(
                        red_horizontal,
                        horizontal_site == RED,
                        "{pattern:?} green at ({x}, {y}): horizontal neighbour is {horizontal_site}"
                    );
                    assert_eq!(blue_horizontal, !red_horizontal, "{pattern:?}");
                }
            }
        }
    }
}
