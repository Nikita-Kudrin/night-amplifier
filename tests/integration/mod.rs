//! Integration tests for the complete stacking and stretching pipeline, run on real TIFF or
//! FITS frames under `tests/fixtures/`: one subdirectory per session, at least 2 frames each,
//! whose name names its output in `tests/fixtures/processed/` (gitignored).
//!
//! ```text
//! tests/fixtures/
//! ├── processed/
//! │   └── session_001.tiff
//! └── session_001/
//!     ├── frame_00000.tiff
//!     └── frame_00001.tiff
//! ```
//!
//! # Module Organization
//!
//! - `common` - Shared constants, types, and fixture discovery utilities
//! - `fixture_contents_tests` - A fixture counts as present only if it holds what its archive does
//! - `image_loading` - TIFF and FITS file loading, frame saving
//! - `prefetch` - Parallel frame prefetching for optimized loading
//! - `cfa_tests` - Raw-CFA stage: hot pixels, row/column FPN, superpixel debayer
//! - `debayer_tests` - Tests for Bayer pattern detection and debayering
//! - `stacking_tests` - Tests for the complete stacking pipeline
//! - `stretch_tests` - Tests for auto-stretch and rendering
//! - `detection_tests` - Tests for star detection on real images
//! - `display_output_tests` - Black floor, stream resolution and both denoisers, in output levels
//! - `dither_tests` - The blue-noise dither against the Bayer matrix: lattice lines, what JPEG keeps
//! - `instruments` - The measurement instruments, shared with Pro by `#[path]`: no `crate::` there
//! - `replay_cache_tests` - Replays shared across a run: bit for bit, keyed on their inputs, made once
//! - `render_brightness_tests` - The brightness-against-grain trade, on the instruments that judge it
//! - `auto_crop_tests` - `auto_crop` on real sessions: area kept, crop motion, the cut before stretch
//! - `early_subs_tests` - Bad first subs, streamed against the stack's probation and an oracle
//! - `planetary` / `planetary_tests` - 2026-10-24's drifting Moon from its SER: shifts and sharpness
//! - `bad_subs_scan` - Diagnostic: every local session's subs tabulated, to find bad ones
//! - `fixture_processing` - Long-running tests that process complete fixture sets

pub mod auto_crop_tests;
pub mod background_tests;
pub mod bad_subs_scan;
pub mod cfa_tests;
pub mod common;
pub mod debayer_tests;
pub mod detection_tests;
pub mod display_output_tests;
pub mod dither_tests;
pub mod early_subs;
pub mod early_subs_tests;
pub mod encoding_tests;
pub mod fixture_contents_tests;
pub mod fixture_processing;
pub mod image_loading;
pub mod instruments;
pub mod planetary;
pub mod planetary_tests;
pub mod prefetch;
pub mod render_brightness_tests;
pub mod replay_cache_tests;
pub mod sky_estimate_tests;
pub mod stack_depth_grain_tests;
pub mod stack_quality_tests;
pub mod stacking_tests;
pub mod stretch_tests;
pub mod temporal_stability_tests;
