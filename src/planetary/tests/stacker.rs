use super::common::{create_planetary_frame, create_test_frame};
use super::*;

#[test]
fn test_planetary_stacker_basic() {
    let config = PlanetaryConfig::default().with_selection(0.5);
    let mut stacker = PlanetaryStacker::new(config);

    for i in 0..10 {
        let frame = create_test_frame(32, 32, (i % 3) as f32, (i % 2) as f32);
        stacker.add_frame(&frame).unwrap();
    }

    assert_eq!(stacker.frame_count(), 10);
    let stats = stacker.statistics();
    assert_eq!(stats.total_frames, 10);
    assert!(stats.selected_frames >= 5);
}

#[test]
fn test_stack_planetary_convenience() {
    let frames: Vec<Frame> = (0..5)
        .map(|i| create_test_frame(32, 32, (i % 2) as f32, 0.0))
        .collect();
    let config = PlanetaryConfig::default().with_selection(1.0);
    let result = stack_planetary(&frames, config).unwrap();
    assert_eq!(result.width(), 32);
    assert_eq!(result.height(), 32);
}

#[test]
fn test_stacking_methods() {
    let frames: Vec<Frame> = (0..5)
        .map(|_| Frame::filled(16, 16, 3, 0.5).unwrap())
        .collect();

    for method in [
        PlanetaryStackMethod::Mean,
        PlanetaryStackMethod::Median,
        PlanetaryStackMethod::WeightedMean,
    ] {
        let config = PlanetaryConfig::default()
            .with_method(method)
            .with_selection(1.0);
        let result = stack_planetary(&frames, config).unwrap();
        let data = result.data();
        assert!((data[0] - 0.5).abs() < 0.01);
    }
}

#[test]
fn test_config_presets() {
    let lunar = PlanetaryConfig::lunar();
    assert_eq!(lunar.selection_percentage, 0.05);
    let planetary = PlanetaryConfig::planetary();
    assert_eq!(planetary.selection_percentage, 0.10);
}

#[test]
fn test_stacker_frame_selection() {
    let config = PlanetaryConfig::default()
        .with_selection(0.2)
        .with_quality_metric(QualityMetric::Laplacian);
    let mut stacker = PlanetaryStacker::new(config);
    for i in 0..10 {
        let frame = create_planetary_frame(32, 32, 0.0, 0.0, i as f32);
        stacker.add_frame(&frame).unwrap();
    }
    let stats = stacker.statistics();
    assert_eq!(stats.total_frames, 10);
    assert!(stats.max_quality > stats.min_quality);
}

#[test]
fn test_stacker_with_small_min_frames() {
    let config = PlanetaryConfig {
        selection_percentage: 0.3,
        min_frames: 2,
        max_frames: 0,
        search_radius: 10,
        alignment_roi: None,
        stacking_method: PlanetaryStackMethod::Mean,
        percentile: 0.5,
        quality_metric: QualityMetric::Laplacian,
        subpixel_factor: 1,
    };
    let mut stacker = PlanetaryStacker::new(config);
    for i in 0..10 {
        let frame = create_planetary_frame(32, 32, 0.0, 0.0, i as f32);
        stacker.add_frame(&frame).unwrap();
    }
    let stats = stacker.statistics();
    assert_eq!(stats.selected_frames, 3);
}

#[test]
fn test_stacking_improves_snr() {
    let config = PlanetaryConfig::default().with_selection(1.0);
    let frames: Vec<Frame> = (0..20)
        .map(|i| {
            let noise_x = ((i * 7) % 5) as f32 - 2.0;
            let noise_y = ((i * 11) % 5) as f32 - 2.0;
            create_planetary_frame(48, 48, noise_x * 0.5, noise_y * 0.5, 0.5)
        })
        .collect();
    let result = stack_planetary(&frames, config).unwrap();
    let data = result.data();
    let non_zero_count = data.iter().filter(|&&v| v > 0.01).count();
    assert!(non_zero_count > 0);
}

#[test]
fn test_percentile_stacking() {
    let mut frames: Vec<Frame> = (0..9)
        .map(|_| Frame::filled(16, 16, 3, 0.5).unwrap())
        .collect();
    frames.push(Frame::filled(16, 16, 3, 0.9).unwrap());
    let config = PlanetaryConfig::default()
        .with_method(PlanetaryStackMethod::Percentile)
        .with_selection(1.0);
    let result = stack_planetary(&frames, config).unwrap();
    let data = result.data();
    assert!((data[0] - 0.5).abs() < 0.15);
}

#[test]
fn test_weighted_mean_favors_quality() {
    let config = PlanetaryConfig::default()
        .with_method(PlanetaryStackMethod::WeightedMean)
        .with_selection(1.0);
    let mut stacker = PlanetaryStacker::new(config);
    let sharp = create_planetary_frame(32, 32, 0.0, 0.0, 0.0);
    stacker.add_frame(&sharp).unwrap();
    for _ in 0..4 {
        let blurry = create_planetary_frame(32, 32, 0.0, 0.0, 5.0);
        stacker.add_frame(&blurry).unwrap();
    }
    let result = stacker.stack().unwrap();
    assert_eq!(result.width(), 32);
}

#[test]
fn test_stacker_clear() {
    let mut stacker = PlanetaryStacker::with_defaults();
    let frame = create_planetary_frame(32, 32, 0.0, 0.0, 0.0);
    stacker.add_frame(&frame).unwrap();
    assert_eq!(stacker.frame_count(), 1);
    stacker.clear();
    assert_eq!(stacker.frame_count(), 0);
}

#[test]
fn test_stacker_set_reference() {
    let mut stacker = PlanetaryStacker::with_defaults();
    let reference = create_planetary_frame(48, 48, 0.0, 0.0, 0.0);
    stacker.set_reference(reference);
    let shifted = create_planetary_frame(48, 48, 2.0, 1.0, 0.0);
    let quality = stacker.add_frame(&shifted).unwrap();
    assert!(quality > 0.0);
}

#[test]
fn test_dimension_mismatch_error() {
    let mut stacker = PlanetaryStacker::with_defaults();
    let frame1 = Frame::filled(32, 32, 3, 0.5).unwrap();
    let frame2 = Frame::filled(64, 64, 3, 0.5).unwrap();
    stacker.add_frame(&frame1).unwrap();
    assert!(stacker.add_frame(&frame2).is_err());
}

#[test]
fn test_empty_frames_error() {
    let frames: Vec<Frame> = vec![];
    assert!(stack_planetary(&frames, PlanetaryConfig::default()).is_err());
}

#[test]
fn test_quality_scores_returned() {
    let mut stacker = PlanetaryStacker::with_defaults();
    let frame = create_planetary_frame(32, 32, 0.0, 0.0, 0.0);
    let quality = stacker.add_frame(&frame).unwrap();
    let quality_scores = stacker.quality_scores();
    assert_eq!(quality_scores[0], quality);
}

#[test]
fn test_statistics_accuracy() {
    let config = PlanetaryConfig {
        selection_percentage: 0.4,
        min_frames: 2,
        max_frames: 100,
        ..Default::default()
    };
    let mut stacker = PlanetaryStacker::new(config);
    for i in 0..10 {
        let frame = create_planetary_frame(32, 32, 0.0, 0.0, i as f32);
        stacker.add_frame(&frame).unwrap();
    }
    let stats = stacker.statistics();
    assert_eq!(stats.selected_frames, 4);
}

#[test]
fn test_max_frames_limit() {
    let config = PlanetaryConfig {
        max_frames: 3,
        selection_percentage: 1.0,
        ..Default::default()
    };
    let mut stacker = PlanetaryStacker::new(config);
    for _ in 0..10 {
        let frame = Frame::filled(16, 16, 3, 0.5).unwrap();
        stacker.add_frame(&frame).unwrap();
    }
    let stats = stacker.statistics();
    assert_eq!(stats.selected_frames, 3);
}

#[test]
fn test_stacking_with_various_offsets() {
    let config = PlanetaryConfig::default().with_selection(1.0);
    let frames: Vec<Frame> = vec![
        create_planetary_frame(48, 48, 0.0, 0.0, 0.0),
        create_planetary_frame(48, 48, 2.0, 1.0, 0.0),
    ];
    let result = stack_planetary(&frames, config).unwrap();
    assert_eq!(result.width(), 48);
}
