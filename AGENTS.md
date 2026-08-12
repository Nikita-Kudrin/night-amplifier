## Project Overview

Night Amplifier is an EAA (Electronically Assisted Astronomy) live stacking and auto-stretching engine in Rust.
Pipeline: calibration → debayer → detection → registration → stacking → background → render.

## Code guidelines

When interacting with this repository suggest architectures that are scalable, maintainable, secure, and highly
readable.
Always prioritize long-term maintainability over quick hacks.

Apply these core design philosophies to all code generation and changes:

SOLID Principles:

- Single Responsibility: One reason to change.
- Open/Closed: Open for extension, closed for modification.
- Liskov Substitution: Subtypes must be substitutable for base types.
- Interface Segregation: Many client-specific interfaces are better than one general-purpose interface.
- Dependency Inversion: Depend on abstractions, not concretions.
- DRY (Don't Repeat Yourself): Abstract shared logic into reusable utilities, but do not force abstractions prematurely
  if
  it couples unrelated domains.
- KISS (Keep It Simple, Stupid): Avoid over-engineering. Choose the simplest solution that effectively solves the
  problem.

When the file is too big (over 500 lines) consider refactoring and extracting functionality.
Follow standard Rust code conventions for backend and JavaScript for frontend.
Write tests for new functionality or for changed code.
Don't write useless obvious comments. Code should be self-describing. Use comments only to describe non-obvious
behavior.
Think how expensive operations should be for backend and frontend. Expensive operations should be avoided. Extremely
heavy operations should not be performed on the fly.
Remember to optimize imports and remove unused code you have created.
Try to avoid deep nesting. Use if+return to simplify the code.
ALWAYS prefer editing an existing file to creating a new one.
Proactively update documentation files (\\*.md, especially the VitePress user manual in `manual/`) or README files after changes in the code.
After making big changes, run backend and frontend tests.

## Architecture

When designing new features or refactoring, adhere to the following architectural principles:

- Clean Architecture / Hexagonal Architecture: Keep the core business logic (domain) isolated from external concerns (
  UI, databases, third-party APIs). Use interfaces/ports to decouple components.
- Domain-Driven Design (DDD): Group code by feature or domain (e.g., user, billing, inventory) rather than technical
  role (e.g., controllers, models, services).
- Separation of Concerns (SoC): Ensure each module, class, and function has a single, well-defined responsibility.
- Asynchronous Communication: Favor event-driven architectures (Pub/Sub, message queues) for long-running processes or
  cross-service communication to reduce tight coupling.
- Design for Failure (Resilience)
- Plugin system: performance-critical / Pro-only logic lives behind traits (`REJECTION_PLUGIN`, `PUSH_TO_PLUGIN`,
  `COMET_PLUGIN`, `BACKGROUND_PLUGIN`, `PLANETARY_STACKER_PLUGIN`) so Community works standalone.
- f32 normalization - All pixel math uses [0.0, 1.0] range to prevent overflow
- Rayon for multi-core processing
- No allocations in hot paths - Pre-allocated buffers where possible
- ARM friendly - Optimized for Raspberry Pi 5
- FFI safety - All C/C++ library calls wrapped with `catch_ffi_panic` to handle panics from vendor SDKs

# Test Guidelines

If you can't fix the test, don't try to simplify if by removing the idea of the test.
Tests might run a minute or two - you should wait for them to finish. Benches migth run even longer.
**Do not run benchmarks at the same time with other tests and tasks - this may affect the performance metrics.**

## Build & Test

**System prerequisite:** `nasm` (required by `turbojpeg-sys` for libjpeg-turbo SIMD).

```bash
cargo build --release
cargo test                                                          # fast unit tests
# These are ignored by default and must be run explicitly:
cargo test --test integration_pipeline -- --ignored --test-threads=1 # integration (slow)
cargo bench --bench <name>                                          # benchmarks — keep each bench binary ≤ ~30 s on CI; use `sample_size(10)`, short warm-up (~500 ms), and 1–2 s `measurement_time` (see `benches/background_benchmark.rs`).
cargo bench --bench <name> -- --noplot                              # ~4x faster wall clock: without gnuplot installed, criterion's plotters fallback dominates the run (debayer_benchmark: 95 s -> 22 s) while the measurements are identical. Prefer this unless you want the HTML report.
cargo run --release -- [port]
cargo run --release --features telemetry -- --telemetry

# Performance investigation
cargo run --release -- --span-timings                               # log per-stage durations on span close
cargo build --profile profiling                                     # release codegen with symbols, for `perf`

# Frontend (from web/)
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm install && npm run dev      # dev server on :5173, proxies to :8080
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run build                    # production build to web/dist/
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run lint:fix
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run test:run
```

Load nvm in subshells when needed: `. "$HOME/.nvm/nvm.sh" 2>/dev/null || true`.
**Note:** All `npm` commands must be run from the `web/` directory.

**Do not run `npm run format`.** Prettier rewrites every file in the tree, which pollutes the diff with
unrelated changes. The developer runs formatting on their own schedule — agents must leave it alone.

**Important:** Always run `cargo test` after making any code changes to ensure nothing is broken. For frontend changes,
also run `cd web && npm run test:run` to verify frontend tests pass.

## Core Modules (src/)

| Module                        | Purpose                                                                              |
|-------------------------------|--------------------------------------------------------------------------------------|
| `frame/`                      | `Frame` with normalized f32 pixels; format conversion                                |
| `debayer/`                    | RGGB/BGGR/GRBG/GBRG debayering; Bilinear + VNG                                       |
| `calibration/`                | Master dark / flat: `(raw - dark) / flat`                                            |
| `detection/`                  | Star detection with CoM sub-pixel centroiding, FWHM/SNR                              |
| `registration/`               | Triangle matching + RANSAC → `AffineTransform`                                       |
| `stacking/`                   | `MasterStack` accumulator, rejection, warping, quality weighting                     |
| `background/`                 | Grid-based gradient extraction (gradient_only / adaptive modes)                      |
| `render/`                     | Stretch (asinh/MTF), autostretch solver, white balance, black point, S-curve, output |
| `statistics/`                 | Robust per-channel median/MAD (sampling-based)                                       |
| `camera/`                     | Traits + ZWO/PlayerOne/QHY/ToupTek SDKs + simulator (see Camera Notes below)         |
| `planetary/`                  | Correlation-based alignment, percentile stacking (Moon/planets)                      |
| `ser/`                        | SER video format (read/write) for planetary                                          |
| `disk_writer/`                | Async bounded-queue frame writer                                                     |
| `plugins/`                    | Trait definitions for Pro-delegated features                                         |
| `push_to/`                    | Community-side Push-To trait definitions (impl is in Pro)                            |
| `server/`                     | Axum REST + WebSocket server                                                         |
| `app.rs`                      | Shared `app::run()` entry point for Community and Pro binaries                       |
| `ffi_safety.rs`               | `catch_ffi_panic`, buffer/dimension validation                                       |
| `logging.rs` / `telemetry.rs` | `tracing` + optional OpenTelemetry (OTLP)                                            |

### Server (src/server/)

Axum-based. REST at `/api/*`, WebSocket streams at `/ws/stream` and `/ws/eyepiece` (dynamic JPEG),
`/ws/eyepiece_quality` (lossless LZ4), and `/ws/events` (JSON). Shared state via
`Arc<RwLock<_>>` in `AppState`. See source for exact endpoints, DTOs, and event variants.

### Web Frontend (web/)

Vue 3 SPA, mobile-first, dark theme. Composables in `src/composables/`, components in `src/components/`. Vite proxies
`/api` and `/ws` to `localhost:8080` in dev.

## Camera Notes

Behavior that's not obvious from the code:

- **Cooler lifecycle**: open-at-connect handle lives in `AppState.active_camera`. `CameraPhase` transitions
  `Precooling → Idle → Capturing → WarmingUp`. Cool-down and warm-up are rate-limited to 5 °C/min to avoid mechanical
  stress and dew. Warm-up keeps the TEC on and ramps setpoint to 20 °C; handle closes once sensor ≥10 °C and duty ≤5 % (
  or 5 min timeout).
- **Live cooler edits**: while `Idle`, settings are forwarded via `camera_session::lifecycle::apply_cooler_settings`.
  During `Capturing` the per-frame path owns it; during `WarmingUp` the monitor intentionally holds the cooler off.
- **`cooler_fast_mode`**: expert override that bypasses the ramp entirely. UI shows a persistent warning while on.
- **Dual Sampling (Player One)**: sensor mode auto-selected via `StackingType::desired_sensor_mode()` (DeepSky/Comet →
  `LowReadoutNoise`, Planetary → `Normal`). Override with `CaptureSettings.sensor_mode_override`. Name matching lives in
  `src/camera/playerone/sensor_mode.rs` using raw `playerone-sdk-sys` bindings inside `catch_ffi_panic`.
- **Monitor thread**: runs on a dedicated `std::thread` (not tokio) so USB stalls can't poison the runtime.

## Storage Formats

| Output           | Format | Bit Depth       |
|------------------|--------|-----------------|
| Raw frames       | FITS   | 16-bit unsigned |
| Stacked image    | FITS   | 32-bit float    |
| Stacked preview  | PNG    | 8-bit           |
| Planetary frames | SER    | 16-bit unsigned |

### SER Video File Format

SER is the standard format for planetary imaging - uncompressed with per-frame timestamps.

**SER Color Formats:**
| ID | Format | Description |
|----|--------|-------------|
| 0 | Mono | Grayscale (1 channel) |
| 8 | BayerRGGB | Raw Bayer RGGB pattern |
| 9 | BayerGRBG | Raw Bayer GRBG pattern |
| 10 | BayerGBRG | Raw Bayer GBRG pattern |
| 11 | BayerBGGR | Raw Bayer BGGR pattern |
| 100| RGB | RGB color (3 channels) |
| 101| BGR | BGR color (3 channels) |

Directory layout: `captures/raw/DD-MM-YYYY_HH-MM-SS/frame_NNNNNN.fits` and `captures/stacked/DD-MM-YYYY_HH-MM-SS.fits`.

## Streaming Protocols

### Dynamic JPEG (SA10) — `/ws/stream`, `/ws/eyepiece`

Default streaming format. Encoded via TurboJPEG (SIMD) in the render task, not in the
WebSocket handlers — see *Demand-driven resolution tiers* below.

```
Magic "SA10" (4B, 0x53413130 LE) | Width u32 LE | Height u32 LE | Payload size u32 LE | JPEG bytes
```

#### Demand-driven resolution tiers

Clients send `{width, height}` JSON. The tier is selected from the viewport's **shorter edge**
clamped to 1080 … 2160 — its display-resolution class — not by fitting both edges into a box:

| Tier       | Bounding box  | Serves class | IMX464 (2712×1538) output |
|------------|---------------|--------------|---------------------------|
| `Hd1080`   | 1920×1080     | ≤ 1080       | 1904×1080                 |
| `Qhd1440`  | 2560×1440     | ≤ 1440       | 2539×1440                 |
| `Uhd2160`  | 3840×2160     | ≤ 2160       | 2712×1538 (no downsample) |
| `Original` | unbounded     | —            | 2712×1538                 |

Short-edge selection is deliberate and load-bearing. Frames are fitted to the viewport with their
aspect ratio intact, so the usable pixels are bounded by the shorter edge in either orientation.
Testing both edges against a 16:9 box would put a portrait 1080×2220 phone in the 4K tier — a
2× bandwidth increase to display 1080 px of a full-resolution frame. Rotating a device must not
change which tier it uses.

`clamp_client_resolution` still clamps per axis, but only for `encode_rgb8_jpeg_dynamic`; tier
selection does not go through it.

Each tier has an `AtomicUsize` client counter in `AppState.jpeg_tier_clients`, held by a
`JpegTierClientGuard` for the lifetime of a connection. The render task encodes one payload per
tier that has clients and caches it in `AppState.jpeg_tier_cache`; tiers that do not downsample
the frame share a single encode (for sub-4K sensors that collapses `Uhd2160` and `Original`).
Handlers wake on `frame_ready` and write the cached payload — no encoding, no `spawn_blocking`.
The exception is a client that has just connected or changed tier: it encodes once inline so the
view is not blank until the next frame, and publishes the result for others on that tier.

Frame publication is split so this stays race-free: the render task calls `begin_frame()` to claim
the counter, stores every payload against it, then `publish_frame()` to wake clients. A woken
client therefore never sees a counter whose payloads are still missing.

### Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`

Full-resolution lossless path for the eyepiece quality view.

```
Magic "SA08" (4B, 0x53413038 LE) | Width u32 LE | Height u32 LE | Compressed size u32 LE | LZ4 RGB8 payload
```

SA09 is the chunked variant (parallel LZ4 compression). Frontend renders via WebGL with
Canvas2D fallback.

## Adding a Stacking Type

Add variant to `StackingType` (`src/stacking/config.rs`), update `StackingType::all()`, and implement capability
methods: `display_name`, `description`, `uses_star_registration`, `supports_stacking`, `supports_quality_weighting`,
`uses_aggressive_stretch`, `desired_sensor_mode`. No changes needed in `capture.rs`.

## Settings Persistence

`settings.json` in server working directory. Loaded on startup, saved on `POST /api/settings`.

## Full Image Processing Pipeline

The image processing engine implements a comprehensive, multi-phase linear and non-linear mathematical pipeline designed
to extract the maximum possible signal from noisy astronomical data:

### Phase 1: Sensor Data Acquisition & Calibration

Corrects for sensor imperfections:

- **Master Dark Subtraction**: Removes thermal noise and amp glow by subtracting a stacked reference dark frame.
- **Master Flat Division**: Corrects for vignetting, dust motes, and uneven sensor illumination:
  `calibrated = (raw - dark) / flat`.
- Applies math purely in 32-bit floating-point precision.

### Phase 2: Debayering (Demosaicing)

Converts mono Bayer pattern (CFA) data into full RGB color.

- Auto-detects patterns (RGGB, BGGR, GRBG, GBRG).
- Bilinear Algorithm: Fast interpolation for live preview or less critical data.
- VNG (Variable Number of Gradients): High-quality interpolation avoiding color artifacts on edge transitions.

Non-obvious invariant, and the source of a fixed GRBG colour bug: at a green pixel, whether red is
interpolated horizontally or vertically depends on the **row only, never the column**. A green pixel
sits either in a red-and-green row or in a blue-and-green row, and its horizontal neighbours are
whichever colour the row carries. That is why `get_rb_orientation` keys on `y & 1` alone and why RGGB
and GRBG share an arm (as do BGGR and GBRG) despite their green pixels sitting at opposite column
parities. Keying it on `x` too used to send GRBG's odd-row greens down a "not a green position" path
that filled blue from the two *red* neighbours above and below — a quarter of every GRBG frame.
`test_debayer_reproduces_constant_colour_planes` pins this against ground truth for all four
patterns; keep it passing rather than adjusting it.

### Phase 3: Star Detection & Centroiding

Isolates and locates reference stars in the frame:

- Estimates local background statistics using Median and MAD.
- Thresholds image to find local maxima while rejecting isolated hot pixels.
- Calculates sub-pixel precision coordinates using a Center of Mass (CoM) algorithm within a search window.
- Calculates quality metrics: FWHM (sharpness) and SNR.

### Phase 4: Image Registration (Alignment)

Computes frame-to-frame shifts to counteract tracking errors and target movement. Supports multiple alignment strategies
based on the celestial target:

- **Deep Sky (Stars)**: Adaptive registration generates scale/rotation-invariant triangle patterns, matches them using
  RANSAC, and computes an `AffineTransform`.
- **Planetary (Correlation)**: Uses surface feature cross-correlation within an ROI to align high-framerate
  planetary/lunar frames where stars are absent.
- **Comet (Centroid)**: [Pro] Employs a specific `CometDetector` using an ROI around the comet's nucleus to compute the
  center of mass centroid for alignment, enabling the stack to track the moving comet while stars trail.

### Phase 5: Live Stacking & Rejection

Accumulates aligned frames to dramatically improve Signal-to-Noise Ratio (SNR).

- **Deep Sky (MasterStack)**: Warps frames via Bilinear Interpolation using the `AffineTransform`. Frames are weighted
  based on their FWHM/SNR relative to the reference frame. Outliers (satellite trails, cosmic rays) are rejected using
  specialized algorithms in the Pro version (e.g., `SigmaClip`, `WinsorizedSigmaClip`) via the `REJECTION_PLUGIN`.
- **Planetary Stacking (Lucky Imaging)**: Employs percentile stacking (e.g., top 10%-30% of frames) based on
  high-frequency sharpness metrics like Laplacian, Sobel, or Tenengrad.
- **Comet Stacking**: [Pro] Bypasses traditional weighting and uses highly aggressive `WinsorizedSigmaClip` to
  ruthlessly reject the trailing star field, cleanly isolating the comet signal.

### Phase 6: Background Extraction (Light Pollution Removal)

Removes uneven illumination gradients common in urban skies.

### Phase 7: Image Statistics (The Foundation)

Computes robust per-channel statistics.

### Phase 8: Auto-Color / Background Neutralization

Neutralizes color casts from light pollution.

### Phase 9: Black Point Calculation

Establishes the dark reference level.

### Phase 10: Shadow Saturation Boost (Optional)

Selectively enhances color saturation in faint signal regions.

### Phase 11: Core Tone Mapping (The Stretch)

### Phase 12: Autostretch Heuristic Solver

#### The Math

- **Asinh**: We solve for `stretch_factor` such that when `input = adjusted_median`, `output = target_background` (
  default 0.15). Uses a hybrid Newton-Raphson/Bisection solver.
- **MTF**: Solves algebraically for `m` based on the target background.

#### Pipeline Steps

1. Compute image statistics (median, MAD, sigma per channel)
2. Calculate black point: `BP = Median - (c × Sigma)`
3. Solve for tone mapping parameter linking `adjusted_median → target_background`
4. Subtract black point from frame
5. Apply chosen Tone Mapping algorithm with the computed parameter

### Phase 13: Final Output Mapping & Contrast

#### S-Curve Contrast (`ContrastConfig`)

Luminance-preserving contrast adjustment using a parametric S-curve:

## Logging

`RUST_LOG` overrides levels. `tracing` + daily file rotation via `tracing-appender`. Telemetry via `--telemetry` /
`OTEL_EXPORTER_OTLP_ENDPOINT` when built with `--features telemetry`.

### Pipeline performance instrumentation

`--span-timings` turns on `FmtSpan::NEW | CLOSE`, so every stage span (`camera_capture`, `Debayerer::debayer`,
`stacking_iteration`, `render_iteration`, `process_preview_frame`, `encode_jpeg_tiers`) logs its duration. This is
the on-device breakdown — no OTLP collector required.

Inside `process_preview_frame`, note that the render tail is **fused**: black point subtraction, tone mapping and
S-curve contrast run as a single pass under `auto_stretch` → `apply_fused_stretch_frame`. There is no
`contrast_adjustment` span on that path. Contrast falls back to its own `contrast_adjustment` pass only when
`saturation_boost` is on (saturation sits between stretch and contrast and is a cross-channel op that breaks the
fusion) or when auto-stretch is off or failed.

With `--features telemetry`, the same stages also export OTel histograms from `telemetry::metrics`:
`frame.capture_ms`, `frame.debayer_ms`, `frame.stack_ms`, `frame.render_ms`, `frame.encode_lz4_ms`,
`frame.encode_jpeg_ms` (attribute `tier`), plus counters `frame.published` (rate = delivered FPS),
`frame.dropped` and `frame.render_skipped` (render stage falling behind).

Two rules for anything added here:

- **Stage granularity only.** Per-row or per-pixel instrumentation distorts what it measures.
- **Cache the instrument.** The session-scoped `record_*` helpers rebuild their meter and instrument per call,
  which is fine at their cadence but must never be copied for a per-frame metric. Follow `pipeline_histogram` /
  `frame_counter`, which cache in a `OnceLock` and only populate it once the meter provider exists.

For `perf`, build with `--profile profiling`: the `release` profile sets `strip = true` and profiles otherwise show
raw addresses. Note that rayon worker threads inherit the name of the thread that first used the global pool, so
rayon work is attributed to `tokio-rt-worker` in `perf` output.
