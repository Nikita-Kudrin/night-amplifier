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

- Files over 500 lines: consider refactoring/extracting.
- Follow standard Rust conventions for backend, JavaScript for frontend.
- Write tests for new or changed functionality.
- Don't write obvious comments — code should be self-describing; comment only non-obvious behavior.
- Keep comments, doc comments, and prose summaries (module docs, AGENTS.md sections) to 5-10 lines per point: state
  the non-obvious "why" and any load-bearing numbers, cut restated context and hedging. If an explanation is still
  ballooning past that, split it or link out rather than let it sprawl. Exempt: runnable doctest/example code,
  wire-format/byte-layout tables, and other structured reference data (tables, one-fact-per-line lists) — tighten
  wording there without breaking the structure that makes them scannable.
- Avoid expensive operations; never run extremely heavy ones on the fly.
- Optimize imports and remove unused code you created.
- Avoid deep nesting — use if+return to simplify.
- ALWAYS prefer editing an existing file over creating a new one.
- Update docs (*.md, especially the VitePress manual in `manual/`) after code changes, concisely.
- After big changes, run backend and frontend tests.

## Architecture

When designing new features or refactoring, adhere to the following architectural principles:

- Clean/Hexagonal Architecture: isolate core domain logic from UI, DB, and third-party APIs via interfaces/ports.
- Domain-Driven Design: group code by feature/domain, not technical role (controllers/models/services).
- Separation of Concerns: each module/class/function has one well-defined responsibility.
- Asynchronous Communication: favor event-driven (Pub/Sub, queues) for long-running or cross-service work.
- Design for Failure (Resilience).
- Plugin system: performance-critical / Pro-only logic lives behind traits (`REJECTION_PLUGIN`,
  `PUSH_TO_PLUGIN`, `COMET_PLUGIN`, `BACKGROUND_PLUGIN`, `PLANETARY_STACKER_PLUGIN`, `DENOISE_PLUGIN`,
  `AI_DENOISE_PLUGIN`) so Community works standalone.
- f32 normalization: all pixel math uses [0.0, 1.0] to prevent overflow.
- Rayon for multi-core processing; no allocations in hot paths (pre-allocated buffers where possible).
- ARM friendly (optimized for Raspberry Pi 5); FFI safety — all C/C++ calls wrapped with `catch_ffi_panic`.

# Test Guidelines

If you can't fix a test, don't simplify it into not testing the idea. Tests can take a minute or two; benches longer.
**Never run benchmarks alongside other tests/tasks** — it skews the numbers. Real-data fixtures live in
`DEFAULT_FIXTURES` (`tests/integration/common.rs`), download on demand; a test wanting one calls
`stack_depth_grain_tests::managed_session`, which **panics** rather than skip. Measurement instruments
(`tests/integration/instruments.rs`) are shared with Pro via `#[path]` — no `crate::`.

## Benchmark sizing

Every case ≥~100ms (below that, overhead/thermal noise dominates); every bench binary ≤~30s. Repeat pure routines
`REPS`× in `b.iter` (suffix `_xN`); for in-place mutation use `iter_batched_ref` over `REPS` clones, never
`frame.clone()` inside `b.iter`. Match production input size/shape, and bench whole-pipeline sums too, not just
per-stage.

## Build & Test

**Prerequisite:** `nasm` (for `turbojpeg-sys`/libjpeg-turbo SIMD). `dev` profile is `opt-level = 1` in both repos
(image tests ran ~7x slower at 0) — expect optimised-out locals in a debugger.

```bash
cargo build --release
cargo test                                                          # fast unit tests
cargo test --test integration_pipeline -- --ignored --test-threads=1 # integration (slow, ignored by default)
cargo bench --bench <name> -- --noplot                              # see Benchmark sizing
cargo run --release -- [port]
cargo run --release --features telemetry -- --telemetry
cargo run --release -- --span-timings                               # per-stage durations on span close
```

Frontend commands (install/dev/build/lint/test) run from `web/` — see README. **Never `npm run format`** (rewrites
the whole tree). Always run `cargo test` after changes, `npm run test:run` too.

## Core Modules (src/)

| Module             | Purpose                                                                       |
|--------------------|--------------------------------------------------------------------------------|
| `frame/`           | `Frame` with normalized f32 pixels; format conversion                         |
| `fits/`            | FITS read/write; NAXIS layout                                                  |
| `debayer/`         | RGGB/BGGR/GRBG/GBRG; Bilinear + VNG + Superpixel                               |
| `cfa/`             | Raw-CFA stage before demosaic: hot pixels, row/column FPN                      |
| `render/denoise/`  | Denoise plugin boundary: config data, buffer pool, `ai_compute`                |
| `calibration/`     | Master dark/flat: `(raw - dark) / flat`                                       |
| `detection/`       | Star detection, CoM centroiding, FWHM/SNR                                      |
| `registration/`    | Triangle matching + RANSAC → `AffineTransform`                                 |
| `stacking/`        | `MasterStack` accumulator, rejection, warping                                  |
| `background/`      | Grid-based gradient extraction                                                 |
| `render/`          | Stretch, autostretch solver, white balance, S-curve, output; `statistics/` for robust median/MAD |
| `camera/`          | Traits + vendor SDKs + simulator                                               |
| `planetary/` / `ser/` | Correlation alignment + percentile stacking; SER video read/write           |
| `disk_writer/`     | Async bounded-queue frame writer                                               |
| `plugins/` / `push_to/` | Pro-delegated trait definitions, incl. Community-side Push-To (impl in Pro) |
| `server/`          | Axum REST + WebSocket server                                                   |
| `app.rs` / `parallel.rs` | Shared `app::run()`; `balanced_chunk_len` rayon partitioning, both shared with Pro |
| `ffi_safety.rs` / `native_library.rs` | `catch_ffi_panic`; eager dlopen of vendor libraries             |
| `logging.rs` / `telemetry.rs` | `tracing` + optional OpenTelemetry (OTLP)                           |

### Server & Web Frontend (src/server/, web/)

Axum: REST `/api/*`; WS `/ws/stream` + `/ws/eyepiece` (JPEG), `/ws/eyepiece_quality` (lossless LZ4), `/ws/events`
(JSON). `/ws/events` opens with `state_changed` + `camera_phases` and resends both after a `Lagged` client; `GET
/api/cameras` carries `phase`/`warmup_remaining_s` for every client, not just the one that acted.
`GET /api/eyepiece/snapshot?circular=` is the only REST route returning image bytes; native size is costly, so
`SNAPSHOT_SLOT` *refuses* rather than queues (503 + `Retry-After`).

Vue 3 SPA, mobile-first, dark theme, proxying `/api`/`/ws` to `localhost:9955` in dev. Pro-only controls stay
visible and locked (`BaseProLock`) on `/api/capabilities`. `useCatalogSearch` skips a programmatic query by
*value*, never a one-shot flag — a flag-based version once left M1-M9 unfindable after clearing a 1-character query.

## Camera Notes

- **Camera roles**: at most one `Main` and one `Guide`; every handle/monitor/cancel-token lives in
  `AppState.camera_slots[role]`. `connect` to a taken role swaps while idle, else `CameraRoleBusy`.
- **Cooler lifecycle**: `Precooling → Idle → Capturing|Guiding → WarmingUp`, ramped ≤5°C/min. **Disconnect** stops
  capture first and cuts an in-flight sub short (≤15 s, stack saved) before closing the cooler — Stop alone once let
  a 300 s sub outlast it; a watchdog ends every warm-up by deadline regardless of the monitor.
- **Per-camera profiles** are keyed `"{provider}/{model}"`; **telescope profiles** by camera *name*, seeding the
  solver's FOV — without a seed, two bodies of one model share one FOV and ASTAP fails to solve (once cost 19 min).
- **Vendor SDKs are dlopen'd, never linked or shipped**: an unused `#[link]` breaks every build without that SDK,
  and QHY grants no redistribution. QHY/ToupTek/SVBony bind eagerly, since a lazy unresolved symbol kills the
  process at first call. C `long` is 32-bit on Windows, widen with `i64::from`.
- **Guide camera is one thread, not the pipeline**: nothing stacked/queued, started by `connect` so solving/preview
  work *while* framing. Post-processing runs only while `guide_stream.has_viewers()`; solving/raw saving sit above
  both early exits. Two `FrameStream`s, two counters, so one camera's payloads can't invalidate the other's.
- **Vendor closes take a device *index*, not a handle** — a stuck call's late `Drop` can close a reconnected
  camera; every close goes through `lease.begin_close()`, and an abandoned handle is never closed eagerly.
  `{provider}_{index}` ids follow USB enumeration order, so a guide reconnect can install the *imaging* camera —
  recovery keeps a device's **recorded id** even if the index moved.
- **Fault detection**: one detector with a per-role+name streak serves all watchdogs, so alternating faults still
  escalate. Recovery is a ladder (stream restart → quiet suspend → supervisor retry → give-up), silent before 20 s;
  a failed hand-off always resumes the monitor. Only a *named, different* camera discards the FOV cache, since a
  stale FOV *fails* hinted solves.
- **Focus/Finder mode** forces six settings off, recorded only in `focus_mode_snapshot` — **entering must be
  idempotent**, or re-snapshotting destroys the observer's values. **Mutually exclusive with an accumulating
  stack**: starting/resuming a stacking capture drops the mode instead; *leaving* is never refused.

## Push-To gating (`capture::solving`)

Two slots: `try_begin_solve` may block for a whole ASTAP ladder; `try_begin_watch` runs *during* one so a slew is
noticed and a doomed search abandoned. Loops offer frames via `offer_plate_solve` to two tasks
(`push-to-solve`/`push-to-watch`), each running the plugin under `rt.block_on`. A frame goes only to an *idle*
consumer and is dropped otherwise — never queued (spawning a task per offer once kept 27 of 32 frames alive and
stale). `PushToBlocker` announces why nothing is happening, one event per transition; the UI ranks a live blocker
above the last verdict.

## Storage Formats

| Output           | Format | Bit Depth       |
|------------------|--------|-----------------|
| Raw frames       | FITS   | 16-bit unsigned |
| Stacked image    | FITS   | 32-bit float    |
| Stacked preview  | PNG    | 8-bit           |
| Planetary frames | SER    | 16-bit unsigned |

**The stacked preview PNG goes through the live-view encoder, not the render pipeline**, so denoise and
`DisplayOutput` pedestal/dither still apply, and it carries the stack's depth *and* coverage map — without the map
a drifting stack's border saved grainier than it streamed.

SER (planetary) is uncompressed with per-frame timestamps; color id 0 is Mono, 8-11 are BayerRGGB/GRBG/GBRG/BGGR,
100/101 are RGB/BGR. Layout: `captures/raw/DD-MM-YYYY_HH-MM-SS-<mode>/frame_NNNNNN.fits` (or `capture.ser`) and
`captures/stacked/...-stacking.fits`. **A resumed capture rejoins its folder and must not overwrite it**: the
resume plan carries `next_frame`, and video writes `capture_2.ser`, etc.

## Streaming Protocols

### Dynamic JPEG (SA10) — `/ws/stream`, `/ws/eyepiece`

Default format; TurboJPEG (SIMD) encodes in the render task, not the WebSocket handlers. **Quality follows the
denoisers**: 95 below 1440p, 90 above, but 95 at every size once denoise is on — q90 would erase what the dither
and denoiser add.

```
Magic "SA10" (4B) | Width u32 LE | Height u32 LE | Payload size u32 LE | JPEG bytes
```

**Streaming resolution is a setting, not negotiated**: every client of a family gets the **same payload**
(`streaming_resolution` / `eyepiece.stream_resolution`, both default 1440p) — per-viewport tiers were tried and
reverted (one denoised conversion per distinct size plus unbounded concurrent first-frame encodes).

### Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`

Lossless (beyond 8-bit) for the eyepiece quality view; SA09 chunks it for parallel LZ4; WebGL with Canvas2D
fallback.

```
Magic "SA08" (4B) | Width u32 LE | Height u32 LE | Compressed size u32 LE | LZ4 RGB8 payload
```

**Downsampling** area-averages down to the configured box (`encoding::axis_taps`: a 1 px tent per source sample,
not a whole-pixel box, which once averaged a different sample count on different lines and left a visible
lattice). WebGL needs `UNPACK_ALIGNMENT` 1 for the frontend's unpadded RGB rows.

## Adding a Stacking Type / Settings Persistence

Add a `StackingType` variant (`src/stacking/config.rs`), update `all()`, and implement its capability methods
(`display_name`, `uses_star_registration`, `uses_fpn_removal`, etc.) — no changes needed in `capture.rs`. Settings
persist as `settings.json` in the server working directory, loaded on startup, saved on `POST /api/settings`.

## Full Image Processing Pipeline

Multi-phase linear/non-linear pipeline: calibration → raw-CFA → debayer → detection → registration → stacking →
background → statistics → auto-color → black point → saturation → stretch → autostretch solve → output/contrast.

### Phase 1: Sensor Data Acquisition & Calibration

**Master Dark/Flat**: `calibrated = (raw - dark) / flat`, in 32-bit float, correcting thermal noise and
vignetting/dust/illumination.

**The raw-CFA stage** (`cfa/`) runs before demosaic, one colour site at a time. **`hot_pixels`** is gated on the
*fraction* of centre amplitude the brightest neighbour carries (not a raw diff), so star cores survive — it's
**unconditional** (no setting), since without it bilinear turns each hot pixel into a star-sized blob and the
solver fails. **`fpn`** levels each line against a narrow even-order average of its own neighbours.

**Frame memory layout is plane-major** (`idx = channel*w*h + y*w + x`); every 8-bit output format is interleaved —
crossing wrongly still compiles and greys the channels. Use `planes()`/`channel_data()`/`get_pixel()`, never
`frame.data()` with `* channels` math.

### Phase 2: Debayering (Demosaicing)

Bayer mosaic → full RGB; auto-detects RGGB/BGGR/GRBG/GBRG. **Bilinear**: fast, live preview. **VNG**: higher
quality, no edge artifacts. **Superpixel**: one RGB pixel per 2x2 quad, worth it only when the sensor oversamples
the display. **Invariant**: at a green pixel, red interpolates by **row only, never column** — keying on column
once misrouted GRBG's odd-row greens over a quarter of every frame.

### Phase 3: Star Detection & Centroiding

Estimates local background (Median/MAD), thresholds to find local maxima while rejecting isolated hot pixels,
locates sub-pixel coordinates via Center of Mass, and computes FWHM/SNR.

### Phase 4: Image Registration (Alignment)

**Deep Sky**: scale/rotation-invariant triangle patterns + RANSAC → `AffineTransform`. **Planetary**: surface
feature cross-correlation in an ROI (no stars needed). **Comet** [Pro]: centroids the nucleus so stars trail while
the comet stacks sharp.

### Phase 5: Live Stacking & Rejection

`MasterStack` accumulates in O(1) memory (16 B/pixel, 434 MB at 3008²x3). **Never estimate the clip threshold from
samples that survived the clip**: an early underestimate once rejected the samples that would have widened it,
discarding 15% of real subs. `m2` is now a running mean of squared deviations over *every* offered sample, rejected
ones winsorised to the threshold — cosmic rays can't widen the window, but a collapsed scale still recovers
geometrically. `RejectionMethod` has four variants but the live path implements only two (`MinMax` would need
per-pixel min/max, +650 MB); the plain-mean path maintains `m2` too, so a settings toggle lands mid-stack without
opening a gap.

### Phase 6: Background Extraction (Light Pollution Removal)

**Crowded targets stay out of both models** (bilinear and Pro's RBF): brightness pruning seeds a disc around a
frame-filling halo (capped, refilled from the outer surface) so the target can't become its own reference. The
surface is a **plane unless a quadratic halves the outer nodes' residual RMS** — vignetting is a dome a plane
reads as excess.

### Phase 7-8: Statistics & Auto-Color

Robust per-channel statistics (Phase 7) feed background neutralization of light-pollution colour casts (Phase 8).

### Phase 9: Black Point Calculation

`black_point = mode - k * sigma`, so the sky estimate must resolve far finer than the sky itself. Reporting a
histogram bin's *centre* made the mode a step function of depth, once snapping a whole bin and blacking out half a
target — **the binned peak picks the region; the value is refined** by re-binning inside the winning bin. The sky
level comes from the **samples** the refined peak points at, whose own spread uses the samples **below** the peak
only (their *lower quartile*) — safe *upwards* against a frame-filling target and *downwards* against a population
darker than the sky.

### Phase 10-12: Saturation Boost, Tone Mapping & Autostretch Solver

Optional shadow saturation boost, then the core asinh/MTF stretch. **Asinh** solves for `stretch_factor` such that
`input = adjusted_median` maps to `output = target_background` (default 0.15), via Newton-Raphson/Bisection. **MTF**
solves algebraically for `m`. Steps: compute statistics → black point `= Median - c×Sigma` → solve the tone
parameter → subtract black point → apply the stretch.

### Phase 13: Final Output Mapping & Contrast

Everything below runs in the streaming encoders at display resolution, not in the pipeline proper.

**Spatial denoising** (Pro plugin behind `DENOISE_PLUGIN`): guided chroma + à trous wavelet luma, tuned in Pro
(`plugins::denoise`). Community owns the boundary — `DenoiseConfig` (`Default` is always `OFF`, byte-identical
output without the plugin) and the gates: Planetary is refused before the plugin is asked; the switch is unmanaged
by Focus/Finder mode. The **grain-split dial** (`background_grain`) spends three levers in order — below the
middle only the cheap finest wavelet level moves (fine speckle), above it the coarse pair (levels 5-6) takes over,
the only mechanism reaching 16-64 px noise; the dial's own tone-curve split never runs past `DEFAULT_GRAIN_SPLIT`
(1/8, not an even 1/4), since spending more there costs target brightness 1:1 with the grain removed. **Detail**
(levels 2-4 local contrast) ships on by default, held off the sky/stars by the plugin.

**The AI denoiser** (Pro plugin behind `AI_DENOISE_PLUGIN`) is display-referred, not linear — it runs after the
stretch, before saturation/S-curve/floor (trained on stretched RGB; after the S-curve it over-softened stars).
Gated like the classic filters; its compute-unit benchmark never runs under a capture (Pro AGENTS.md "AI compute").

**The stack's noise map** (`frame::NoiseField`) carries **coverage, never the full variance** — the variance's
brightness term fattened every star, so the per-frame path carries only a 1/8-grid coverage plane. Coverage counts
subs that *reached* a pixel, not subs *kept*, or sigma clipping breaks its bit-identity.

**The f32 → 8-bit boundary** (`sample_to_u8`): `dither` uses **64x64 void-and-cluster blue noise**, not an ordered
matrix, since an 8x8 Bayer tile once quantised a smooth sky into a visible lattice — judge it after the encoder,
since JPEG takes most of it back. `black_point_sigma` is scale-invariant; `depth_grain_gain` scales it by `N^s`, so
**any** renderer of a stack must pass `stack_depth` through `AnalysisContext`, never `stacked_count`. The fused
scale LUT's cache key is quantised *relatively*, not by an absolute step — the latter once let two tone curves 3%
apart collide, so whichever rendered *second* silently reused the first's curve.

**The darkening half of the black floor** (`black_floor` negative) is a spatial gain from a 3x3 local mean — the
sky itself is measured in-kernel as the *darkest* histogram peak holding ≥20% of samples, not the median, or a
bright nebula over half the frame gets darkened like sky. Any pointwise darkening or soft-thresholded coarse
wavelet level digs a ring around every star; the coarse pair needs a non-negative garrote instead.

#### S-Curve Contrast (`ContrastConfig`)

**`strength` is 1.0, `midpoint` 0.2** — below the sky, which is what makes full strength affordable: the sky lands
on the curve's compressive half, the target on its expansive one, so one pass brightens the target and darkens the
sky at once. Lowering the midpoint below 0.2 washes the background out instead.

## Logging & Performance

`RUST_LOG` overrides levels; `tracing` + daily file rotation. The file layer uses its own `PlainFields` formatter,
since tracing-subscriber caches formatted fields per formatter *type* — sharing one with the coloured console once
leaked ANSI escapes into the file. **Startup system report** (`system_info`) logs build/host/CPU/memory facts as
INFO events, self-contained, collected on `spawn_blocking` under a 5 s timeout.

A dropped *capture* frame is signal lost for good, so render/stacking favour stacking: the display copy is gated
by `want_display` but the frame always stacks; the capture→stacking queue is **latency-bounded** at 2s of
exposures; per-stack estimates (white balance, background) are reused and refreshed only on proportional depth
growth.

**A static sky must render the same frame after frame**: live view re-solves everything every frame, which is what
holds it still; stacked view keeps one consistent snapshot and carries its MAD forward along a noise trend instead
of re-measuring fresh each frame; signal-fraction gates are **bands**, not steps, so a field living in one still
renders its own small jitter rather than a hard curve-flip. `IncrementalPixel` (16B/sample, 434MB at 3008²x3) is
read+written whole every frame — already the memory ceiling.

`--span-timings` logs every stage span's duration (per-frame work belongs at `info_span!`, not `debug_span!`, or
it's invisible); `--features telemetry` adds histograms/counters/queue-depth gauges, and the UI shows the **drop
rate** over delivered frames. Build `--profile profiling` for `perf`.
