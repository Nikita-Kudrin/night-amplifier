# Benchmark Plan: Jpegli vs JPEG vs TurboJPEG

The goal of this plan is to implement a benchmark suite to compare the performance and compression ratios of `jpegli`, `image::codecs::jpeg` (usual jpeg), and `turbojpeg`. 

## User Review Required

> [!IMPORTANT]
> The `jpegli` library can be integrated in a few different ways in Rust (e.g. `jpegli` crate which is a C wrapper, or `jpegli-rs` crate which is a pure Rust port). I propose using `jpegli` (wrapper) or `jpegli-rs`. Please confirm which crate you prefer.
>
> I propose creating a standalone binary `src/bin/jpeg_benchmark.rs` rather than extending `benches/encoding_benchmark.rs` with `criterion`. Criterion does not easily support printing custom markdown tables with custom columns (speed, file size, fps). A custom binary allows us to format the output exactly as requested and run the benchmark directly.

## Open Questions

1. **Jpegli Crate:** Which `jpegli` crate should be used? `jpegli` (C wrapper) or `jpegli-rs` (pure Rust port)?
2. **Fixture Frame:** Since the specified fixtures are directories containing multiple frames (e.g. `frame_00000.tiff`), I plan to pick the first frame from each directory to run the benchmarks on. Is this acceptable?
3. **Execution:** Should this benchmark be an independent cargo binary (e.g., `cargo run --bin jpeg_benchmark --release`) or a test/script?

## Proposed Changes

### Configuration

#### [MODIFY] Cargo.toml
- Add the chosen `jpegli` crate to the `[dependencies]` or `[dev-dependencies]`.
- Add a new `[[bin]]` section for `jpeg_benchmark` if the standalone binary approach is approved.

### Benchmarking Logic

#### [NEW] src/bin/jpeg_benchmark.rs
This binary will perform the following steps:
1. **Load Fixtures:** Load one frame from each of the following test fixture directories:
   - `tests/fixtures/35mm-imx464-orion-tiff`
   - `tests/fixtures/250mm-dob-imx464-orion-png`
   - `tests/fixtures/130mm-imx464-dumbell-nebulae-png`
2. **Resize Images:** For each loaded frame, resize it to the 3 target resolutions:
   - 1920x1080 (FHD)
   - 2712x1538 (imx464 original size)
   - 3840x2160 (4K)
3. **Run Benchmarks:** For each resolution and each quality setting (90% and 95%), run encoding using:
   - `Jpegli`
   - `image::codecs::jpeg::JpegEncoder` (usual jpeg)
   - `turbojpeg`
4. **Collect Metrics:**
   - Measure **speed of encoding** (using `std::time::Instant`).
   - Measure **output file size** in bytes.
   - Calculate **fps on wifi** for a client with 60Mb/s (60 Mbps = 7.5 MB/s). FPS = `(7.5 * 1024 * 1024) / output_file_size`.
5. **Save Outputs:** Store the resulting encoded images with the following naming pattern:
   `tests/fixtures/processed/JPEG-bench/<FIXTURE_NAME>/<RESOLUTION>/<ENCODER>_<QUALITY>.jpg`
6. **Generate Output:** Print out 3 markdown tables per fixture (one for each resolution) with columns: `Encoder`, `Quality`, `Encoding Speed`, `Output File Size`, `FPS on 60Mbps WiFi`.

## Verification Plan

### Manual Verification
- Run the new benchmark using `cargo run --bin jpeg_benchmark --release`.
- Verify that the output tables are printed correctly in the terminal.
- Check the `tests/fixtures/processed/JPEG-bench/` directory to ensure that the images were saved correctly and that their file sizes match the reported sizes.
- Visually inspect the generated JPEG images to ensure they are properly encoded without severe artifacts.
