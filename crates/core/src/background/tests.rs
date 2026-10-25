use super::*;

#[test]
fn test_uniform_background() {
    // Create a uniform frame - background should be uniform
    let frame = Frame::filled(256, 256, 3, 0.3).unwrap();
    let extractor = BackgroundExtractor::with_defaults();
    let model = extractor.estimate(&frame).unwrap();

    // Check that all grid values are close to 0.3
    for c in 0..3 {
        for v in model.grid_values(c) {
            assert!((v - 0.3).abs() < 0.01, "Grid value {} differs from 0.3", v);
        }
    }
}

#[test]
fn test_gradient_background() {
    // Create a frame with a horizontal gradient
    let mut frame = Frame::zeros(256, 256, 1).unwrap();
    for y in 0..256 {
        for x in 0..256 {
            let value = x as f32 / 255.0 * 0.5; // 0.0 to 0.5 gradient
            frame.set_pixel(x, y, 0, value);
        }
    }

    let extractor = BackgroundExtractor::new(BackgroundConfig::default().with_grid_size(8, 8));
    let model = extractor.estimate(&frame).unwrap();

    // Left side should be lower than right side
    let left_val = model.get_background(0, 128, 0);
    let right_val = model.get_background(255, 128, 0);
    assert!(left_val < right_val, "Gradient not detected");
    assert!(left_val < 0.1, "Left side should be near 0");
    assert!(right_val > 0.4, "Right side should be near 0.5");
}

#[test]
fn test_star_rejection() {
    // Create a uniform background with a bright "star"
    let mut frame = Frame::filled(64, 64, 1, 0.2).unwrap();

    // Add a bright star in the center
    frame.set_pixel(32, 32, 0, 1.0);
    frame.set_pixel(31, 32, 0, 0.8);
    frame.set_pixel(33, 32, 0, 0.8);
    frame.set_pixel(32, 31, 0, 0.8);
    frame.set_pixel(32, 33, 0, 0.8);

    let extractor = BackgroundExtractor::new(BackgroundConfig::default().with_grid_size(4, 4));
    let model = extractor.estimate(&frame).unwrap();

    // The background should still be close to 0.2 despite the star
    let center_bg = model.get_background(32, 32, 0);
    assert!(
        (center_bg - 0.2).abs() < 0.05,
        "Star not rejected, background is {}",
        center_bg
    );
}

#[test]
fn test_background_subtraction_full() {
    // Test full background subtraction mode (gradient_only: false)
    let mut frame = Frame::filled(128, 128, 1, 0.4).unwrap();

    // Add a "star" at 0.9
    frame.set_pixel(64, 64, 0, 0.9);

    let config = BackgroundConfig::default().with_gradient_only(false);
    let extractor = BackgroundExtractor::new(config);
    extractor.subtract(&mut frame).unwrap();

    // Background should be near 0 after full subtraction
    let corner_val = frame.get_pixel(0, 0, 0);
    assert!(
        corner_val < 0.05,
        "Background not removed, got {}",
        corner_val
    );

    // Star should still be visible (above background)
    let star_val = frame.get_pixel(64, 64, 0);
    assert!(star_val > 0.3, "Star was removed, got {}", star_val);
}

#[test]
fn test_background_subtraction_gradient_only() {
    // Test gradient-only mode (default): uniform background should be preserved
    let mut frame = Frame::filled(128, 128, 1, 0.4).unwrap();

    // Add a "star" at 0.9
    frame.set_pixel(64, 64, 0, 0.9);

    let extractor = BackgroundExtractor::with_defaults();
    extractor.subtract(&mut frame).unwrap();

    // With uniform background and gradient_only=true, background should be preserved
    let corner_val = frame.get_pixel(0, 0, 0);
    assert!(
        (corner_val - 0.4).abs() < 0.05,
        "Uniform background should be preserved in gradient-only mode, got {}",
        corner_val
    );

    // Star should still be at 0.9
    let star_val = frame.get_pixel(64, 64, 0);
    assert!(
        (star_val - 0.9).abs() < 0.05,
        "Star should be preserved, got {}",
        star_val
    );
}

#[test]
fn test_gradient_subtraction() {
    // Test that gradient-only mode removes gradients while preserving base level
    let mut frame = Frame::zeros(256, 256, 1).unwrap();
    for y in 0..256 {
        for x in 0..256 {
            // Gradient from 0.1 to 0.6
            let value = 0.1 + x as f32 / 255.0 * 0.5;
            frame.set_pixel(x, y, 0, value);
        }
    }

    let extractor = BackgroundExtractor::new(BackgroundConfig::default().with_grid_size(8, 8));
    extractor.subtract(&mut frame).unwrap();

    // After gradient-only subtraction, the image should be relatively uniform
    // at approximately the minimum background level (0.1)
    let left_val = frame.get_pixel(10, 128, 0);
    let right_val = frame.get_pixel(245, 128, 0);

    // Values should be close to each other (gradient removed)
    assert!(
        (left_val - right_val).abs() < 0.1,
        "Gradient not removed: left={}, right={}",
        left_val,
        right_val
    );

    // Values should be near the original minimum (~0.1)
    assert!(left_val > 0.05, "Signal level too low: {}", left_val);
}

#[test]
fn test_no_negative_values() {
    // Create a frame with low values
    let mut frame = Frame::filled(64, 64, 1, 0.1).unwrap();

    // Set some pixels even lower
    frame.set_pixel(32, 32, 0, 0.05);

    let extractor = BackgroundExtractor::with_defaults();
    extractor.subtract(&mut frame).unwrap();

    // Ensure no negative values
    for &v in frame.data() {
        assert!(v >= 0.0, "Negative value found: {}", v);
    }
}

#[test]
fn test_bilinear_interpolation() {
    // Create a simple 2x2 grid model and check interpolation
    let model = BackgroundModel::new(
        vec![vec![0.0, 1.0, 0.0, 1.0]], // 2x2 grid: TL=0, TR=1, BL=0, BR=1
        2,
        2,
        100,
        100,
        1,
        true,
        0.1,
        1.0,
    );

    // Center should be average of all corners
    let center = model.get_background(50, 50, 0);
    assert!(
        (center - 0.5).abs() < 0.1,
        "Center interpolation wrong: {}",
        center
    );

    // Far left should be low
    let left = model.get_background(0, 50, 0);
    assert!(left < 0.3, "Left interpolation wrong: {}", left);

    // Far right should be high
    let right = model.get_background(99, 50, 0);
    assert!(right > 0.7, "Right interpolation wrong: {}", right);
}

#[test]
fn test_to_frame() {
    let frame = Frame::filled(64, 64, 3, 0.5).unwrap();
    let extractor = BackgroundExtractor::with_defaults();
    let model = extractor.estimate(&frame).unwrap();

    let bg_frame = model.to_frame().unwrap();
    assert_eq!(bg_frame.width(), 64);
    assert_eq!(bg_frame.height(), 64);
    assert_eq!(bg_frame.channels(), 3);
}

#[test]
fn test_multichannel() {
    // Create a frame with different backgrounds per channel
    // `set_pixel` rather than hand-computed offsets: the fixture must not encode
    // an assumption about the memory layout it is testing through.
    let mut frame = Frame::zeros(64, 64, 3).unwrap();
    for y in 0..64 {
        for x in 0..64 {
            frame.set_pixel(x, y, 0, 0.1);
            frame.set_pixel(x, y, 1, 0.3);
            frame.set_pixel(x, y, 2, 0.5);
        }
    }

    let extractor = BackgroundExtractor::with_defaults();
    let model = extractor.estimate(&frame).unwrap();

    let r = model.get_background(32, 32, 0);
    let g = model.get_background(32, 32, 1);
    let b = model.get_background(32, 32, 2);

    assert!((r - 0.1).abs() < 0.02, "R channel wrong: {}", r);
    assert!((g - 0.3).abs() < 0.02, "G channel wrong: {}", g);
    assert!((b - 0.5).abs() < 0.02, "B channel wrong: {}", b);
}

#[test]
fn test_convenience_function() {
    // Create a frame with a gradient
    let mut frame = Frame::zeros(64, 64, 1).unwrap();
    for y in 0..64 {
        for x in 0..64 {
            let value = 0.1 + x as f32 / 63.0 * 0.3; // 0.1 to 0.4 gradient
            frame.set_pixel(x, y, 0, value);
        }
    }

    subtract_background(&mut frame).unwrap();

    // After gradient-only subtraction, the gradient should be flattened
    let left_val = frame.get_pixel(5, 32, 0);
    let right_val = frame.get_pixel(58, 32, 0);

    // Values should be close to each other (gradient removed)
    assert!(
        (left_val - right_val).abs() < 0.15,
        "Gradient not removed: left={}, right={}",
        left_val,
        right_val
    );
}

#[test]
fn test_boundary_hugging_grid() {
    // Verify that the grid nodes span the full image boundaries
    let frame = Frame::filled(256, 256, 1, 0.3).unwrap();
    let config = BackgroundConfig::default().with_grid_size(8, 8);
    let extractor = BackgroundExtractor::new(config);
    let model = extractor.estimate(&frame).unwrap();

    // The model should produce valid background values at all corners
    let tl = model.get_background(0, 0, 0);
    let tr = model.get_background(255, 0, 0);
    let bl = model.get_background(0, 255, 0);
    let br = model.get_background(255, 255, 0);

    for (name, val) in [("TL", tl), ("TR", tr), ("BL", bl), ("BR", br)] {
        assert!(
            (val - 0.3).abs() < 0.02,
            "{} corner should be ~0.3, got {}",
            name,
            val
        );
    }
}

#[test]
fn test_nebulosity_pruning_with_inpaint() {
    // Create a frame with uniform background + bright nebula region
    let mut frame = Frame::filled(256, 256, 1, 0.1).unwrap();

    // Paint a large "nebula" in the center (80x80 bright region)
    for y in 88..168 {
        for x in 88..168 {
            frame.set_pixel(x, y, 0, 0.5);
        }
    }

    let config = BackgroundConfig::default().with_grid_size(12, 12);
    let extractor = BackgroundExtractor::new(config);
    let model = extractor.estimate(&frame).unwrap();

    // Background at the corner should be close to 0.1
    let corner = model.get_background(5, 5, 0);
    assert!(
        (corner - 0.1).abs() < 0.05,
        "Corner should be ~0.1, got {}",
        corner
    );

    // Background in the nebula region should also be close to 0.1
    // because nebula nodes were pruned and inpainted from neighbors
    let center = model.get_background(128, 128, 0);
    assert!(
        (center - 0.1).abs() < 0.1,
        "Center should be ~0.1 (nebula pruned + inpainted), got {}",
        center
    );
}

#[test]
fn test_flat_field_fallback() {
    // Create a frame that is mostly bright (simulating a tightly cropped nebula)
    let frame = Frame::filled(64, 64, 1, 0.8).unwrap();

    // With a uniform bright frame, most/all nodes will have similar high values.
    // The pruning should not crash even if aggressive pruning triggers the fallback.
    let config = BackgroundConfig::default().with_grid_size(4, 4);
    let extractor = BackgroundExtractor::new(config);
    let result = extractor.estimate(&frame);

    assert!(result.is_ok(), "Should not crash on all-bright frame");

    let model = result.unwrap();
    // The model should produce some background value (either original or fallback)
    let bg = model.get_background(32, 32, 0);
    assert!(bg > 0.0, "Background should be positive, got {}", bg);
}

#[test]
fn test_no_double_processing_at_boundaries() {
    // Create a uniform frame and verify node interpolation doesn't create grid-line artifacts
    let mut frame = Frame::filled(128, 128, 1, 0.5).unwrap();

    let config = BackgroundConfig::default()
        .with_grid_size(8, 8)
        .with_gradient_only(false);
    let extractor = BackgroundExtractor::new(config);
    extractor.subtract(&mut frame).unwrap();

    // After subtracting a uniform background, all pixels should be ~0.0
    // If boundaries were processed twice, those pixels would be negative (clamped to 0)
    // while interior pixels would be slightly positive — creating visible lines.
    let data = frame.data();
    let max_val = data.iter().cloned().fold(0.0f32, f32::max);
    let min_val = data.iter().cloned().fold(f32::MAX, f32::min);

    // All values should be very close to each other (uniform after uniform subtraction)
    assert!(
        max_val - min_val < 0.02,
        "Non-uniform result suggests double-processing: min={}, max={}",
        min_val,
        max_val
    );
}

/// The dynamic pedestal must be derived from the centre crop of a single plane.
///
/// The interleaved indexing that survived the planar migration stayed silent for
/// three-channel frames — `y * w * 3` lands inside the green plane by accident — so it
/// never scrambled channels. It did sample the wrong *rows*: roughly the middle 75% of
/// frame height at a 3-pixel stride wrapping across row ends, instead of the centre
/// 25% x 25% crop. This fixture separates the two by making green flat inside the crop
/// and noisy outside it, reading the pedestal back through its only observable effect.
#[test]
fn pedestal_is_sampled_from_the_centre_crop_of_one_plane() {
    let (w, h) = (256usize, 256usize);
    let crop_w = (w / 4).max(10);
    let crop_h = (h / 4).max(10);
    let x0 = (w - crop_w) / 2;
    let y0 = (h - crop_h) / 2;

    let mut frame = Frame::zeros(w, h, 3).unwrap();
    let mut seed = 12345u32;
    for y in 0..h {
        for x in 0..w {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let noise = (seed >> 16) as f32 / 65536.0;
            let in_crop = (x0..x0 + crop_w).contains(&x) && (y0..y0 + crop_h).contains(&y);
            frame.set_pixel(x, y, 0, noise);
            frame.set_pixel(x, y, 1, if in_crop { 0.25 } else { noise });
            frame.set_pixel(x, y, 2, noise);
        }
    }

    // A zero background at zero aggressiveness leaves the pedestal as the only thing
    // `subtract_from` can change, which makes it directly observable.
    let model = BackgroundModel::new(vec![vec![0.0f32; 16]; 3], 4, 4, w, h, 3, false, 0.1, 0.0);
    let before = frame.get_pixel(x0, y0, 1);
    let mut subtracted = frame.clone();
    model.subtract_from(&mut subtracted);
    let pedestal = subtracted.get_pixel(x0, y0, 1) - before;

    assert!(
        (pedestal - 0.001).abs() < 1e-6,
        "pedestal {pedestal} — a flat crop has zero MAD, so this must clamp to the \
         0.001 floor; the interleaved version reported ~0.975"
    );
}

/// A dense grid (RBF's) is read by bilinear weights between cell centres: `subtract_from`
/// must agree everywhere with that formula, written out here independently.
///
/// Exercised on a **portrait** grid, the case that used to diverge: `subtract_weight_based`
/// clamped the grid *row* index against `grid_width - 1`. That's invisible while the grid
/// is square (extractor's 12x12/16x16 defaults) or wider than tall (RBF's
/// `eval_height = 256 * h / w` on landscape), and wrong once rows outnumber columns —
/// every row past `grid_width - 1` collapsed onto that one.
#[test]
fn weight_based_subtraction_agrees_with_get_background_on_a_portrait_grid() {
    let (w, h) = (16usize, 32usize);
    let (grid_cols, grid_rows) = (4usize, 8usize);

    // Varies along the grid's y axis, so a mis-clamped row index changes the answer.
    let grid: Vec<Vec<f32>> = (0..3)
        .map(|c| {
            (0..grid_rows * grid_cols)
                .map(|i| 0.02 + (i / grid_cols) as f32 * 0.01 + c as f32 * 0.002)
                .collect()
        })
        .collect();

    // No node coordinates, so this takes the weight-based path (as RBF does).
    let model = BackgroundModel::new(grid, grid_cols, grid_rows, w, h, 3, false, 0.1, 1.0);

    // A constant frame has zero MAD, so the pedestal is its 0.001 floor and the
    // output is a pure readout of the interpolated background.
    let mut frame = Frame::filled(w, h, 3, 1.0).unwrap();
    model.subtract_from(&mut frame);

    let cell_centres = |x: usize, y: usize, c: usize| {
        let axis = |p: usize, pixels: usize, cells: usize| {
            let g = (p as f32 + 0.5) * cells as f32 / pixels as f32 - 0.5;
            let g0 = (g.floor() as isize).clamp(0, cells as isize - 1) as usize;
            (g0, (g0 + 1).min(cells - 1), (g - g0 as f32).clamp(0.0, 1.0))
        };
        let ((x0, x1, fx), (y0, y1, fy)) = (axis(x, w, grid_cols), axis(y, h, grid_rows));
        let g = model.grid_values(c);
        let v0 = g[y0 * grid_cols + x0] * (1.0 - fx) + g[y0 * grid_cols + x1] * fx;
        let v1 = g[y1 * grid_cols + x0] * (1.0 - fx) + g[y1 * grid_cols + x1] * fx;
        v0 * (1.0 - fy) + v1 * fy
    };
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let want = 1.0 - cell_centres(x, y, c) + 0.001;
                let got = frame.get_pixel(x, y, c);
                assert!(
                    (got - want).abs() < 1e-5,
                    "({x}, {y}, {c}): subtract_from gave {got}, the cell-centre formula {want}"
                );
                assert!((model.get_background(x, y, c) - cell_centres(x, y, c)).abs() < 1e-6, "get_background ({x}, {y}, {c})");
            }
        }
    }
}

/// The node path interpolates between *node coordinates*. `get_background` reads the same
/// rows `subtract_from` renders, so it can't serve as reference. The pin is analytic: at a node the background is that node's value exactly, and halfway between
/// two on a linear grid it's their mean (a monotone cubic reproduces a linear run).
#[test]
fn node_interpolation_reproduces_the_node_grid() {
    let (w, h) = (65usize, 33usize);
    let (grid_cols, grid_rows) = (3usize, 3usize);

    // Boundary-hugging, exactly as `initialize_grid` builds them.
    let nodes_x: Vec<usize> = (0..grid_cols)
        .map(|i| i * (w - 1) / (grid_cols - 1))
        .collect();
    let nodes_y: Vec<usize> = (0..grid_rows)
        .map(|j| j * (h - 1) / (grid_rows - 1))
        .collect();

    // Distinct per node and per channel, so a transposed or mis-strided read shows up.
    let grid: Vec<Vec<f32>> = (0..3)
        .map(|c| {
            (0..grid_rows * grid_cols)
                .map(|i| 0.05 + i as f32 * 0.01 + c as f32 * 0.003)
                .collect()
        })
        .collect();

    let model = BackgroundModel::with_node_coords(
        grid.clone(),
        grid_cols,
        grid_rows,
        w,
        h,
        3,
        false,
        0.1,
        1.0,
        nodes_x.clone(),
        nodes_y.clone(),
    );

    // A constant frame makes the output a direct readout of the interpolation: MAD
    // is zero, so the pedestal is its 0.001 floor.
    let mut frame = Frame::filled(w, h, 3, 1.0).unwrap();
    model.subtract_from(&mut frame);
    let background_at = |x: usize, y: usize, c: usize| 1.0 + 0.001 - frame.get_pixel(x, y, c);

    for (c, plane) in grid.iter().enumerate() {
        for (jy, &y) in nodes_y.iter().enumerate() {
            for (ix, &x) in nodes_x.iter().enumerate() {
                let want = plane[jy * grid_cols + ix];
                let got = background_at(x, y, c);
                assert!(
                    (got - want).abs() < 1e-5,
                    "node ({ix}, {jy}) channel {c}: background {got}, node value {want}"
                );
            }
        }

        // Midway along the top edge: the mean of the two nodes either side.
        let mx = (nodes_x[0] + nodes_x[1]) / 2;
        let want = (grid[c][0] + grid[c][1]) / 2.0;
        let got = background_at(mx, 0, c);
        assert!(
            (got - want).abs() < 2e-3,
            "midpoint ({mx}, 0) channel {c}: background {got}, expected ~{want}"
        );

        // And midway down the left edge.
        let my = (nodes_y[0] + nodes_y[1]) / 2;
        let want = (grid[c][0] + grid[c][grid_cols]) / 2.0;
        let got = background_at(0, my, c);
        assert!(
            (got - want).abs() < 2e-3,
            "midpoint (0, {my}) channel {c}: background {got}, expected ~{want}"
        );
    }
}

/// A frame the model was not built for must be refused rather than half-corrected:
/// the weight-based path recovers the channel index from the model's stored height.
#[test]
fn subtract_from_leaves_a_mismatched_frame_untouched() {
    let model =
        BackgroundModel::new(vec![vec![0.5f32; 16]; 3], 4, 4, 64, 64, 3, false, 0.1, 1.0);

    let mut frame = Frame::filled(32, 32, 3, 0.4).unwrap();
    model.subtract_from(&mut frame);

    assert!(
        frame.data().iter().all(|&v| (v - 0.4).abs() < 1e-6),
        "a mismatched frame must be left alone"
    );
}

/// The 2026-10-24 M57 sky: a band peaking between node lines, near the frame edge. Joined
/// by straight lines the model cut its peak and shoulder and left 0.6 sigma on screen; the
/// monotone cubic through the same nodes must follow it markedly closer.
#[test]
fn a_band_peaking_between_node_lines_is_followed() {
    let (w, h) = (1432usize, 4usize);
    let xs = node_axis(w, 12, 0.035);
    let band = |x: usize| (-((x as f32 / (w - 1) as f32 - 0.89) / 0.1).powi(2)).exp();
    let nodes: Vec<f32> = xs.iter().map(|&x| band(x)).collect();
    let model = BackgroundModel::with_node_coords(vec![nodes.repeat(2)], xs.len(), 2, w, h, 1, false, 0.1, 1.0, xs.clone(), vec![0, h - 1]);
    let mut frame = Frame::filled(w, h, 1, 2.0).unwrap();
    model.subtract_from(&mut frame);

    let cubic: Vec<f32> = (0..w).map(|x| 2.0 + 0.001 - frame.get_pixel(x, 1, 0)).collect();
    let chord: Vec<f32> = (0..w)
        .map(|x| {
            let k = xs.windows(2).position(|p| x <= p[1]).unwrap();
            let t = (x - xs[k]) as f32 / (xs[k + 1] - xs[k]) as f32;
            nodes[k] + (nodes[k + 1] - nodes[k]) * t
        })
        .collect();
    let error = |model: &[f32]| {
        let e: Vec<f32> = model.iter().enumerate().map(|(x, m)| m - band(x)).collect();
        let worst = e.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        (worst, (e.iter().map(|v| v * v).sum::<f32>() / w as f32).sqrt())
    };
    let ((cubic_worst, cubic_rms), (chord_worst, chord_rms)) = (error(&cubic), error(&chord));
    assert!(
        cubic_worst < 0.7 * chord_worst && cubic_rms < 0.7 * chord_rms,
        "cubic worst {cubic_worst:.3} rms {cubic_rms:.4}, straight lines {chord_worst:.3} {chord_rms:.4}"
    );
}

/// A stack's estimate measures the edges' profiles; a single sub's, which live view takes
/// every frame, does not.
#[test]
fn edge_profiles_follow_the_config() {
    let frame = Frame::filled(512, 512, 1, 0.1).unwrap();
    let with = BackgroundExtractor::with_defaults().estimate(&frame).unwrap();
    assert!(with.edges().is_some());
    let config = BackgroundConfig::default().with_edge_profiles(false);
    let without = BackgroundExtractor::new(config).estimate(&frame).unwrap();
    assert!(without.edges().is_none());
}

/// A catalogued filament along one edge (a Veil arc, a rim of IC 1396) is protected from the
/// node grid, so the edge profiles must not take back what protection kept: it runs the
/// edge's whole length, which is all they ask of an instrument band.
#[test]
fn edge_profiles_respect_catalog_protection() {
    use crate::background::protection::{FieldProtection, ProtectedRegion};
    let size = 1024usize;
    let sigma = 1e-4f32;
    let jitter = |i: usize| {
        let h = (i as u32).wrapping_mul(2_654_435_761).rotate_left(13).wrapping_mul(2_246_822_519);
        (h >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    };
    // 15 sigma, at 0.9 of the width, 0.03 of it wide, the frame's whole height.
    let nebula = |x: f32, _: f32| 15.0 * sigma * (-((x - 0.9) / 0.03).powi(2) / 2.0).exp();
    let mut frame = Frame::zeros(size, size, 1).unwrap();
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 / (size - 1) as f32, y as f32 / (size - 1) as f32);
            frame.set_pixel(x, y, 0, 0.05 + nebula(fx, fy) + sigma * jitter(y * size + x));
        }
    }
    let s = size as f32;
    let region = ProtectedRegion { cx: 0.9 * s, cy: 0.5 * s, semi_major: 0.6 * s, semi_minor: 0.07 * s, angle: std::f32::consts::FRAC_PI_2 };
    let protection = std::sync::Arc::new(FieldProtection { generation: 1, width: size, height: size, regions: vec![region] });
    let config = BackgroundConfig::default().with_protection(Some(protection));
    let background = |edges: bool| {
        let model = BackgroundExtractor::new(config.clone().with_edge_profiles(edges)).estimate(&frame).unwrap();
        model.to_frame().unwrap()
    };
    let (with, without) = (background(true), background(false));
    let x = (0.9 * (size - 1) as f32) as usize;
    let grid_lift = (without.get_pixel(x, size / 2, 0) - without.get_pixel(size / 2, size / 2, 0)) / sigma;
    assert!(grid_lift < 3.0, "the protected grid itself followed {grid_lift:.1} sigma of the filament");
    let taken = (0..size)
        .flat_map(|y| (size * 3 / 4..size).map(move |x| (x, y)))
        .map(|(x, y)| (with.get_pixel(x, y, 0) - without.get_pixel(x, y, 0)) / sigma)
        .fold(0.0f32, f32::max);
    assert!(taken < 1.0, "the edge profiles took {taken:.1} sigma of a protected 15 sigma filament");
}

/// The edge profiles read the model through its sampler, which must be the background
/// `subtract_from` removes, pixel for pixel: node models and RBF's dense grid alike, with
/// the edge profiles on top.
#[test]
fn the_sampler_reads_the_rendered_background() {
    let (w, h) = (300usize, 180usize);
    let mut frame = Frame::zeros(w, h, 2).unwrap();
    for c in 0..2 {
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
                let band = 0.01 * (-((fx - 0.92) / 0.03).powi(2)).exp();
                frame.set_pixel(x, y, c, 0.1 + 0.05 * fx * fy + 0.02 * (3.0 * fx).sin() + band + 0.001 * c as f32);
            }
        }
    }
    let nodes = BackgroundExtractor::with_defaults().estimate(&frame).unwrap();
    assert!(nodes.edges().is_some_and(|e| e.peak() > 0.0), "a model with edge profiles to read");
    let grid: Vec<Vec<f32>> = (0..2).map(|c| (0..16 * 9).map(|i| 0.1 + 0.001 * (i * (c + 3) % 17) as f32).collect()).collect();
    let dense = BackgroundModel::new(grid, 16, 9, w, h, 2, false, 0.1, 1.0);
    for (name, model) in [("nodes", nodes), ("dense", dense)] {
        let ys: Vec<usize> = (0..h).step_by(7).chain([h - 1]).collect();
        for c in 0..2 {
            let mut plane = vec![0.0; w * h];
            model.render_plane(c, &mut plane);
            let sampler = model.sampler(c, &ys);
            for (slot, &y) in ys.iter().enumerate() {
                for x in 0..w {
                    let (got, want) = (sampler.at(x, slot), plane[y * w + x]);
                    assert!((got - want).abs() < 1e-6, "{name} c{c} ({x}, {y}): sampled {got}, rendered {want}");
                }
            }
        }
    }
}
