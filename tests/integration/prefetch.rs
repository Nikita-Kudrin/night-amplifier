//! Frame prefetcher implementations for parallel loading pipeline.
//!
//! Provides background loading and debayering of frames to maximize throughput.

#![allow(dead_code)]

use std::sync::mpsc;
use std::thread;

use night_amplifier::{CfaPattern, DebayerAlgorithm, DebayerConfig, Frame};

use crate::integration::common::LoadedImage;

// ============================================================================
// Prefetched Frame
// ============================================================================

/// A prefetched frame ready for processing
pub struct PrefetchedFrame {
    pub frame: Frame,
    pub index: usize,
}

// ============================================================================
// Sequential Frame Prefetcher
// ============================================================================

/// Frame prefetcher that loads and debayers images in background threads.
/// Uses a bounded channel to limit memory usage to ~num_cpus frames.
#[allow(dead_code)]
pub struct FramePrefetcher {
    receiver: mpsc::Receiver<Option<PrefetchedFrame>>,
    _workers: Vec<thread::JoinHandle<()>>,
}

impl FramePrefetcher {
    /// Creates a new prefetcher that loads frames in parallel.
    ///
    /// # Arguments
    /// * `images` - Slice of loaded images (raw Bayer data)
    /// * `pattern` - Detected CFA pattern for debayering
    /// * `start_index` - Index to start from (skip reference frame)
    /// * `prefetch_count` - Number of frames to prefetch (typically num_cpus)
    #[allow(dead_code)]
    pub fn new(
        images: &[LoadedImage],
        pattern: Option<CfaPattern>,
        start_index: usize,
        prefetch_count: usize,
    ) -> Self {
        // Bounded channel limits memory to prefetch_count frames
        let (tx, rx) = mpsc::sync_channel::<Option<PrefetchedFrame>>(prefetch_count);

        // Clone image data for the worker thread
        // We only clone the paths and raw frames needed
        let image_data: Vec<(usize, Frame, bool)> = images[start_index..]
            .iter()
            .enumerate()
            .map(|(i, img)| (start_index + i, img.frame.clone(), img.is_bayer))
            .collect();

        // Spawn worker thread that loads frames sequentially
        // (we use sync_channel's backpressure to limit parallelism)
        let worker = thread::spawn(move || {
            for (index, raw_frame, is_bayer) in image_data {
                let frame = if is_bayer && raw_frame.channels() == 1 {
                    let config = pattern
                        .map(|p| DebayerConfig::new(p).with_algorithm(DebayerAlgorithm::Bilinear))
                        .unwrap_or_else(|| {
                            DebayerConfig::new(CfaPattern::Rggb)
                                .with_algorithm(DebayerAlgorithm::Bilinear)
                        });
                    night_amplifier::debayer_with_config(&raw_frame, config).ok()
                } else {
                    Some(raw_frame)
                };

                if let Some(f) = frame {
                    // This blocks if channel is full (backpressure)
                    if tx.send(Some(PrefetchedFrame { frame: f, index })).is_err() {
                        break; // Receiver dropped
                    }
                }
            }
            // Signal completion
            let _ = tx.send(None);
        });

        Self {
            receiver: rx,
            _workers: vec![worker],
        }
    }

    /// Gets the next prefetched frame, blocking if not yet ready.
    /// Returns None when all frames have been processed.
    #[allow(dead_code)]
    pub fn next(&self) -> Option<PrefetchedFrame> {
        match self.receiver.recv() {
            Ok(Some(frame)) => Some(frame),
            Ok(None) | Err(_) => None,
        }
    }
}

// ============================================================================
// Parallel Frame Prefetcher
// ============================================================================

/// Parallel frame prefetcher using rayon thread pool for maximum throughput.
/// Loads multiple frames simultaneously using all available CPU cores.
pub struct ParallelPrefetcher {
    receiver: mpsc::Receiver<Option<PrefetchedFrame>>,
    _worker: thread::JoinHandle<()>,
}

impl ParallelPrefetcher {
    /// Creates a parallel prefetcher that loads frames using rayon.
    pub fn new(
        images: &[LoadedImage],
        pattern: Option<CfaPattern>,
        start_index: usize,
        buffer_size: usize,
    ) -> Self {
        use rayon::prelude::*;

        let (tx, rx) = mpsc::sync_channel::<Option<PrefetchedFrame>>(buffer_size);

        // Clone image data for processing
        let image_data: Vec<(usize, Frame, bool)> = images[start_index..]
            .iter()
            .enumerate()
            .map(|(i, img)| (start_index + i, img.frame.clone(), img.is_bayer))
            .collect();

        let worker = thread::spawn(move || {
            // Process frames in parallel chunks
            // Each chunk is processed by rayon, results sent through channel
            let chunk_size = buffer_size.max(4);

            for chunk in image_data.chunks(chunk_size) {
                // Process chunk in parallel
                let results: Vec<_> = chunk
                    .par_iter()
                    .filter_map(|(index, raw_frame, is_bayer)| {
                        let frame = if *is_bayer && raw_frame.channels() == 1 {
                            let config = pattern
                                .map(|p| {
                                    DebayerConfig::new(p).with_algorithm(DebayerAlgorithm::Bilinear)
                                })
                                .unwrap_or_else(|| {
                                    DebayerConfig::new(CfaPattern::Rggb)
                                        .with_algorithm(DebayerAlgorithm::Bilinear)
                                });
                            night_amplifier::debayer_with_config(raw_frame, config).ok()
                        } else {
                            Some(raw_frame.clone())
                        };

                        frame.map(|f| PrefetchedFrame {
                            frame: f,
                            index: *index,
                        })
                    })
                    .collect();

                // Send results in order
                for prefetched in results {
                    if tx.send(Some(prefetched)).is_err() {
                        return; // Receiver dropped
                    }
                }
            }
            let _ = tx.send(None);
        });

        Self {
            receiver: rx,
            _worker: worker,
        }
    }

    pub fn next(&self) -> Option<PrefetchedFrame> {
        match self.receiver.recv() {
            Ok(Some(frame)) => Some(frame),
            Ok(None) | Err(_) => None,
        }
    }
}
