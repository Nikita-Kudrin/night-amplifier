use super::*;

#[test]
fn box_size_is_always_odd_and_at_least_the_floor() {
    // 2712 * 0.015 = 40.68 -> 40 -> 41
    assert_eq!(compute_box_size(2712), 41);
    // Below the floor, the floor wins and is already odd.
    assert_eq!(compute_box_size(100), MIN_BOX_SIZE);
    for width in [64usize, 200, 640, 1936, 3008, 4144] {
        assert!(!compute_box_size(width).is_multiple_of(2));
    }
}

#[test]
fn median_handles_both_parities_and_the_empty_case() {
    assert_eq!(median(&mut []), 0.0);
    assert_eq!(median(&mut [5.0]), 5.0);
    assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
    assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
}

#[test]
fn mad_and_mad_with_scratch_agree() {
    let values = [1.0f32, 2.0, 3.0, 10.0];
    let med = median(&mut values.to_vec());
    let mut scratch = vec![0.0; 99];
    assert_eq!(mad(&values, med), mad_with_scratch(&values, med, &mut scratch));
    assert_eq!(mad(&[], 0.0), 0.0);
}

/// Deterministic N(0, 1).
fn gaussian(seed: &mut u32) -> f32 {
    let mut uniform = || {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / (1u32 << 24) as f32 + 1e-7
    };
    let (u1, u2) = (uniform(), uniform());
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

/// Both extractors take the pruning channel's node values from the sample, so it has to
/// be exactly what the per-channel extraction returns — for every node, frame edges, a
/// flat (zero-dispersion) patch, star clusters and nodes off the frame included.
#[test]
fn node_sample_value_matches_node_value() {
    let mut seed = 7;
    let mut frame = Frame::zeros(96, 96, 1).unwrap();
    for y in 0..96 {
        for x in 0..96 {
            let flat = x >= 64 && y >= 64;
            frame.set_pixel(x, y, 0, if flat { 0.2 } else { 0.1 + 0.01 * gaussian(&mut seed) });
        }
    }
    for (x, y) in [(40, 40), (41, 40), (40, 41), (10, 80), (90, 5)] {
        frame.set_pixel(x, y, 0, 0.9);
    }
    let nodes = (0..=12)
        .flat_map(|row| (0..=12).map(move |col| GridNode::new(col * 8, row * 8, col, row)))
        .chain([GridNode::new(200, 200, 0, 0)]);
    for node in nodes {
        let sample = extract_node_sample(&frame, &node, 21, 0);
        assert_eq!(sample.map(|s| s.value), extract_node_value(&frame, &node, 21, 0), "node at {}, {}", node.x, node.y);
    }
}

/// A gradient across the box is not roughness; unresolved structure is.
#[test]
fn scatter_ignores_a_gradient_but_sees_crowding() {
    let (size, sigma) = (64usize, 0.01f32);
    let build = |extra: &dyn Fn(usize, usize) -> f32| {
        let mut seed = 11;
        let mut frame = Frame::zeros(size, size, 1).unwrap();
        for y in 0..size {
            for x in 0..size {
                frame.set_pixel(x, y, 0, 0.1 + sigma * gaussian(&mut seed) + extra(x, y));
            }
        }
        extract_node_sample(&frame, &GridNode::new(32, 32, 0, 0), 41, 0).unwrap()
    };
    let flat = build(&|_, _| 0.0);
    let sloped = build(&|x, _| 0.05 * sigma * x as f32);
    // Faint unresolved stars at random, 5 % of pixels, ~0.6 sigma of glow: the real
    // halo read 11-39 % rougher than open sky.
    let mut star_seed = 23u32;
    let stars: Vec<(f32, f32)> = (0..size * size / 20)
        .map(|_| {
            let mut next = || {
                star_seed = star_seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (star_seed >> 8) as f32 / (1u32 << 24) as f32 * size as f32
            };
            (next(), next())
        })
        .collect();
    let crowded = build(&|x, y| {
        stars
            .iter()
            .map(|&(cx, cy)| {
                let r2 = (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2);
                sigma * (-r2 / 4.0).exp()
            })
            .sum()
    });
    assert!((sloped.scatter / flat.scatter - 1.0).abs() < 0.05, "{sloped:?} vs {flat:?}");
    assert!(crowded.scatter > flat.scatter * 1.08, "{crowded:?} vs {flat:?}");
}

/// A star inside the box must be clipped away, leaving the sky level.
#[test]
fn node_extraction_rejects_a_star_in_the_box() {
    let mut frame = Frame::filled(64, 64, 1, 0.1).unwrap();
    for y in 30..34 {
        for x in 30..34 {
            frame.set_pixel(x, y, 0, 0.95);
        }
    }
    let node = GridNode::new(32, 32, 0, 0);
    let value = extract_node_value(&frame, &node, 21, 0).unwrap();
    assert!(
        (value - 0.1).abs() < 1e-4,
        "sigma clipping should have left the 0.1 sky level, got {value}"
    );
}

/// The node reads the plane it was asked for, not plane 0 with an offset slip.
/// A colour fixture is what makes that observable at all.
#[test]
fn node_extraction_reads_the_requested_plane() {
    let mut frame = Frame::zeros(32, 32, 3).unwrap();
    for y in 0..32 {
        for x in 0..32 {
            frame.set_pixel(x, y, 0, 0.10);
            frame.set_pixel(x, y, 1, 0.50);
            frame.set_pixel(x, y, 2, 0.90);
        }
    }
    let node = GridNode::new(16, 16, 0, 0);
    for (channel, want) in [(0usize, 0.10f32), (1, 0.50), (2, 0.90)] {
        let got = extract_node_value(&frame, &node, 9, channel).unwrap();
        assert!((got - want).abs() < 1e-6, "channel {channel}: {got} != {want}");
    }
}

/// A node whose box falls entirely outside the frame has nothing to sample.
#[test]
fn a_node_outside_the_frame_yields_nothing() {
    let frame = Frame::filled(16, 16, 1, 0.2).unwrap();
    let node = GridNode::new(64, 64, 0, 0);
    assert!(extract_node_value(&frame, &node, 9, 0).is_none());
}

#[test]
fn pruning_rejects_a_bright_node_and_keeps_the_sky() {
    let cols = 4;
    let rows = 4;
    let mut nodes: Vec<GridNode> = (0..rows)
        .flat_map(|row| (0..cols).map(move |col| GridNode::new(col, row, col, row)))
        .collect();
    for node in nodes.iter_mut() {
        node.value = Some(0.1);
    }
    nodes[5].value = Some(0.9);

    prune_nebulosity(
        &mut nodes,
        cols,
        rows,
        PruneConfig {
            global_sigma: 2.5,
            neighbour_threshold: 1.05,
        },
    );

    assert!(nodes[5].value.is_none(), "the bright node should be pruned");
    assert_eq!(
        nodes.iter().filter(|n| n.value.is_some()).count(),
        cols * rows - 1,
        "only the bright node should be pruned"
    );
}

/// A stricter config prunes strictly more. Pins that the thresholds are actually
/// wired through rather than shadowed by a constant.
#[test]
fn a_stricter_config_prunes_at_least_as_much() {
    let cols = 6;
    let rows = 6;
    let build = || {
        let mut nodes: Vec<GridNode> = (0..rows)
            .flat_map(|row| (0..cols).map(move |col| GridNode::new(col, row, col, row)))
            .collect();
        for (i, node) in nodes.iter_mut().enumerate() {
            node.value = Some(0.1 + (i % 5) as f32 * 0.01);
        }
        nodes
    };

    let mut lenient = build();
    prune_nebulosity(
        &mut lenient,
        cols,
        rows,
        PruneConfig {
            global_sigma: 2.5,
            neighbour_threshold: 1.05,
        },
    );

    let mut strict = build();
    prune_nebulosity(
        &mut strict,
        cols,
        rows,
        PruneConfig {
            global_sigma: 1.0,
            neighbour_threshold: 1.02,
        },
    );

    let survivors = |n: &[GridNode]| n.iter().filter(|g| g.value.is_some()).count();
    assert!(
        survivors(&strict) < survivors(&lenient),
        "strict {} vs lenient {} — the thresholds are not reaching the algorithm",
        survivors(&strict),
        survivors(&lenient)
    );
}
