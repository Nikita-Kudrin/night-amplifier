# JPEG Encoder Comparison Benchmark: Jpegli vs TurboJPEG vs "usual" JPEG

## Context

The `push-to-speed-optimization` branch (checked out identically in both
`night-amplifier` and `night-amplifier-pro`) has been tightening the live-view
WiFi streaming path — `encoding_tests.rs` and `Image_Streaming_Results.md` were
both touched as recently as 2026-08-13 for exactly this reason. The next
question is whether **Jpegli** (Google/libjxl's modernized libjpeg-compatible
encoder) beats the two encoders already in play — production TurboJPEG and the
pure-Rust `image` crate encoder used as a baseline — on the metrics that matter
for this app: encode latency, wire size, and the frames/sec a 60 Mb/s client
link can sustain at that size. This plan adds a repeatable in-repo benchmark
that answers that, producing 9 markdown tables (3 fixtures × 3 resolutions) at
quality 90 and 95.

**Work happens in `/opt/GitHub/night-amplifier`** (the community repo), not
this session's cwd (`night-amplifier-pro`) — that's where TurboJPEG, `image`,
the target fixtures, and all prior encoding-benchmark precedent already live.
Branch `push-to-speed-optimization` is already checked out there and clean.

## Corrections to the request

- The fixture is `130mm-imx464-dumbell-nebulae-png` — one **"b"** ("dumbell"),
  not "dumbbell". Its sibling `130mm-imx464-ring-nebulae-png` exists and must
  be explicitly excluded (a plain `130mm-*` prefix match would catch both).
- All three requested fixtures (`35mm-imx464-orion-tiff`,
  `250mm-dob-imx464-orion-png`, `130mm-imx464-dumbell-nebulae-png`) are raw,
  single-channel, 16-bit **Bayer-mosaic** captures at the sensor's native
  2712×1538 (IMX464) — confirmed by directly parsing the PNG/TIFF headers, not
  finished photos. Encoding the mosaic directly would benchmark noise
  patterns, not real streamed frames, so each gets debayered and rendered
  before it reaches any JPEG encoder (see Pipeline below).

## Key decisions

1. **One representative frame per fixture** (first frame, sorted by
   filename), not a full multi-sub stack. This matches the currently-active
   harness (`encoding_tests.rs::load_first_fixture_frame`, last touched
   yesterday) rather than the older `Image_Streaming_Results.md` methodology
   (which used fully stacked+stretched exports). A per-frame link-fps metric
   is inherently about the live-view single-frame path, which is what this
   branch is optimizing.
2. **"Usual jpeg" = `image::codecs::jpeg::JpegEncoder`** (pure-Rust baseline,
   already a dependency) — matches the `image_jpeg_95` naming already used
   identically in both `encoding_benchmark.rs` and `encoding_tests.rs`.
3. **Jpegli via the `jpegli` crate** (safe wrapper over `jpegli-sys`, a real
   FFI binding to Google/libjxl's C++ jpegli 0.10.2 — not a reimplementation),
   gated behind a new **opt-in, non-default Cargo feature** `jpegli-bench`.
   Explicitly *not* using `jpegli-rs` (crates.io name for an independent
   from-scratch Rust port, itself mid-rename to "zenjpeg") and *not* shelling
   out to a `cjpegli` CLI (not packaged by any distro; building it means
   building the entire libjxl monorepo with submodules — heavier than
   `jpegli-sys`'s already-vendored, scoped-down source — and a subprocess
   would unfairly add spawn overhead to its "encode time" versus the two
   in-process encoders).
4. **New standalone report** `benches/JPEG_Encoder_Comparison.md`, not an
   extension of `Image_Streaming_Results.md` — different fixtures, different
   encoder set (adds Jpegli, drops WebP/PNG/LZ4), different source-frame
   methodology. Keeping them separate avoids implying directly comparable
   numbers across two different measurement setups.
5. **New test file** `tests/integration/jpeg_encoder_comparison.rs`, not more
   code in `encoding_tests.rs` (already 775 lines, over AGENTS.md's 500-line
   guideline).

## Output layout (as requested)

```
tests/fixtures/processed/JPEG-bench/<fixture-dir-name>/<resolution-label>/<encoder>_q<quality>.jpg
```

- `<fixture-dir-name>`: `35mm-imx464-orion-tiff`, `250mm-dob-imx464-orion-png`,
  `130mm-imx464-dumbell-nebulae-png` — the real directory names, no aliasing.
- `<resolution-label>`: `1080p` (1920×1080), `original` (native, 2712×1538),
  `4k` (3840×2160 — necessarily a **synthetic upscale**: the IMX464 sensor is
  smaller than 4K, and production's box-downsample path only ever shrinks, so
  there is no "real" 4K frame to source this from).
- Files per resolution dir: `turbojpeg_q90.jpg`, `turbojpeg_q95.jpg`,
  `usual_jpeg_q90.jpg`, `usual_jpeg_q95.jpg`, `jpegli_q90.jpg`,
  `jpegli_q95.jpg` — 54 files total across the full run. Already covered by
  the existing `.gitignore` rule for `tests/fixtures/processed/`.

## Pipeline (per fixture, reusing existing code — no new rendering logic)

```
raw Bayer Frame (load_png_mono / load_tiff, first frame by filename)
  → night_amplifier::debayer::debayer_auto()
  → RenderPipeline with live-view config (background subtraction OFF,
    AutoStretchConfig::from_profile(false, StretchAggressiveness::High),
    auto-stretch ON, contrast ON) — exactly probe_render_task_stage_breakdown's config
  → preview.to_rgb8_fast()
  → image::imageops::resize_exact(..., FilterType::Lanczos3) to each of the 3 resolutions
```

## Encoders & grid

3 encoders × 2 qualities (90, 95) × 3 resolutions × 3 fixtures = 54 encodes.
Each cell reuses the existing timing convention (1 warm-up + mean of
`ENCODE_TIMING_ITERATIONS` = 5 timed runs) and reports:

- Encode time (ms)
- File size (KB/MB)
- Bandwidth-Bound FPS @ 60 Mb/s = `(BASELINE_NETWORK_MBPS * 1e6 / 8.0) / file_size_bytes`
  — reusing the existing `BASELINE_NETWORK_MBPS = 60.0` constant and formula
  verbatim (this is a **computed** metric from file size, not a live network
  test, matching the existing doc-comment in `encoding_tests.rs`).

| Encoder | Source | Call pattern |
|---|---|---|
| TurboJPEG | existing dep `turbojpeg = "1.5.1"` | `Subsamp::Sub2x2`, matching production `configure_compressor` in `src/server/encoding.rs` |
| "Usual" JPEG | existing dep `image` | `image::codecs::jpeg::JpegEncoder::new_with_quality` (no subsampling control exposed) |
| Jpegli | **new** optional dep `jpegli` | `Compress::new(ColorSpace::JCS_RGB)` → `set_size` → `set_quality(f32)` → `start_compress` → `write_scanlines` → `finish`, wrapped in the existing `night_amplifier::catch_ffi_panic` (`src/ffi_safety.rs`) since jpegli panics rather than returning `Result` on libjpeg errors |

## File-by-file changes

| File | Change |
|---|---|
| `tests/integration/jpeg_encoder_comparison.rs` | **New.** Exact 3-directory fixture allow-list (deliberately not reusing `BASELINE_FIXTURE_PREFIXES`, whose prefix-match semantics would also catch the ring-nebula sibling); pipeline call; `encode_turbojpeg`/`encode_image_jpeg`/`encode_jpegli` (the last `#[cfg(feature = "jpegli-bench")]` with a clear-error stub otherwise); table builder printing ready-to-paste `\| … \|` markdown rows; JPEG file saver; one `#[test] #[serial] #[ignore]` entry point producing all 9 tables. |
| `tests/integration/mod.rs` | Add `pub mod jpeg_encoder_comparison;`. |
| `tests/integration/image_loading.rs` | Promote `load_png_mono` and `load_first_fixture_frame` out of `encoding_tests.rs` (true duplication between the two files — both need byte-identical loading). |
| `tests/integration/common.rs` | Promote `BASELINE_NETWORK_MBPS` and `ENCODE_TIMING_ITERATIONS` constants (its doc-comment already scopes it as the shared-constants home). |
| `tests/integration/encoding_tests.rs` | Remove the now-duplicated private copies; import the promoted versions instead. |
| `Cargo.toml` | Add `jpegli = { version = "0.1", optional = true }` under `[dependencies]` — **not** `[dev-dependencies]`, since Cargo has no mechanism to make an optional dev-dependency (tracked upstream as `rust-lang/cargo#1596`, still open). Add `jpegli-bench = ["dep:jpegli"]` under `[features]`, absent from `default`. |
| `benches/JPEG_Encoder_Comparison.md` | **New.** 3 `##` fixture sections × 3 `###` resolution subsections, columns `Encoder \| Quality \| Encode Time \| File Size \| Bandwidth-Bound FPS @ 60Mb/s`, best-per-column bolded (mirroring `Image_Streaming_Results.md`'s style), plus a methodology section documenting: single-frame-not-stack, 4K-is-a-synthetic-upscale, subsampling is *not* matched across encoders (`image` exposes none), and JPEG quality numbers are *not* perceptually equivalent across encoders (jpegli remaps quality internally to a butteraugli-distance target; its own tooling recommends a 68–96 range). |
| `AGENTS.md` (community repo) | One line under "Build & Test" documenting the `--features jpegli-bench` opt-in and how to run this test, per the repo's own "proactively update documentation" rule. |

Table generation stays **print-only** (ready-to-paste markdown emitted via
`println!`, hand-copied into the `.md` once) rather than having the test write
the tracked doc file itself — this matches how `Image_Streaming_Results.md`
was actually produced (confirmed via `git log`: hand-authored in the same
commit as its code; no Rust code anywhere in the repo writes `.md` files
today), and avoids introducing a "test silently overwrites committed prose"
pattern.

## Risks (flagged, not silently absorbed)

- **`jpegli` is 0.1.0, one release, self-described "rough edges."** First
  implementation step should be an isolated spike — add the dependency and
  confirm it actually compiles here — before wiring it into the harness.
  `cmake` and `pkg-config` are present locally; `ninja` is not, but
  `jpegli-sys`'s build-dependencies are only `cmake`+`pkg-config` (no ninja),
  so the Rust `cmake` crate should fall back to a Makefiles generator — this
  needs empirical confirmation, not just this inference, since its `build.rs`
  wasn't read directly. If it doesn't build in reasonable time, **drop the
  Jpegli row and say so in the report** rather than substituting `jpegli-rs`
  under the "Jpegli" label.
- **CI's `rust-coverage` job runs `cargo llvm-cov --all-features`**, which
  will compile jpegli's vendored C++ regardless of the feature being
  non-default everywhere else (`--all-features` has no exclusion mechanism).
  Bounded risk — GitHub's hosted `ubuntu-24.04` runners already have
  `cmake`/`ninja` preinstalled — but a real added cost to that one job, worth
  flagging to whoever reviews the `Cargo.toml` diff. None of the four
  `ci.yml` jobs currently install `cmake`/`ninja-build` in their apt-get step;
  none should need to, given the above, but confirm after the spike-compile.
  `tests/fixtures/` is never populated in CI, so the new test will no-op
  there exactly like every other fixture-driven test does today (graceful
  skip-if-missing, not a failure).
- **Chroma subsampling and quality scale are not matched across encoders** —
  `image`'s encoder exposes no subsampling control at all, TurboJPEG uses
  `Subsamp::Sub2x2`, jpegli uses raw per-component pixel-size tuples. Quality
  90/95 is a same-numeric-input comparison, not a same-perceptual-target one.
  Both go in the report's methodology notes rather than being quietly glossed
  over.

## Verification

```bash
cd /opt/GitHub/night-amplifier
cargo test --release --features jpegli-bench --test integration_pipeline \
  jpeg_encoder_comparison -- --ignored --test-threads=1
```

Fixtures must exist under `tests/fixtures/` first (auto-downloadable via the
existing `ensure_fixtures`/`ensure_fixtures_sync` helpers in `common.rs`, or
already present locally). Then:

1. Spot-check a handful of the 54 saved JPEGs open cleanly and look like the
   source image (not corrupted/garbled) — this is the step most likely to
   catch a jpegli integration mistake (e.g. wrong color space/scanline order).
2. Copy the printed markdown rows into `benches/JPEG_Encoder_Comparison.md`.
3. Per AGENTS.md, run `cargo test` (fast unit tests) and
   `cargo bench --no-run --no-default-features` (compile check, matching
   CI's `rust-build-all` job) before calling this done.

