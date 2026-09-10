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
  `PUSH_TO_PLUGIN`, `COMET_PLUGIN`, `BACKGROUND_PLUGIN`, `PLANETARY_STACKER_PLUGIN`) so Community works standalone.
- f32 normalization: all pixel math uses [0.0, 1.0] to prevent overflow.
- Rayon for multi-core processing; no allocations in hot paths (pre-allocated buffers where possible).
- ARM friendly (optimized for Raspberry Pi 5); FFI safety — all C/C++ calls wrapped with `catch_ffi_panic`.

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
(cd web && npm ci && npm run build)                                 # only needed for the two `frontend_serving` tests that assert the *real* embedded bundle; `web/dist/` is git-ignored, so without it they skip locally (and fail under CI=1, where CI builds it first). Every other frontend-serving test runs against an in-source fixture bundle instead.
# These are ignored by default and must be run explicitly:
cargo test --test integration_pipeline -- --ignored --test-threads=1 # integration (slow)
cargo bench --bench <name>                                          # benchmarks — see **Benchmark sizing** below before adding or editing one.
cargo bench --bench <name> -- --noplot                              # ~4x faster wall clock: without gnuplot installed, criterion's plotters fallback dominates the run (debayer_benchmark: 95 s -> 22 s) while the measurements are identical. Prefer this unless you want the HTML report.
cargo run --release -- [port]
cargo run --release --features telemetry -- --telemetry
cargo run --release -- --static-dir web/dist                        # serve the frontend from disk instead of the bundle embedded at build time (or NIGHT_AMPLIFIER_STATIC_DIR). Opt-in: the default is always the embedded bundle, so a binary run from a checkout cannot pick up `web/`'s Vite source template by accident.

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

Axum-based. REST at `/api/*`, WebSocket streams at `/ws/stream` and `/ws/eyepiece` (dynamic JPEG,
`?source=guide` for the guide camera), `/ws/eyepiece_quality` (lossless LZ4), and `/ws/events`
(JSON). Shared state via `Arc<RwLock<_>>` in `AppState`. See source for exact endpoints, DTOs, and
event variants.

`GET /api/eyepiece/snapshot?circular=` is the one REST route returning image bytes: it PNG-encodes
`latest_raw_frame` at the frame's *own* size (no tier), on `spawn_blocking`. RGB8 either way.
`circular=true` returns the **centre square** with the field stop **opaque black** — square because
the view's canvas is `100cqmin` + `object-fit: cover`, so masking the full rectangle gave the right
circle in a shape nobody saw (55 % padding on an IMX464); black rather than transparent because a
viewer composites alpha onto its own background, which is white in every default light theme.

Native resolution is deliberate and expensive: 26 MP with the denoisers on transiently holds ~933 MB
of denoise scratch plus two RGB8 buffers of up to 77 MB each, ~0.5 s on a desktop. `SNAPSHOT_SLOT`
(`Semaphore(1)`) therefore admits one render process-wide and *refuses* rather than queues — 503 +
`Retry-After: 2`, which the frontend retries on that cadence for 15 s. 404 is the other refusal (no
frame rendered yet) and is terminal.

The server names the file in `Content-Disposition`, but the browser never sees that header: the
frontend has to `fetch` the PNG for the retry loop, and the `blob:` URL it ends up saving carries no
headers. So `fetchEyepieceSnapshot` returns `{blob, filename}` and `utils/saveBlob.js` puts the name
on `<a download>`. That attribute is load-bearing — without it the browser navigates to the blob and
renders the PNG in the tab, taking the page's streams down with it.

### Web Frontend (web/)

Vue 3 SPA, mobile-first, dark theme. Composables in `src/composables/`, components in `src/components/`. Vite proxies
`/api` and `/ws` to `localhost:9955` in dev.

## Camera Notes

- **Camera roles**: the rig holds at most one `CameraRole::Main` and one `Guide`. Every handle,
  monitor, cancel token and reconnect guard lives in `AppState.camera_slots[role]` — there is no
  "the camera" any more, and each lifecycle entry point names a role. `connect` resolves a taken
  role through `vacate_role`: swap while idle or `Guiding`, refuse (`CameraRoleBusy`) while
  `Capturing` or `WarmingUp`.
- **Cooler lifecycle**: handle lives in `AppState.slot(role).handle`. `CameraPhase`:
  `Precooling → Idle → Capturing | Guiding → WarmingUp`; ramp limited to 5°C/min (`camera_session::ramp`,
  driven by the monitor for a parked handle and by `guide_task` for one it holds); warm-up ramps to
  20°C, closing the handle once sensor ≥10°C and duty ≤5% (or 5min timeout).
- **Live cooler edits**: `Idle` → `apply_cooler_settings`; `Capturing`/`Guiding` → owned by the
  per-frame path; `WarmingUp` → monitor holds cooler off intentionally. The dew heater has no
  `CaptureConfig` field, so under `Guiding` it is queued as a `CameraOp` for the loop instead.
- **Per-camera profiles** are keyed `"{provider}/{model}"`, plus `#guide` for the guide role so two
  bodies of one model cannot overwrite each other. `apply_camera_profile_on_connect` clamps exposure,
  gain and binning to what the camera advertises on *both* paths — an out-of-range one is what
  `CaptureConfig::validate` rejects, and a rejected config stops the camera capturing at all.
- **Telescope profiles** (`camera_telescope_profiles`, keyed by camera *name*) are what
  `solver_telescope` hands Push-To, and they key its per-rig FOV cache. A connect seeds one from
  the sensor the camera reports (`ensure_camera_telescope_profile`) — sensor fields only, focal
  length carried over just when the flat block already describes that sensor. Without it an
  unprofiled camera falls back to the flat block, so two bodies share one rig key and one
  remembered FOV: 2026-09-07 gave the guide rig the main camera's 0.52° for a 1.45° field and
  Push-To returned no solve for 19 minutes. Never overwrites a profile the equipment UI wrote.
- **`cooler_fast_mode`**: bypasses the ramp; UI shows a persistent warning while on.
- **Dual Sampling (Player One)**: sensor mode auto-picked by `desired_sensor_mode()` (DeepSky/Comet
  → `LowReadoutNoise`, Planetary → `Normal`), overridable via `sensor_mode_override`. Main role
  only — nothing the guide camera produces is integrated, so it stays `Normal`.
- **Monitor thread**: dedicated `std::thread`, not tokio, so USB stalls can't poison the runtime;
  uses one reusable `monitor::FfiWorker` rather than a thread per poll. One per slot, bound to its
  role at spawn — it must never look up "whatever is connected".

### Guide camera — non-obvious and load-bearing

- **One thread, not the four-stage pipeline** (`capture::guide_task`). Nothing it produces is
  stacked or queued. Started by `connect`, not by Start Capture: solving and the guide preview are
  wanted *while* framing.
- **The render gate is the whole point.** Post-processing and encoding run only while
  `guide_stream.has_viewers()`. Solving and raw saving sit **above** both early exits — they are why
  the loop runs. Covered by `guide_task::tests`; if you move the gate, keep those honest (they
  assert the camera really exposed the frames it did not render).
- **Two `FrameStream`s, two counters.** `JpegTierCache` serves a tier only while its counter
  matches, so one shared counter would make each camera invalidate the other's payloads every
  exposure. `/ws/stream?source=guide` selects the stream at upgrade time.
- **Hardware settings are per role.** Flat `CaptureSettings` fields are the main camera's;
  `CaptureSettings::guide_camera` is the guide's. Read them through `profile_for(role)`, never
  flat. `POST /api/settings` carries `camera_role` (absent ⇒ main).
- **One solve source at a time.** `solving::plate_solve_available(state, SolveSource)` decides, on
  `guide_loop_running` — the loop *exposing*, not a camera being connected. The two diverge: a
  cooled guide camera stays registered through minutes of warm-up with its loop already stopped,
  and `connect` returns before a loop that may fail to start. Keying on presence stood the imaging
  camera down for a solve source that was not there. `lifecycle::sync_solver_rig` names the solving
  camera *and* its optics together — `camera_telescope_profiles` is finally read here, and naming
  one without the other is worse than naming neither.
- **The loop is the only path to its device.** It owns the handle for the whole connection, so the
  monitor can never check it out: the loop drains `slot.drain_ops()`, samples `status()` itself
  every 2s, and steps its own `RampState` into `config.target_temp_c`. Without that the guide camera
  reported no temperature and its setpoint bypassed the 5°C/min limit.
- **A raw session carries its frame number.** `slot.raw_session` parks `RawSessionResume { dir,
  next_frame }`: rejoining the directory a dropout left while restarting at 1 wrote straight over
  the frames already in it (`frame_{:06}.fits`).
- **`selected_camera` means "the camera being configured", not the capture target.** Captures
  resolve to `camera_in_role(Main)`; a guide camera id is refused.

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

Connect and `finalize_disconnect` both call `PushToService::set_active_camera`. The
solver remembers a field of view per optical configuration, and that key cannot tell two
cameras sharing a sensor format apart; a stale FOV *fails* a hinted solve rather than
merely slowing it. Only a *named, different* camera discards the remembered value —
boot and disconnect both look it up with no camera, and treating that as a mismatch
deleted the entry before the session could use it. See the Pro AGENTS.md.

### Focus/Finder mode (`state::focus_mode`)

Forces six settings off and keeps `focus_mode_snapshot` as the only record of what they
were. **Entering must be idempotent** — a second `set(.., true)` that re-snapshots captures
the already-forced `false`s and destroys the observer's values for good. Invariant:
`focus_mode == focus_mode_snapshot.is_some()`, so drive it through `focus_mode::set`, never
by assignment, and a persisted flag with no snapshot loads as off. `update_settings` applies
it *after* every other field so the mode wins over a managed field in the same request;
with no toggle in the request `reconcile` absorbs a stale client's write into the snapshot
rather than letting the next toggle silently revert it. `superpixel_debayer` is
deliberately unmanaged — it is the cheap debayer, so forcing it either way costs frame rate.
Hot-pixel rejection is not a setting at all any more; see the raw-CFA stage below.

**The mode and an accumulating stack are mutually exclusive**, enforced both ways:
`update_settings` answers 409 to `focus_mode: true` when `conflicts_with_capture` holds, and
`CaptureService::{start,resume}_capture` leave the mode on the way in. One of the six
(`fpn_removal`) runs pre-demosaic, so `stacking_task` integrates whatever it produces and
never resets the stack on a `sensor_correction` change — banding averaged in cannot be
taken out. Live view accumulates nothing and is exempt;
*leaving* the mode is never refused. Measured win, preview stage only:
`preview_pipeline/focus_mode_x6` 39.4 ms against `full_x6` 76.8 ms.

## Push-To gating (`capture::solving`)

A frame is offered on one of two slots. `try_begin_solve` may block for a whole ASTAP
ladder; `try_begin_watch` runs *during* one, so a slew can be noticed and the doomed
search abandoned — that path used to be closed, and the movement detector saw nothing
for the minutes a full-sky search took. Separate cadence floors (1 s / 1.5 s): the solve
timestamp is stamped once per ladder, so sharing it would let the watch free-run.

`plate_solve_available` declines when no target is set *or* the applicable floor has not
elapsed. It is advisory — the `try_begin_*` compare-and-swap still decides — and exists
so the stacking thread does not clone a frame handle for an offer about to be dropped: a
live second handle makes the render task's `Arc::try_unwrap` fail and copy a full frame.

`PushToBlocker` is the vocabulary for "why nothing is happening", including the ordinary
states of a pushed scope (moving, settling, trailing). All of it flows through
`FrameOutcome::blocker` into `announce_blocker`, which emits one event per transition —
including the shutdown clear, which used to update the de-duplication record without
sending anything and so left the last blocker on screen. The plugin must not broadcast
its own — one that did bypassed the de-duplication. The UI ranks a live blocker above
the last solve verdict, so `StatusBar.vue`'s branches must keep the same order as
`solvingMessage`, or a blocker inherits the previous solve's tick and `success` class.

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

Directory layout: `captures/raw/DD-MM-YYYY_HH-MM-SS-<mode>/frame_NNNNNN.fits` (or `capture.ser` for
Planetary) and `captures/stacked/DD-MM-YYYY_HH-MM-SS-stacking.fits` (named after its raw session).

`<mode>` is `live`/`wanderer`/`stacking`, from `CaptureMode::session_dir_suffix`. A collision inside
one second inserts a counter before the suffix, which is why `from_session_dir_name` matches on the
end of the name.

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
  not a raw diff, so bright star cores survive. **Unconditional** — no setting, and Focus/Finder
  mode cannot remove it: the plate solver reads this frame, and bilinear turns each hot pixel
  into a star-sized blob. Guide subs with it off read 72 "stars" vs 25 and failed ASTAP at any
  FOV (`push_to_guide_hot_pixel_tests` in Pro).
- **`fpn`**: levels each line against a narrow (±8) even-order average of its own neighbours,
  not a whole-frame reference (which silently removed 5.2% of real flux). Skipped for Planetary.
- **Planetary gets hot-pixel rejection only** — FPN bands the disc, superpixel halves
  resolution.
- Timed at `info_span!` (~7ms + ~7.2ms/frame on IMX533/Pi). `hot_pixels` measures its sky
  and noise **every frame** from 4,096 samples/site — a 32-frame cache served a gain/exposure
  change the old threshold (1 correction instead of 165, and the guide sub stopped solving).

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

`MasterStack` accumulates in O(1) memory — 16 bytes per pixel, 434 MB at 3008x3008x3, so
the struct's size is a hard constraint (`offered` had to fit in `count`'s tail padding).

**Never estimate the clip threshold from samples that survived the clip.** That is what
`blend_incremental` did: the scale came from accepted samples only, so an early
underestimate rejected the very samples that would have widened it, and the estimator
defended its own error. Measured on 71 real subs it discarded 15 % of samples where 2.5
sigma predicts 1.2 %, left the stack 34 % noisier than a plain mean, and permanently
froze the 0.95 % of pixels whose first samples happened to be identical — 3 frames kept
of 71, for the rest of the session. About 45 % of the integration time, thrown away.

`m2` is now a running mean of squared deviations over *every* offered sample, rejected
ones winsorised to the threshold. That keeps a cosmic ray from widening the window while
still letting a collapsed scale climb back out (a winsorised sample carries `k^2` times
the current variance, so it recovers geometrically — 12 frames at `CLIPPED_SCALE_WINDOW`,
40 at the ordinary one, which is why clipped samples get the shorter memory). Real-data
result: coverage 83.7 % -> 97.6 %, and the rejector now costs 1 % of SNR instead of 21 %.

Rejection also has to survive its own warm-up. Below `min_frames_for_rejection` the tight
clip cannot run, and while nothing ran there at all a satellite trail landing in those 8
frames was averaged in *and* widened the scale enough that the pixel never rejected
anything again — 33.5 sigma of permanent error with every frame kept. A loose 8-sigma
guard now covers everything past `WARMUP_MIN_OBSERVATIONS`; frames 0-2 are irreducible,
since below three samples there is no spread to test against.

`RejectionMethod` is four variants and the incremental path implements two.
`WinsorizedSigmaClip` differs from `SigmaClip` in one line — blend the clamped value
instead of dropping the sample, so a stack of N frames stays a stack of N. `MinMax` needs
the min and max of a sample set nobody keeps (two more floats a pixel is 650 MB at
3008x3008x3), so `live_equivalent` substitutes sigma clipping and logs it. Passing it
through instead left the session with *no* rejection, because
`add_frame_with_border_and_quality` routes only the two clipping methods to the plugin
and averages everything else.

Three things that look like details and are not:
- **The first offered sample has no mean to deviate from.** Its "deviation" is the
  pixel's absolute level — on a 0.0024 sky with 2e-5 sigma that seeds the scale 120x too
  wide, and the rejector clips *nothing*. `observe_scale` ignores it.
- **The mean is an estimate too**, from `count` samples, so the gap under test has
  variance `sigma^2 * (1 + 1/count)`. Without it the clip is tightest exactly when the
  mean is least trustworthy.
- **Do the test on squared quantities.** One avoidable `sqrt` plus a divide per pixel, in
  a loop over 27 million of them, measured 2.9x on the whole kernel; the tables in
  `incremental_pixel` hoist the divides out per frame. Only a clipped sample roots
  anything. `rejection_benchmark`'s `blend_incremental` case guards this — the batch
  `compute_rejection` cases next to it are not the path a live stack takes.

### Phase 6: Background Extraction (Light Pollution Removal)

Removes uneven illumination gradients common in urban skies.

### Phase 7: Image Statistics (The Foundation)

Computes robust per-channel statistics.

### Phase 8: Auto-Color / Background Neutralization

Neutralizes color casts from light pollution.

### Phase 9: Black Point Calculation

Establishes the dark reference level: `black_point = mode - k * sigma`, so the sky
estimate has to resolve far finer than the sky itself. A 71-frame stack's sky sigma is
~3.4e-5 of full scale (2.2 ADU at 16 bits) while `estimate_background_mode`'s histogram
bin is 2.4e-4 (16 ADU) — the whole distribution fits in a fifth of a bin. Reporting the
bin centre made the mode a step function of stack depth: it held for 50 frames, snapped
one bin at 71, and moved the black point 16 ADU against a target 30 ADU above sky. Half
the Dumbbell went below black in a single frame, and deeper integration rendered *worse*
than shallow. The binned peak still selects the region (that is what rejects nebulosity);
the value returned is refined by re-binning those samples 512 ways inside the winning bin
(0.13 ADU) and interpolating that peak. Two failure modes, not one: the bin *choice* has
to be right as well, and a five-wide box smoothing turns a sky narrower than one bin into
a five-bin plateau whose first strict maximum sits two bins low — far enough that the
refinement window misses the samples and falls back to the bin value, 43 ADU out at 2 sky
levels in 21. Ties therefore break on the raw histogram, where a plateau is unambiguous.
Test it by *sweeping* a sky across a bin in tenths at a deep-stack sigma (2e-5); two
sample points found the quantisation but sampled neither plateau position. Refining by *sorting* the window and taking its
half-sample mode gave the same answer but cost 1.40 ms a frame against 0.39 ms unrefined;
the sub-histogram keeps the resolution for 0.51 ms. `black_point_benchmark` guards it.

The same depth trap applies to any quantity derived from `sigma` — see
`estimate_signal_fraction`, which had to stop binning for the same reason — and to the
solver's floor: floor the sky-above-black gap once and derive the black point from it,
never floor only the number handed to the solver. Doing the latter had the solver
stretching for a sky 1.75x brighter than the black point actually left, growing with
depth. Both are pinned by tests in `black_point_tests.rs` and `autostretch/logic.rs`.

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

- **Display copy skipped when unneeded**: `MasterStack::compute()`'s copy (434MB read/108MB
  written) is gated by `want_display`; the frame still stacks regardless.
- **Queue budget sized from the board**: `min(MemTotal/5, 1GiB)`, floored at 64MiB, sized per
  channel from its own payload — capture→stacking is also **bounded by latency** (2s of
  exposures, since a deeper queue only delays the drop and adds preview lag; memory alone put
  19 frames/2.9s ahead of the stacking thread). All three report `pipeline.queue_depth`/
  `_capacity` under `--features telemetry`, separating "slow" from "stalled once".
- **Preview may run binned**, by the largest integer factor `PreviewResolution` allows —
  all-or-nothing at the 2x boundary, fixed for the session (never from the connected client
  set, since 2x2 binning moved the solved `scale_lut` by +25.7% and would re-grade the
  picture for every viewer whenever somebody opened a tab). Default `Native` (no binning).
- **Per-stack estimates (white balance, background, stats) are reused across frames**,
  refreshed by *proportional* stack-depth growth (MAD ~ 1/√N); live view never reuses.

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
