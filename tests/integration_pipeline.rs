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
//! - `debayer_tests` - Tests for Bayer pattern detection and debayering
//! - `stacking_tests` - Tests for the complete stacking pipeline
//! - `stretch_tests` - Tests for auto-stretch and rendering
//! - `detection_tests` - Tests for star detection on real images
//! - `fixture_processing` - Long-running tests that process complete fixture sets

mod integration;
mod encoding_tests {
    include!("integration/encoding_tests.rs");
}
