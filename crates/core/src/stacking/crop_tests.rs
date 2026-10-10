use super::*;

/// A coverage-only field over a `width x height` image, one value per cell.
fn field(cells: Vec<f32>, cw: usize, ch: usize, width: usize, height: usize) -> NoiseField {
    NoiseField::coverage_only(cells, cw, ch, 3, width, height).unwrap()
}

/// A field of `width x height` px, fully covered except where `thin(cx, cy)` says otherwise.
fn field_with(width: usize, height: usize, value: impl Fn(usize, usize) -> f32) -> NoiseField {
    let (cw, ch) = (width.div_ceil(NOISE_REDUCTION), height.div_ceil(NOISE_REDUCTION));
    let cells = (0..cw * ch).map(|i| value(i % cw, i / cw)).collect();
    field(cells, cw, ch, width, height)
}

/// Coverage as `MasterStack` measures it — the median of `offered / N` over each cell — for
/// subs placed by `(rotation rad about the centre, tx, ty)` against the reference.
fn synthetic_coverage(width: usize, height: usize, subs: &[(f64, f64, f64)]) -> NoiseField {
    let mut offered = vec![0u16; width * height];
    let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
    for &(rotation, tx, ty) in subs {
        let (s, c) = rotation.sin_cos();
        for y in 0..height {
            for x in 0..width {
                let (dx, dy) = (x as f64 - cx - tx, y as f64 - cy - ty);
                let (sx, sy) = (c * dx + s * dy + cx, -s * dx + c * dy + cy);
                if sx >= 0.0 && sx < (width - 1) as f64 && sy >= 0.0 && sy < (height - 1) as f64 {
                    offered[y * width + x] += 1;
                }
            }
        }
    }
    let r = NOISE_REDUCTION;
    let (cw, ch) = (width.div_ceil(r), height.div_ceil(r));
    let mut cells = vec![0.0f32; cw * ch];
    for by in 0..ch {
        for bx in 0..cw {
            let mut counts: Vec<u16> = (by * r..((by + 1) * r).min(height))
                .flat_map(|y| (bx * r..((bx + 1) * r).min(width)).map(move |x| (x, y)))
                .map(|(x, y)| offered[y * width + x])
                .collect();
            counts.sort_unstable();
            cells[by * cw + bx] = counts[counts.len() / 2] as f32 / subs.len() as f32;
        }
    }
    field(cells, cw, ch, width, height)
}

fn share(rect: &CropRect, width: usize, height: usize) -> f64 {
    rect.area() as f64 / (width * height) as f64
}

// ---------------------------------------------------------------------------
// largest_covered
// ---------------------------------------------------------------------------

#[test]
fn a_fully_covered_stack_is_its_own_rectangle() {
    let f = field_with(64, 48, |_, _| 1.0);
    let rect = largest_covered(&f, PICK_COVERAGE).unwrap();
    assert!(rect.is_full(64, 48), "{rect:?}");
}

#[test]
fn a_thin_strip_is_cut_away_exactly() {
    let f = field_with(80, 40, |x, _| if x < 2 { 0.5 } else { 1.0 });
    assert_eq!(largest_covered(&f, PICK_COVERAGE), Some(CropRect { x: 16, y: 0, width: 64, height: 40 }));
}

#[test]
fn the_far_edges_clamp_to_an_image_off_the_cell_grid() {
    // 61x45 px: the last cell column holds 5 px, the last row 5.
    let f = field_with(61, 45, |x, y| if x == 0 || y == 0 { 0.2 } else { 1.0 });
    assert_eq!(largest_covered(&f, PICK_COVERAGE), Some(CropRect { x: 8, y: 8, width: 53, height: 37 }));
}

#[test]
fn a_cell_exactly_at_the_threshold_passes() {
    // 19 of 20 subs, computed as the stack computes it.
    let at = 19.0f32 / 20.0;
    let f = field_with(32, 32, |x, _| if x == 0 { at } else { 1.0 });
    assert!(largest_covered(&f, PICK_COVERAGE).unwrap().is_full(32, 32));
}

#[test]
fn no_covered_cell_means_no_rectangle() {
    let f = field_with(32, 32, |_, _| 0.1);
    assert_eq!(largest_covered(&f, PICK_COVERAGE), None);
}

/// Against an exhaustive search over every rectangle, on random masks: the histogram walk
/// finds the largest, not just a large one.
#[test]
fn the_rectangle_found_is_the_largest_there_is() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..200 {
        let (cw, ch) = (1 + (next() % 9) as usize, 1 + (next() % 9) as usize);
        let cells: Vec<f32> = (0..cw * ch).map(|_| if next() % 4 == 0 { 0.3 } else { 1.0 }).collect();
        let good = |x: usize, y: usize| cells[y * cw + x] >= PICK_COVERAGE;
        let mut best = 0;
        for y0 in 0..ch {
            for x0 in 0..cw {
                for y1 in y0 + 1..=ch {
                    for x1 in x0 + 1..=cw {
                        if (y0..y1).all(|y| (x0..x1).all(|x| good(x, y))) {
                            best = best.max((x1 - x0) * (y1 - y0));
                        }
                    }
                }
            }
        }
        let f = field(cells.clone(), cw, ch, cw * NOISE_REDUCTION, ch * NOISE_REDUCTION);
        let found = largest_covered(&f, PICK_COVERAGE).map_or(0, |r| r.area() / (NOISE_REDUCTION * NOISE_REDUCTION));
        assert_eq!(found, best, "{cw}x{ch} mask {cells:?}");
        if let Some(rect) = largest_covered(&f, PICK_COVERAGE) {
            assert!(min_coverage(&f, &rect).unwrap() >= PICK_COVERAGE, "{rect:?} holds a thin cell");
        }
    }
}

// ---------------------------------------------------------------------------
// Mount scenarios, as the stack would measure them
// ---------------------------------------------------------------------------

const W: usize = 800;
const H: usize = 600;
const SUBS: usize = 60;

/// What a coverage-threshold crop and the subs' common footprint (every cell at 100 %) keep.
fn crop_and_footprint(subs: &[(f64, f64, f64)]) -> (Option<CropRect>, f64) {
    let f = synthetic_coverage(W, H, subs);
    let decision = CropTracker::new().observe(subs.len(), Some(&f));
    if let Some(rect) = decision.rect() {
        assert!(min_coverage(&f, &rect).unwrap() >= PICK_COVERAGE, "{rect:?} holds a thin cell");
    }
    let footprint = largest_covered(&f, 1.0).map_or(0.0, |r| share(&r, W, H));
    (decision.rect(), footprint)
}

/// Shares measured on this geometry when the constants were chosen; a change of rule or
/// constant that moves them should be a decision, not a side effect.
fn assert_share(rect: Option<CropRect>, expected: f64, scenario: &str) {
    let got = rect.map_or(1.0, |r| share(&r, W, H));
    assert!((got - expected).abs() < 0.01, "{scenario}: kept {:.1} %, expected {:.1} %", 100.0 * got, 100.0 * expected);
}

#[test]
fn drift_is_cropped_to_the_area_most_subs_share() {
    let subs: Vec<_> = (0..SUBS).map(|i| (0.0, 2.5 * i as f64, 1.25 * i as f64)).collect();
    let (rect, _) = crop_and_footprint(&subs);
    assert_share(rect, 0.729, "150 px diagonal drift");
}

#[test]
fn field_rotation_is_cropped_to_its_inscribed_rectangle() {
    let subs: Vec<_> = (0..SUBS).map(|i| ((0.05 * i as f64).to_radians(), 0.0, 0.0)).collect();
    let (rect, _) = crop_and_footprint(&subs);
    assert_share(rect, 0.909, "3° of field rotation");
}

#[test]
fn rotation_with_drift_is_cropped_to_both() {
    let subs: Vec<_> = (0..SUBS).map(|i| ((0.05 * i as f64).to_radians(), 1.25 * i as f64, 0.0)).collect();
    let (rect, _) = crop_and_footprint(&subs);
    assert_share(rect, 0.852, "3° rotation + 75 px drift");
}

/// A permanent shift: the area only the first 20 subs reached holds a third of the stack.
#[test]
fn a_step_is_cropped_to_what_both_positions_share() {
    let subs: Vec<_> = (0..SUBS).map(|i| if i < 20 { (0.0, 0.0, 0.0) } else { (0.0, 125.0, 0.0) }).collect();
    let (rect, _) = crop_and_footprint(&subs);
    assert_eq!(rect, Some(CropRect { x: 128, y: 0, width: 672, height: 600 }));
}

/// The case the coverage threshold exists for: a single bumped sub that still registered
/// costs the common footprint almost half the frame, and the crop nothing.
#[test]
fn bumped_subs_are_not_worth_a_crop() {
    let one: Vec<_> = (0..SUBS).map(|i| if i == 30 { (0.0, 200.0, 150.0) } else { (0.0, 0.0, 0.0) }).collect();
    let three: Vec<_> = (0..SUBS)
        .map(|i| match i {
            10 => (0.0, 200.0, 0.0),
            30 => (0.0, -150.0, 100.0),
            50 => (0.0, 0.0, -175.0),
            _ => (0.0, 0.0, 0.0),
        })
        .collect();
    for (subs, footprint_at_most, scenario) in [(one, 0.60, "one bump"), (three, 0.35, "three bumps")] {
        let (rect, footprint) = crop_and_footprint(&subs);
        assert_eq!(rect, None, "{scenario}: cropped to {rect:?}");
        assert!(footprint < footprint_at_most, "{scenario}: the footprint kept {:.1} %", 100.0 * footprint);
    }
}

/// The same bump early in the session, seen depth by depth as the live view sees it: below
/// 20 subs `PICK_COVERAGE` admits no missing sub at all, so one bumped sub cut the view to
/// the common footprint for the next 14 subs, then popped back to whole at depth 20.
#[test]
fn an_early_bump_is_not_worth_a_crop_either() {
    let subs: Vec<_> = (0..SUBS).map(|i| if i == 5 { (0.0, 200.0, 150.0) } else { (0.0, 0.0, 0.0) }).collect();
    let mut tracker = CropTracker::new();
    let cropped: Vec<usize> = (2..=SUBS)
        .filter(|&depth| tracker.observe(depth, Some(&synthetic_coverage(W, H, &subs[..depth]))).rect().is_some())
        .collect();
    assert!(cropped.is_empty(), "one bumped sub cropped the view at depths {cropped:?}");
}

#[test]
fn periodic_error_costs_only_its_amplitude() {
    let subs: Vec<_> = (0..SUBS).map(|i| (0.0, 12.5 * (i as f64 * 0.7).sin(), 0.0)).collect();
    let (rect, _) = crop_and_footprint(&subs);
    assert_share(rect, 0.960, "±12.5 px periodic error");
}

// ---------------------------------------------------------------------------
// CropTracker
// ---------------------------------------------------------------------------

/// 160x160 px, 20x20 cells, with the leftmost `columns` cells at `thin` coverage.
fn left_band(columns: usize, thin: f32) -> NoiseField {
    field_with(160, 160, move |x, _| if x < columns { thin } else { 1.0 })
}

#[test]
fn a_complete_stack_is_shown_whole() {
    let mut tracker = CropTracker::new();
    assert_eq!(tracker.observe(30, None), CropDecision::Uncropped);
    assert_eq!(tracker.observe(31, Some(&field_with(64, 64, |_, _| 1.0))), CropDecision::Uncropped);
    assert_eq!(tracker.shown(), None);
}

#[test]
fn the_shown_crop_holds_while_its_cells_keep_their_share() {
    let mut tracker = CropTracker::new();
    let first = tracker.observe(30, Some(&left_band(2, 0.5))).rect().unwrap();
    assert_eq!(first, CropRect { x: 16, y: 0, width: 144, height: 160 });

    // The next column thins to 0.9: below a fresh pick's 0.95, above the 0.85 kept.
    let thinner = field_with(160, 160, |x, _| match x {
        0 | 1 => 0.5,
        2 => 0.9,
        _ => 1.0,
    });
    assert_eq!(tracker.observe(31, Some(&thinner)).rect(), Some(first), "the crop moved within its band");

    // ...and to 0.8, which the shown crop can no longer hold.
    let thinnest = field_with(160, 160, |x, _| match x {
        0 | 1 => 0.5,
        2 => 0.8,
        _ => 1.0,
    });
    assert_eq!(tracker.observe(32, Some(&thinnest)).rect(), Some(CropRect { x: 24, y: 0, width: 136, height: 160 }));
}

/// 320x160 px, 40x20 cells, with the leftmost `columns` cells half covered: one column is
/// under [`GROW_SHARE`]'s 5 % of the area, two are over it.
fn wide_band(columns: usize) -> NoiseField {
    field_with(320, 160, move |x, _| if x < columns { 0.5 } else { 1.0 })
}

#[test]
fn a_crop_grows_back_once_a_fresh_pick_is_clearly_larger() {
    let mut tracker = CropTracker::new();
    tracker.observe(30, Some(&wide_band(4)));
    assert_eq!(tracker.observe(31, Some(&wide_band(3))).rect().unwrap().x, 32, "grew by 2.8 %");
    assert_eq!(tracker.observe(32, Some(&wide_band(2))).rect().unwrap().x, 16, "did not grow by 5.6 %");
}

#[test]
fn a_stack_that_fills_back_in_is_uncropped() {
    let mut tracker = CropTracker::new();
    assert!(tracker.observe(30, Some(&left_band(1, 0.5))).rect().is_some());
    assert_eq!(tracker.observe(40, Some(&left_band(1, 0.96))), CropDecision::Uncropped);
}

/// A restart takes the depth down; the new stack's first pick owes nothing to the old one.
#[test]
fn a_restarted_stack_starts_a_fresh_crop() {
    let mut tracker = CropTracker::new();
    tracker.observe(30, Some(&wide_band(4)));
    // Still valid for the shown crop, and only 2.8 % larger: kept, had the stack not restarted.
    assert_eq!(tracker.observe(31, Some(&wide_band(3))).rect().unwrap().x, 32);
    assert_eq!(tracker.observe(MIN_CROP_DEPTH, Some(&wide_band(3))).rect().unwrap().x, 24);
}

#[test]
fn a_crop_to_a_sliver_is_refused() {
    let mut tracker = CropTracker::new();
    // Only the rightmost column is covered: 8 px wide, 5 % of the frame.
    let sliver = field_with(160, 160, |x, _| if x == 19 { 1.0 } else { 0.3 });
    assert_eq!(tracker.observe(30, Some(&sliver)), CropDecision::TooThin);
    assert_eq!(tracker.shown(), None);
}

#[test]
fn a_shown_crop_that_no_longer_fits_the_image_is_replaced() {
    let mut tracker = CropTracker::new();
    tracker.observe(30, Some(&left_band(2, 0.5)));
    let smaller = field_with(96, 96, |x, _| if x == 0 { 0.5 } else { 1.0 });
    assert_eq!(tracker.observe(31, Some(&smaller)).rect(), Some(CropRect { x: 8, y: 0, width: 88, height: 96 }));
}

/// The depth gate sits where the pick first tolerates one missing sub, not before it.
#[test]
fn the_crop_waits_for_a_depth_that_outvotes_one_sub() {
    let tolerates = |n: usize| (n - 1) as f32 / n as f32 >= PICK_COVERAGE - THRESHOLD_SLACK;
    assert!(tolerates(MIN_CROP_DEPTH) && !tolerates(MIN_CROP_DEPTH - 1), "MIN_CROP_DEPTH {MIN_CROP_DEPTH}");

    let mut tracker = CropTracker::new();
    let thin = left_band(2, 0.5);
    for depth in 2..MIN_CROP_DEPTH {
        assert_eq!(tracker.observe(depth, Some(&thin)), CropDecision::Uncropped, "depth {depth}");
    }
    assert!(tracker.observe(MIN_CROP_DEPTH, Some(&thin)).rect().is_some());
    // A restart below the gate shows the new stack whole again.
    assert_eq!(tracker.observe(3, Some(&thin)), CropDecision::Uncropped);
    assert_eq!(tracker.shown(), None);
}
