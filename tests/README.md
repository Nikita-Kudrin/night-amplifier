# Test Fixtures Directory

Place your astronomical test images here for integration testing.

## Supported Formats

- **TIFF**: `.tif`, `.tiff` (8-bit and 16-bit, grayscale or RGB)
- **FITS**: `.fit`, `.fits` (standard astronomy format)
- **Raw Bayer**: Single-channel CFA data (RGGB, BGGR, GRBG, GBRG patterns)

## Requirements for Stacking Tests

- At least 2 images for stacking tests
- All images should have the same dimensions
- Images should contain detectable stars (>3 stars recommended)
- Preferably sequential frames of the same target

## Recommended Test Data

Good test data includes:

- Light frames from an astronomy camera (ASI, ZWO, etc.)
- Sub-exposures of the same deep sky object
- Images with visible stars for registration testing

## Example Directory Structure

```
tests/fixtures/
├── README.md
├── 20-12-2026-m31
├──├── m31_001.tiff
├──├── m31_003.tiff
├── 10-11-2026-orion
├──├── orion_005.tiff
├──├── orion_008.tiff
```

## Notes

- Tests will skip gracefully if no fixtures are present
- Large files can significantly impact test duration
