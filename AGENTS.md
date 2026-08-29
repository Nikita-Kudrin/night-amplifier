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

Two hard rules:

1. **Every case must report ≥~100 ms.** Below that, criterion's own overhead and the
   machine's thermal management move the number more than a real regression does.
2. **Every bench binary must stay ≤~30 s wall clock** so CI stays usable.

Techniques to satisfy both:

| Situation | Technique |
|---|---|
| Routine is pure (`&Frame` in, fresh value out) | Repeat `REPS` times inside `b.iter`, declare `Throughput::Elements(REPS * n)`, suffix the case name `_xN`. See `debayer_benchmark`. |
| Routine mutates its input in place | `iter_batched_ref` over a `Vec<_>` of `REPS` clones — plain repetition would feed iteration N+1 iteration N's output. See `render_benchmark`. |
| Input size is itself unrealistic | Resize to what production actually produces (e.g. one sensor plane, not an arbitrary megapixel buffer). |
| Small case only re-measures a bigger sibling's kernel | Delete it. |

Keep the binary inside budget with `sample_size(10)`, ~500 ms warm-up, and a 1–2 s
`measurement_time`. Always set `group.sampling_mode(SamplingMode::Flat)` — criterion's
default Linear scheme runs 55 iterations per case at `sample_size(10)`, which is enough
on its own to blow the 30 s budget.

- Suffix the case name (`_x5`) whenever the reported `time:` covers more than one call —
  a reader who divides by the wrong number is worse off than one with no benchmark.
- Setup must not leak into the measured region *or* dominate wall clock: `iter_batched`
  excludes setup from measurement but still runs it every iteration, so hoist anything
  reusable (e.g. a stacker built once) outside the loop instead.
- Feed inputs via `iter_batched_ref(.., BatchSize::LargeInput)`, never `frame.clone()`
  inside `b.iter` — a full-frame clone can dominate the reported figure on its own.
- Watch for workloads that drift across iterations (e.g. an op applied to its own prior
  output, so iteration N stops measuring the same thing iteration 1 did).

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
  It borrows the handle out of `AppState.active_camera` for the duration of each bounded call, via one
  reusable FFI worker thread (`monitor::FfiWorker`) rather than a thread per poll.

### Handle ownership — non-obvious and load-bearing

**A vendor close takes a device *index*, not a handle** — `POACloseCamera(0)` / `ASICloseCamera(0)` /
`SVBCloseCamera(0)` close whatever occupies index 0 *at that moment*. A stuck FFI call gets handed to a
detached thread whose `Drop` can fire minutes later, closing a camera that has since reconnected.

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

One detector, `server::camera_health`: one `PERSISTENT_FAULT_THRESHOLD`, one streak
(`AppState.consecutive_watchdog_timeouts`) fed by the capture watchdog, status-poll watchdog and
monitor alike, so a fault alternating between call sites still escalates. The streak ages out
(`FAULT_STREAK_TTL`) rather than resetting on success, so intermittent faults can't hide behind
occasional good polls.

`camera_session::reconnect` owns recovery policy — bounded attempts, exponential backoff,
re-enumeration before each try, and a liveness probe before declaring success.
`finalize_disconnect` takes a `DisconnectCause`, not a bool: a warmup teardown must not be reconnected.

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
WebSocket handlers.

```
Magic "SA10" (4B, 0x53413130 LE) | Width u32 LE | Height u32 LE | Payload size u32 LE | JPEG bytes
```

#### Demand-driven resolution tiers

Clients send `{width, height}` JSON. The tier is selected from the viewport's **shorter
edge** clamped to 1080…2160 (its display-resolution class), not by fitting both edges into
a box — this is deliberate: testing both edges against a 16:9 box would put a portrait
phone in the 4K tier at 2× the bandwidth, and rotating a device must not change its tier.

| Tier       | Bounding box  | Serves class | IMX464 (2712×1538) output |
|------------|---------------|--------------|---------------------------|
| `Hd1080`   | 1920×1080     | ≤ 1080       | 1904×1080                 |
| `Qhd1440`  | 2560×1440     | ≤ 1440       | 2539×1440                 |
| `Uhd2160`  | 3840×2160     | ≤ 2160       | 2712×1538 (no downsample) |
| `Original` | unbounded     | —            | 2712×1538                 |

`clamp_client_resolution` clamps per axis, but only inside `encode_rgb8_jpeg_dynamic`; tier
selection doesn't go through it.

Each tier tracks its client count (`AppState.jpeg_tier_clients`, `JpegTierClientGuard`). The
render task encodes one payload per tier that has clients and caches it in
`AppState.jpeg_tier_cache` (tiers that don't downsample the frame share one encode — for
sub-4K sensors that's `Uhd2160`/`Original`). Handlers wake on `frame_ready` and write the
cached payload — no per-client encoding — except a client that just connected or changed
tier, which encodes once inline so its view isn't blank, then publishes that for others on
the tier.

Publication is race-free by construction: the render task calls `begin_frame()` to claim the
counter, stores every payload against it, then `publish_frame()` to wake clients — so a woken
client never sees a counter whose payloads are still missing.

### Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`

Lossless (unquantized-beyond-8-bit) path for the eyepiece quality view.

```
Magic "SA08" (4B, 0x53413038 LE) | Width u32 LE | Height u32 LE | Compressed size u32 LE | LZ4 RGB8 payload
```

SA09 is the chunked variant (parallel LZ4 compression). Frontend renders via WebGL with
Canvas2D fallback.

#### It is resolution-negotiated, not full-resolution — and that is a noise decision

This stream used to hardcode a 3840x2160 box and ignore the client's viewport. It now
takes a `{width, height}` report like the JPEG path, maps it through the same `JpegTier`,
and box-averages down to it.

**The resampling is the point, not a bandwidth saving.** A server-side box downsample is an
area average, so it removes noise in proportion to the reduction. The browser's fallback is
`TEXTURE_MIN_FILTER = LINEAR` with no mipmaps, which takes four taps however far it is
minifying — capped around 1.45x of noise reduction and discarding the rest as aliasing.
Measured on `250mm-dob-imx533-dumbbell-fits`, encoding into a 1440 tier rather than the 4K
cap takes sky sigma from 8.90 to 7.41 output levels and the payload to 2.25x smaller;
`display_output_tests` reports both figures.

Two structural consequences:

- **Client accounting is per tier, via `StreamKind`.** `lz4_clients` still only answers "is
  anyone watching"; `lz4_tier_clients` carries the viewport. The stream keeps *one* payload
  rather than one per tier, so `AppState::lossless_target_box` serves the **largest** tier
  any client asked for, and falls back to the 4K cap when nobody has reported one — which
  is exactly the old behaviour for a client that never sends a viewport.
- **The shared native RGB8 buffer is no longer unconditionally reusable.** The render task
  builds one conversion for LZ4 and the non-downsampling JPEG tiers to share. Once the
  lossless stream downsamples, reusing it silently ships a native-size payload and undoes
  the whole change; `lz4_downsample_does_not_reuse_the_shared_native_buffer` pins that.

The frontend reports the **canvas** size, not the window size. In binoview each eye canvas
shows the whole frame at roughly half the window width, so reporting the window doubles the
pixels either eye can use — that difference is 1.20x against 2.19x of grain reduction.

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

Every provider used to debayer inside its capture shim, so the first thing any pipeline
code saw was already RGB and there was nowhere to put a correction that is only defined on
the mosaic. `RawFrame::to_cfa_frame` now yields a `CfaFrame` — still mosaiced for a colour
sensor — and the **stacking task** owns the demosaic, after running a `CfaPipeline` over it.
`RawFrame::to_frame` is kept as `to_cfa_frame` + an empty pipeline + a bilinear demosaic,
which is the pre-seam behaviour; `the_raw_stage_with_no_registered_corrections_is_bit_identical`
pins the two together.

Three corrections need this seam, and `Calibration` / `MasterDark` / `MasterFlat` are the
reason it exists at all — they are fully implemented and still have no call sites in
`src/server/`, because `(raw - dark) / flat` is defined on raw samples. It lands as one more
`Box<dyn CfaStage>`.

- **Both filters work one colour site at a time.** `CfaFrame::planes()` describes the
  sub-lattice: four sites at `step` 2 for a Bayer mosaic, one at `step` 1 for mono, so
  neither filter needs a separate mono path. Mixing sites is the bug this prevents — an R
  sample and the B sample beside it sit at different levels, so a filter that treats them
  as neighbours reads the mosaic itself as signal.
- **`cfa::hot_pixels` is one-sided *and* isolation-gated, multiplicatively.** The plain
  `centre - max(8 neighbours) > tau` form still clips the core of a bright star: 38 % of a
  200-sigma peak is 76 sigma. What separates a defect from a PSF regardless of brightness
  is the *fraction* of the centre's amplitude its brightest neighbour carries — a star keeps
  60 % or more one sample out, sky beside a hot pixel keeps a few per cent. Max-of-eight
  rather than a median-of-9 sorting network: the brightest neighbour *is* the second-brightest
  of the 3x3 whenever the centre is the brightest, which is the only case the filter acts on.
  No de-interleave into planar buffers either — row triples `step` apart with stride-`step`
  reads hit the same cache lines without two 36 MB copies per frame.
- **`cfa::fpn` levels each row then each column against the median of the line medians**,
  not against a whole-site median: offsets built that way sum to zero, so the correction
  cannot shift the frame's overall level and change what the autostretch solves for.
  It is **skipped for `StackingType::Planetary`** — a lunar disc fills enough of each line
  to move its level, and flattening that carves bands across the disc.
  On the IMX533 fixture it subtracts ~5.8 ADU RMS per row and ~6.2 per column, matching the
  measured excess; `cfa_tests` reports how much of that is genuinely line-to-line (about a
  quarter of the row figure, and none of the column figure) versus smooth structure.
- Detection is separated from application in both filters, so a corrected sample never feeds
  the test for one of its neighbours and the result does not depend on how rayon split the rows.

`server::capture::pipeline::{build_cfa_pipeline, debayer_algorithm, convert_captured_frame}`
own the settings-to-stages mapping; `cfa/` itself knows nothing about `CaptureSettings`. The
probe frame in `task.rs` sizes the pipeline's channels through the *same* call, because
superpixel debayering changes the frame size by 4x.

### Frame memory layout (planar) — non-obvious and load-bearing

`Frame` stores samples **plane-major**: `idx = channel * width * height + y * width + x`
(`RRR...GGG...BBB`, not `RGBRGB...`) — this is what lets SIMD and spatial filters read a
channel as a contiguous run instead of a stride-3 gather.

**`Frame` is planar; every 8-bit output format is interleaved.** Crossing that boundary
wrongly still compiles and produces an image whose channels collapse toward grey. This has
already happened simultaneously across `to_rgb8_fast`, `render::frame_to_rgb8`, the PNG
writer, the SER writer/reader, and the Pro saturation/comet plugins — none of which the
compiler or test suite noticed.

| Consumer | Layout |
|---|---|
| JPEG (SA10), LZ4 (SA08/SA09), PNG, SER `Rgb`/`Bgr` | interleaved |
| FITS (NAXIS3 = 3) | planar — passes straight through |

Rules:

- Use `planes()` / `planes_mut()` / `channel_data()` / `get_pixel()`. A new `frame.data()`
  next to `* 3` or `* channels` is a review flag. `planes()`/`planes_mut()` **panic**
  unless `channels() == 3`.
- Build fixtures with `set_pixel`, never hand-computed offsets — a fixture that encodes
  the layout can't catch a layout bug in the code under test.
- `src/frame/layout_tests.rs` pushes distinct constant channels through every output
  path (raster and downsample-fused traversals, all supported bit depths and pixel
  formats). Add a row when you add a format — a uniform-grey fixture proves nothing here,
  it's layout-invariant by construction; use `tests/integration/common.rs`'s
  `mean_chroma_spread_*` to assert on real fixture data instead.
- **Cover each traversal separately, not just each format** — e.g. `ser/writer.rs`'s
  8-bit and 16-bit encoders gather planes independently, so a gap in one's coverage
  doesn't show up via the other.
- **8 bits round, 16 bits truncate.** Every 8-bit consumer must go through
  `frame::sample_to_u8` (`* 255.0 + 0.5`), never an open-coded multiply — a local
  truncating lambda in one SER arm once disagreed with every other 8-bit path by 1 LSB.
- Sweep constant-plane fixtures over the whole interior, not one pixel — an interleaved
  write can land on the exact offset a planar read expects for some positions, letting a
  spot-check pass against an otherwise-scrambled buffer.
- Partition rayon work with `parallel::balanced_chunk_len`; **never derive a channel
  index from a flat chunk index** — that forces the chunk length to divide the plane
  size, which gets expensive to compute or, worse, silently collapses to a single chunk
  (no parallelism) when it doesn't divide evenly. Dispatch per plane instead; see
  `render::black_point::subtract_black_point`.
- **`get_pixel` in a whole-frame loop is a review flag too, not just a correctness one.**
  It's fine for fixtures and spot checks, but wrong for a traversal that visits every
  sample — planar layout already hands you the contiguous run that makes the loop a slice
  copy. `render::white_balance::block_medians` was the one hot consumer this was missed
  in: per-sample `get_pixel`, per-block `Vec` allocations, and no rayon cost it **120 ms
  per preview frame** (90% of the whole pipeline budget) until rewritten onto `planes()`
  plus `into_par_iter` over grid blocks (27 ms, bit-identical output).
- **Grid-node background sampling lives in `background::grid`**, shared by Community's
  `background::extractor` and Pro's `plugins::rbf` (previously duplicated, each reaching
  pixels via hand-computed offsets). Thresholds the two genuinely disagree on pass in via
  `PruneConfig`; grid *placement* stays per-crate.

### The f32 -> 8-bit boundary (`render::output::quantize`)

Every byte that reaches a screen crosses this boundary exactly once: the tails of both
fused kernels in `server::encoding::format` and `render::frame_to_rgb8`. All three go
through `write_row_rgb8` / `write_pixel_rgb8`, which wrap the canonical `sample_to_u8`.
Keeping them on one helper is the same rule as the rest of the layout contract — parallel
8-bit conversions in this repo have drifted by an LSB before.

`DisplayOutput` carries two things, both defaulting to off (`DisplayOutput::PLAIN` is
byte-identical to a bare `sample_to_u8`, which is what makes the feature safe to add to a
path everything already uses):

- **`pedestal`** maps `[0, 1]` onto `[pedestal, 1]`. The autostretch black point is
  `mode - black_point_sigma * sigma` with a hard clamp at zero, so a real stretched frame
  lands ~0.8 % of its samples on exactly 0 (measured). An OLED switches those fully off,
  and at ~1.7 arcmin per pixel through an eyepiece they read as black speckle rather than
  as sky. Reached from `EyepieceSettings::black_floor`.
- **`dither`** adds a sub-LSB ordered-dither offset **before** rounding, so the expected
  output equals the true value and quantization error becomes a pattern the eye integrates
  away. This replaced an implementation that added +/-8 LSB to the *already-rounded* byte,
  which recovers no sub-LSB information and is simply visible crosshatch — and which was
  unreachable from the streaming path anyway.

Two things are easy to get wrong here:

- **Index the dither in output coordinates.** A pattern applied before resampling is
  averaged into mush by the downsample. Both fused kernels hold the output row index; the
  flat-run traversal in `frame_to_rgb8` recovers it from the absolute pixel index.
- **The matrix is 8x8, not the conventional 4x4.** At the eyepiece a 4x4 cell's ~7 arcmin
  period sits inside what the eye resolves, so it reads as texture instead of disappearing.

`black_point_sigma` is the only parameter in the current design that moves displayed sky
grain: the MTF solve pins `mtf(black_point_sigma * sigma) -> target_background`, which is
scale-invariant, so grain amplitude does **not** fall with stack depth or exposure. The
eyepiece intensity slider interpolates it *upward* (toward `EYEPIECE_BLACK_POINT_SIGMA`)
for that reason; it used to interpolate downward under a comment claiming it clipped noise,
which left more grain visible and clamped more sky to pure black.

### Phase 2: Debayering (Demosaicing)

Converts mono Bayer pattern (CFA) data into full RGB color.

- Auto-detects patterns (RGGB, BGGR, GRBG, GBRG).
- Bilinear Algorithm: Fast interpolation for live preview or less critical data.
- VNG (Variable Number of Gradients): High-quality interpolation avoiding color artifacts on edge transitions.
- Superpixel: one RGB pixel per 2x2 quad, at half the width and height. Interpolates nothing,
  so it invents no chroma noise and keeps a surviving hot sample inside one output pixel.
  Opt-in (`sensor_correction.superpixel_debayer`) because it is free only on a sensor that
  already oversamples the display — IMX533's 3008² lands at 1504², above a 1440² eyepiece
  screen; IMX464's 2712x1538 lands at 1356x769, below it. It is a **separate traversal** from
  the two interpolating kernels, so `layout_tests` carries its own row for it.

Non-obvious invariant, source of a fixed GRBG colour bug: at a green pixel, whether red interpolates
horizontally or vertically depends on the **row only, never the column** — a green pixel's row is
either red-and-green or blue-and-green, and its horizontal neighbours follow from that. This is why
`get_rb_orientation` keys on `y & 1` alone, so RGGB/GRBG share an arm (as do BGGR/GBRG) despite
opposite green column parities; keying on `x` too used to misroute GRBG's odd-row greens, filling blue
from red neighbours across a quarter of every frame. `test_debayer_reproduces_constant_colour_planes`
pins this for all four patterns — keep it passing rather than adjusting it.

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
