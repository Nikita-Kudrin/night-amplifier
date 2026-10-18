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

/// Both edges and the inset lines are nodes; the inner lines keep their even spacing.
#[test]
fn node_axis_hugs_both_edges_and_adds_the_inset_lines() {
    assert_eq!(node_axis(1000, 12, 0.0), (0..12).map(|i| i * 999 / 11).collect::<Vec<_>>());
    let axis = node_axis(1000, 12, 0.035);
    assert_eq!(axis.len(), 14);
    assert_eq!((axis[0], axis[1], axis[12], axis[13]), (0, 35, 964, 999));
    assert!(axis.windows(2).all(|w| w[0] < w[1]), "strictly increasing: {axis:?}");
}

/// An inset past the first inner line would cross it, so it is left out.
#[test]
fn an_inset_wider_than_the_spacing_is_dropped() {
    assert_eq!(node_axis(100, 12, 0.2), node_axis(100, 12, 0.0));
}

#[test]
fn layout_nodes_is_row_major_with_grid_indices() {
    let nodes = layout_nodes(&[0, 5, 9], &[0, 7]);
    assert_eq!(nodes.len(), 6);
    let n = nodes[4];
    assert_eq!((n.x, n.y, n.col, n.row), (5, 7, 1, 1));
}
