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
Proactively update documentation files (\\*.md, especially the VitePress user manual in `manual/`) or README files after changes in the code but keep the docs concise and focused.  
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

## Benchmark sizing

Two hard rules: every case reports **≥~100ms** (below that, criterion overhead and thermal
throttling dominate the number), and every bench binary stays **≤~30s** wall clock.

To hit both: repeat pure routines `REPS`× in `b.iter` with `Throughput::Elements(REPS*n)` and
a `_xN` suffix (`debayer_benchmark`); for in-place mutation use `iter_batched_ref` over `REPS`
clones, not plain repetition (`render_benchmark`); match production input *size and shape*, not
an arbitrary buffer (`star_detection_benchmark` was mono-only and missed `mean_luminance`'s
24.9ms colour-channel cost); delete cases that only re-measure a bigger sibling's kernel; add
cases that can *refute* a hypothesis, not just confirm it (`image_stats/full_precision` reads
42x the samples to prove a gather wasn't the cost); benchmark whole-pipeline sums too — per-stage
coverage still missed a regression in `process_preview_frame` (190 of 300ms).

Stay in budget with `sample_size(10)`, ~500ms warm-up, 1–2s `measurement_time`, and
`SamplingMode::Flat` (default Linear's 55 iterations/case alone blows 30s).

- Suffix `_x5` etc. whenever `time:` covers more than one call.
- Hoist reusable setup out of `iter_batched`'s loop — setup still runs every iteration.
- Use `iter_batched_ref(.., BatchSize::LargeInput)`, never `frame.clone()` inside `b.iter`.
- Watch for workloads that drift across iterations (op applied to its own prior output).

## Build & Test

**System prerequisite:** `nasm` (required by `turbojpeg-sys` for libjpeg-turbo SIMD).

```bash
cargo build --release
cargo test                                                          # fast unit tests
# These are ignored by default and must be run explicitly:
cargo test --test integration_pipeline -- --ignored --test-threads=1 # integration (slow)
cargo bench --bench <name>                                          # benchmarks — see **Benchmark sizing** below before adding or editing one.
cargo bench --bench <name> -- --noplot                              # ~4x faster wall clock: without gnuplot installed, criterion's plotters fallback dominates the run (debayer_benchmark: 95 s -> 22 s) while the measurements are identical. Prefer this unless you want the HTML report.
cargo run --release -- [port]
cargo run --release --features telemetry -- --telemetry

# Performance investigation
cargo run --release -- --span-timings                               # log per-stage durations on span close
cargo build --profile profiling                                     # release codegen with symbols, for `perf`

# Frontend (from web/)
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm install && npm run dev      # dev server on :8844, proxies to :9955
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
| `fits/`                       | FITS read (`read_frame`) and write; `interpret_shape` for NAXIS layout               |
| `debayer/`                    | RGGB/BGGR/GRBG/GBRG debayering; Bilinear + VNG + Superpixel                          |
| `cfa/`                        | Raw-CFA stage run before demosaic: hot pixels, row/column FPN                        |
| `render/denoise/`             | Guided-filter chroma + à trous wavelet luma, run at *stream* resolution              |
| `calibration/`                | Master dark / flat: `(raw - dark) / flat`                                            |
| `detection/`                  | Star detection with CoM sub-pixel centroiding, FWHM/SNR                              |
| `registration/`               | Triangle matching + RANSAC → `AffineTransform`                                       |
| `stacking/`                   | `MasterStack` accumulator, rejection, warping, quality weighting                     |
| `background/`                 | Grid-based gradient extraction (gradient_only / adaptive modes)                      |
| `render/`                     | Stretch (asinh/MTF), autostretch solver, white balance, black point, S-curve, shadow floor, output |
| `statistics/`                 | Robust per-channel median/MAD (sampling-based)                                       |
| `camera/`                     | Traits + ZWO/PlayerOne/QHY/ToupTek SDKs + simulator (see Camera Notes below)         |
| `planetary/`                  | Correlation-based alignment, percentile stacking (Moon/planets)                      |
| `ser/`                        | SER video format (read/write) for planetary                                          |
| `disk_writer/`                | Async bounded-queue frame writer                                                     |
| `plugins/`                    | Trait definitions for Pro-delegated features                                         |
| `push_to/`                    | Community-side Push-To trait definitions (impl is in Pro)                            |
| `server/`                     | Axum REST + WebSocket server                                                         |
| `app.rs`                      | Shared `app::run()` entry point for Community and Pro binaries                       |
| `parallel.rs`                 | `balanced_chunk_len` — rayon work partitioning, shared with Pro                      |
| `ffi_safety.rs`               | `catch_ffi_panic`, buffer/dimension validation                                       |
| `logging.rs` / `telemetry.rs` | `tracing` + optional OpenTelemetry (OTLP)                                            |

### Server (src/server/)

Axum-based. REST at `/api/*`, WebSocket streams at `/ws/stream` and `/ws/eyepiece` (dynamic JPEG),
`/ws/eyepiece_quality` (lossless LZ4), and `/ws/events` (JSON). Shared state via
`Arc<RwLock<_>>` in `AppState`. See source for exact endpoints, DTOs, and event variants.

### Web Frontend (web/)

Vue 3 SPA, mobile-first, dark theme. Composables in `src/composables/`, components in `src/components/`. Vite proxies
`/api` and `/ws` to `localhost:9955` in dev.

## Camera Notes

- **Cooler lifecycle**: handle lives in `AppState.active_camera`. `CameraPhase`:
  `Precooling → Idle → Capturing → WarmingUp`. Ramp limited to 5°C/min; warm-up ramps to 20°C,
  closes the handle once sensor ≥10°C and duty ≤5% (or 5min timeout).
- **Live cooler edits**: `Idle` → `apply_cooler_settings`; `Capturing` → owned by the per-frame
  path; `WarmingUp` → monitor holds cooler off intentionally.
- **`cooler_fast_mode`**: bypasses the ramp; UI shows a persistent warning while on.
- **Dual Sampling (Player One)**: sensor mode auto-picked by `desired_sensor_mode()`
  (DeepSky/Comet → `LowReadoutNoise`, Planetary → `Normal`), overridable via
  `sensor_mode_override`.
- **Monitor thread**: dedicated `std::thread`, not tokio, so USB stalls can't poison the
  runtime; uses one reusable `monitor::FfiWorker` rather than a thread per poll.

### Handle ownership — non-obvious and load-bearing

**A vendor close takes a device *index*, not a handle** — `POACloseCamera(0)` / `ASICloseCamera(0)` /
`SVBCloseCamera(0)` close whatever occupies index 0 at that moment, so a stuck FFI call handed to a
detached thread can close a camera that has since reconnected when its `Drop` fires minutes later.

- Every shim-level handle holds a `camera::DeviceLease`; **every vendor close goes through
  `lease.begin_close()`**, which authorizes exactly one close for the lease that still owns the slot.
  A `Drop` that calls the SDK directly is a review flag.
- Never close an abandoned handle eagerly — a stuck synchronous FFI call can't be cancelled; the lease
  is what makes abandoning safe.
- `connect()` **probes the handle before reporting success** — `open()` returning proves nothing.

### Device-loss classification

`CameraError::is_sdk_disconnected()` knows no vendor vocabulary — each shim classifies its **own
numeric/enum code** and tags the message via `camera::device_lost::mark` (matching vendor substrings
after the fact doesn't generalize: only PlayerOne renders errors symbolically, the rest print bare
integers).

`status()` reads go through `device_lost::tolerate_unsupported`, not `.unwrap_or(default)`, so an
unsupported parameter falls back while a lost device still propagates instead of reading as a fake
`Ok`.

### Fault detection and recovery

One detector (`server::camera_health`), one threshold, one streak
(`consecutive_watchdog_timeouts`) fed by all three watchdog/monitor sites, so an alternating
fault still escalates; it ages out (`FAULT_STREAK_TTL`) instead of resetting on success.

`camera_session::reconnect` owns recovery (bounded attempts, backoff, re-enumeration,
liveness probe). `finalize_disconnect` takes a `DisconnectCause`, not a bool — a warmup
teardown must never reconnect.

## Storage Formats

| Output           | Format | Bit Depth       |
|------------------|--------|-----------------|
| Raw frames       | FITS   | 16-bit unsigned |
| Stacked image    | FITS   | 32-bit float    |
| Stacked preview  | PNG    | 8-bit           |
| Planetary frames | SER    | 16-bit unsigned |

**The stacked preview PNG goes through the live-view encoder, not the render pipeline** —
denoise and `DisplayOutput` pedestal/dither are encoder-only stages a bare `RenderPipeline`
call would skip. Same reason it's always RGB8, even for mono: replicated like the live stream.

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
WebSocket handlers.

```
Magic "SA10" (4B, 0x53413130 LE) | Width u32 LE | Height u32 LE | Payload size u32 LE | JPEG bytes
```

#### Demand-driven resolution tiers

Clients send `{width, height}`; tier is picked from the viewport's **shorter edge**, clamped
1080…2160 (fitting both edges into a box would push a portrait phone into the 4K tier).

| Tier       | Bounding box  | Serves class | IMX464 (2712×1538) output |
|------------|---------------|--------------|---------------------------|
| `Hd1080`   | 1920×1080     | ≤ 1080       | 1904×1080                 |
| `Qhd1440`  | 2560×1440     | ≤ 1440       | 2539×1440                 |
| `Uhd2160`  | 3840×2160     | ≤ 2160       | 2712×1538 (no downsample) |
| `Original` | unbounded     | —            | 2712×1538                 |

The render task encodes one cached payload per tier with clients (shared across
non-downsampling tiers on sub-4K sensors); handlers serve it on `frame_ready` except a
newly-connected client, which encodes once inline. `begin_frame`/`publish_frame` keep
publication race-free.

### Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`

Lossless (unquantized-beyond-8-bit) path for the eyepiece quality view.

```
Magic "SA08" (4B, 0x53413038 LE) | Width u32 LE | Height u32 LE | Compressed size u32 LE | LZ4 RGB8 payload
```

SA09 is the chunked variant (parallel LZ4 compression). Frontend renders via WebGL with
Canvas2D fallback.

#### Client streaming resolution negotiation

Reports `{width, height}` (was hardcoded 3840×2160), box-averages down through the same
`JpegTier`. The averaging *removes noise* proportional to the reduction — unlike WebGL's
~1.45x-capped fallback — so value scales with spare resolution (IMX533 2.25x smaller payload
at 8.26→6.76 sky-sigma; IMX464 barely moves, needs denoising instead).

- Stream sizes to the **largest** requested tier; unreported viewport defaults to the **4K
  cap**, not the floor — never downgrade an old client.
- Frontend reports **canvas**, not window, size (binoview's eyes are ~half-window each).
- Re-reported on every reconnect (no server-side memory) — never memoize "same size, skip".

## Adding a Stacking Type

Add variant to `StackingType` (`src/stacking/config.rs`), update `StackingType::all()`, and implement capability
methods: `display_name`, `description`, `uses_star_registration`, `supports_stacking`, `supports_quality_weighting`,
`uses_aggressive_stretch`, `desired_sensor_mode`. No changes needed in `capture.rs`.

## Settings Persistence

`settings.json` in server working directory. Loaded on startup, saved on `POST /api/settings`.

## Full Image Processing Pipeline

Multi-phase linear/non-linear pipeline that extracts maximum signal from noisy astronomical data:

### Phase 1: Sensor Data Acquisition & Calibration

Corrects for sensor imperfections:

- **Master Dark Subtraction**: Removes thermal noise and amp glow by subtracting a stacked reference dark frame.
- **Master Flat Division**: Corrects for vignetting, dust motes, and uneven sensor illumination:
  `calibrated = (raw - dark) / flat`.
- Applies math purely in 32-bit floating-point precision.

### The raw-CFA stage (`cfa/`) — where pre-demosaic corrections live

`RawFrame::to_cfa_frame` yields a still-mosaiced `CfaFrame`; the **stacking task** runs a
`CfaPipeline` over it before demosaic (`to_frame` = that + empty pipeline + bilinear, pinned by
a test). Same seam will host calibration (dark/flat), not yet wired in.

- Both filters work one colour site at a time — mixing sites reads the mosaic pattern as signal.
- **`hot_pixels`**: gated on the *fraction* of centre amplitude the brightest neighbour carries,
  not a raw diff, so bright star cores survive.
- **`fpn`**: levels each line against a narrow (±8) even-order average of its own neighbours,
  not a whole-frame reference (which silently removed 5.2% of real flux). Skipped for Planetary.
- **Planetary gets hot-pixel rejection only** — FPN bands the disc, superpixel halves
  resolution.
- Timed at `info_span!` (~7ms + ~7.2ms/frame on IMX533/Pi). Per-site stats are precomputed and
  TTL-cached, so `CfaPipeline` rebuilds per settings-change, not per frame.

### Frame memory layout (planar) — non-obvious and load-bearing

`Frame` stores samples **plane-major** (`idx = channel*w*h + y*w + x`) so filters read a
channel as one contiguous run. **Every 8-bit output format is interleaved instead** — crossing
the boundary wrongly still compiles and collapses channels toward grey. FITS (NAXIS3=3) alone
stays planar.

Rules: use `planes()`/`channel_data()`/`get_pixel()`, never `frame.data()` with `* channels`
math; build fixtures with `set_pixel`, covered by `layout_tests` per format and traversal;
8-bit conversion always rounds via `sample_to_u8` (16-bit truncates); never derive a channel
index from a flat rayon chunk index, dispatch per plane instead. `get_pixel` in a whole-frame
loop is a review flag — cost 120ms/frame in `white_balance::block_medians` until moved onto
`planes()` + rayon (27ms).

### Spatial denoising (`render::denoise`) — runs in the encoder, not the pipeline

Two filters in `server::encoding::fused`, not the pipeline: **guided** (chroma mottle,
luma-guided) and **wavelet** (à trous B3, 4 levels, MAD-thresholded luma). Run at stream
resolution, after resample/before tone curve — full-res then discarding 3/4 would be 4.5x the
memory traffic. ~17ms combined at 1440² (20-core x86).

- Off fuses per-row; either filter on stages the whole image as f32 first (cross-row access).
- Thresholds `k=[0,3,2,1]` get weaker at finer scale on purpose — coarse-heavy denoising erases
  real nebula structure.
- `k[0]` (grain) is user-exposed as `star_protection`; off by default, ceiling reaches ~7x
  noise reduction.
- Skipped for `StackingType::Planetary` — lucky imaging needs the detail this removes.

### Denoising cost

Denoising is ~**5x the cost of the encode it sits in** (IMX533 @1440 tier, 20-core x86: 4.7ms
without, 17.9ms with). Two structures stop that from multiplying:

- **`ConversionCache`** shares one RGB8 conversion per distinct output size, keyed on
  `output_dimensions`, so a session with lossless + two JPEG tiers doesn't denoise three times.
- **`DenoiseScratch`** is owned by the render thread, not allocated per pass — a 1440² pass
  would otherwise page-fault ~75MB (13 of the 20ms the filters add). Passed down explicitly
  rather than thread-local, since per-client inline encodes run on pooled tokio blocking
  threads where thread-local would strand 75MB/thread.

Both spans report under `--span-timings`.

### The f32 -> 8-bit boundary (`render::output::quantize`)

Every displayed byte crosses this boundary once, via `sample_to_u8` — kept as one helper
because parallel 8-bit conversions have drifted by an LSB here before.

`DisplayOutput` (both off by default):
- **`pedestal`**: maps `[0,1]`→`[pedestal,1]` — autostretch clamps ~0.8% of samples to exactly
  0, which OLEDs show as speckle.
- **`dither`**: sub-LSB ordered dither before rounding (replaced a post-round version with
  visible crosshatch). Indexed in **output**, not input, coordinates, or resampling would
  average it away. Matrix is **8x8**: 4x4's ~7 arcmin period is still eye-resolvable.

`black_point_sigma` is scale-invariant (grain doesn't shrink with stack depth), so the eyepiece
slider interpolates it *upward*, not down.

### The shadow floor (`render::output::shadow_floor`) — the other half of black floor slider

`EyepieceSettings::black_floor` is **signed**: positive is `DisplayOutput::pedestal`
(panel-relative), negative is the shadow floor (sky-relative) — at `-5%`, sky measures
71%/65% darker with contrast *up*, vs. a plain black-level slider's flat 50% darker.

- Tone-curve stage, before quantization, applied in exactly **two** places that must agree.
  Order is always `stretch → saturation → contrast → floor`.
- Anchors to the *solved* `AutoStretchResult::target_background`, not the configured value.
- Three gates: sign, auto-stretch on, and not `StackingType::Planetary`.

MTF stretch arms (incl. default `Medium`) can't fuse the floor into a table and apply it
explicitly after contrast instead — silently dropped once, now swept by a test. Cost: free
fused; ~1.3ms of a 28ms 1440² encode when deferred.

### Phase 2: Debayering (Demosaicing)

Converts mosaic Bayer (CFA) data to full RGB. Auto-detects RGGB/BGGR/GRBG/GBRG.

- **Bilinear**: fast, for live preview.
- **VNG**: higher quality, avoids edge-transition color artifacts.
- **Superpixel**: one RGB pixel per 2x2 quad (half width/height), interpolates nothing so it
  invents no chroma noise. Opt-in (`superpixel_debayer`) — only worthwhile on sensors that
  oversample the display (IMX533 3008²→1504² is still above a 1440² eyepiece; IMX464
  2712x1538→1356x769 is below it).

**Non-obvious invariant** (source of a fixed GRBG bug): at a green pixel, whether red
interpolates horizontally or vertically depends on **row only, never column** —
`get_rb_orientation` keys on `y & 1` alone. Keying on `x` too used to misroute GRBG's odd-row
greens across a quarter of every frame; `test_debayer_reproduces_constant_colour_planes` pins
all four patterns.

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

Spatial denoising is *not* one of these phases — it runs in the streaming
encoders at display resolution rather than in the pipeline. See **Spatial
denoising** above.

#### S-Curve Contrast (`ContrastConfig`)

Luminance-preserving contrast adjustment using a parametric S-curve:

## Logging

`RUST_LOG` overrides levels. `tracing` + daily file rotation via `tracing-appender`. Telemetry via `--telemetry` /
`OTEL_EXPORTER_OTLP_ENDPOINT` when built with `--features telemetry`.

### Three things the render and stacking threads deliberately do not do every frame

From one production trace: both workers at 97% utilisation, 34.7% of captured frames dropped
for want of a stacking thread. A dropped *capture* frame loses signal permanently — most of
these favor the stacking side for that reason.

- **Display copy skipped when unneeded.** `MasterStack::compute()`'s copy (434MB read/108MB
  written) is gated by `want_display`; the frame still stacks regardless.
- **Queue budget sized from the board**: `min(MemTotal/5, 1GiB)`, floored at 64MiB. Each
  channel is sized from its own payload (raw vs. debayered), not one shared figure. The
  capture→stacking channel is additionally **bounded by latency** (2s of exposures): a
  deeper queue does not raise throughput, it delays the drop and pays in preview lag —
  memory alone put 19 frames / 2.9s in front of the stacking thread. The storage channel
  keeps its memory depth; backlog there is not lag. All three report
  `pipeline.queue_depth`/`_capacity` under `--features telemetry`, which is what separates
  "the stage is slow" from "it stalled once".
- **Preview may run binned**, by the largest integer factor `PreviewResolution` allows.
  All-or-nothing at the 2x boundary, and **fixed for the capture session** — resolved from
  the sensor shape plus that setting, never from the connected client set. Binning is not
  neutral to what follows it: the tone curve is solved from median and MAD, and 2x2 binning
  moved the solved `scale_lut` by +25.7% at the 1% input point, so a client-driven factor
  re-graded the picture for every viewer whenever somebody opened a tab. Default is
  `Native` (no binning), which is also what makes `JpegTier::Original`'s "no downsampling"
  structurally true.
- **Per-stack estimates (white balance, background, stats) are reused across frames**,
  refreshed by *proportional* stack-depth growth (MAD ~ 1/√N), not frame count. Live view
  never reuses — every sub there is a different image.

### The accumulator layout

`IncrementalPixel` (16B/sample) makes a 3008² colour stack a 434MB accumulator, read+written
whole every frame — ~32GB/s at 26.7ms, already the memory ceiling (no arithmetic win left,
only traffic).

Two fixes are blocked on the same thing: dropping `m2` when rejection is off (halves to
8B/sample), and struct-of-arrays (`compute()` becomes a memcpy, not a gather — 4x win).
Blocker: `RejectionPlugin::blend_incremental`'s cross-crate `&mut [IncrementalPixel]`
signature — either fix breaks Pro and needs both repos moved together.

### Pipeline performance instrumentation

`--span-timings` logs every stage span's duration on-device. Per-frame/payload work belongs at
`info_span!`, not `debug_span!`, or it's invisible.

A span with large self time and no children is a **blind spot**: these were added because the
residue (duration minus children) was the largest thing in a production trace:

| Span | Inside | Separates |
|---|---|---|
| `wb_grid`/`wb_apply` | `background_neutralization` | estimate vs. application |
| `blend_pixels` | `add_frame` | Pro's rejection blend (was unspanned) |
| `resample`/`row_tail` | `frame_to_rgb8` | input-scaled gather vs. output-scaled tail |
| `publish_state` | render/stacking iteration | async-lock overhead off-tokio |

`camera_capture` has `call_us` + a **signed** `overhead_us` instead (one opaque vendor
call, no seam for a child span). Signed because the saturating unsigned version read `0`
on the continuous path it was added for — `get_video_data` returns an already-completed
frame in under one exposure. Negative now means the frame was already waiting.
`process_preview_frame`'s render tail is fused, so it carries no per-sub-stage span.

`--features telemetry` adds histograms `frame.{capture,debayer,stack,render,encode_jpeg}_ms`
and counters `frame.published/dropped/render_skipped`, plus per-channel
`pipeline.queue_depth`/`queue_capacity` gauges. The **drop rate**, not the count, is what
the UI shows: `AppState::drop_rate()` divides by `delivered_frames`, because 40 drops is a
ruined evening at 30s subs and a rounding error at 100ms.

Rules: stage granularity only; cache instruments in a `OnceLock`, never rebuild per frame.

Build `--profile profiling` for `perf`; rayon threads there show as `tokio-rt-worker`.
