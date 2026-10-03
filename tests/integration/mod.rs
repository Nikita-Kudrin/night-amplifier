//! Integration tests for the complete stacking and stretching pipeline.
//!
//! These tests read actual TIFF or FITS files from the `tests/fixtures/` directory
//! and run them through the full processing pipeline.
//!
//! # Test Data Setup
//!
//! Place your test images in subdirectories under `tests/fixtures/`:
//! - Each subdirectory should contain TIFF files (`*.tif` or `*.tiff`) or FITS files (`*.fit` or `*.fits`)
//! - Subdirectory names will be used as output filenames
//!
//! The test expects at least 2 frames for stacking per subdirectory.
//! Processed results are saved to `tests/fixtures/processed/`.
//!
//! # Example Directory Structure
//! ```text
//! tests/
//! └── fixtures/
//!     ├── README.md
//!     ├── processed/          <- Output directory (gitignored)
//!     │   ├── session_001.tiff
//!     │   └── session_002.tiff
//!     ├── session_001/
//!     │   ├── frame_00000.tiff
//!     │   └── frame_00001.tiff
//!     └── session_002/
//!         ├── frame_00000.tiff
//!         └── frame_00001.tiff
//! ```
//!
//! # Module Organization
//!
//! - `common` - Shared constants, types, and fixture discovery utilities
//! - `image_loading` - TIFF and FITS file loading, frame saving
//! - `prefetch` - Parallel frame prefetching for optimized loading
//! - `cfa_tests` - Raw-CFA stage: hot pixels, row/column FPN, superpixel debayer
//! - `debayer_tests` - Tests for Bayer pattern detection and debayering
//! - `stacking_tests` - Tests for the complete stacking pipeline
//! - `stretch_tests` - Tests for auto-stretch and rendering
//! - `detection_tests` - Tests for star detection on real images
//! - `display_output_tests` - Black floor, stream resolution and the two denoisers,
//!   measured in output levels on real fixtures
//! - `dither_tests` - The blue-noise dither, judged by the lattice lines it leaves (none)
//!   against the Bayer matrix it replaced, and by what the stream's JPEG keeps of it
//! - `instruments` - the six measurement instruments, shared with the Pro repo by
//!   `#[path]` inclusion: octave-band sky noise, the star radial profile, a centre/edge
//!   split, line coherence, lattice lines and block-mean error. **No `crate::` paths may
//!   appear there.**
//! - `render_brightness_tests` - Octave-band sky noise, the star radial profile and
//!   object brightness at three radii: the instruments any change to the
//!   brightness-against-grain trade has to be judged on
//! - `fixture_processing` - Long-running tests that process complete fixture sets

pub mod background_tests;
pub mod cfa_tests;
pub mod common;
pub mod debayer_tests;
pub mod detection_tests;
pub mod display_output_tests;
pub mod dither_tests;
pub mod encoding_tests;
pub mod fixture_processing;
pub mod image_loading;
pub mod instruments;
pub mod prefetch;
pub mod render_brightness_tests;
pub mod sky_estimate_tests;
pub mod stack_depth_grain_tests;
pub mod stacking_tests;
pub mod stretch_tests;
pub mod temporal_stability_tests;
