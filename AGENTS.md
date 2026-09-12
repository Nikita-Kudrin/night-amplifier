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

Every case reports **≥~100ms** (below that, criterion overhead and thermal throttling dominate); every bench binary
stays **≤~30s**. Budget: `sample_size(10)`, ~500ms warm-up, 1–2s `measurement_time`, `SamplingMode::Flat` (Linear's
55 iterations/case alone blows 30s).

- Pure routines: repeat `REPS`× in `b.iter`, `Throughput::Elements(REPS*n)` (`debayer_benchmark`); suffix `_xN`
  whenever `time:` covers more than one call.
- In-place mutation: `iter_batched_ref(.., BatchSize::LargeInput)` over `REPS` clones, never `frame.clone()` in
  `b.iter` (`render_benchmark`); hoist reusable setup out (it runs every iteration); beware ops applied to their own output.
- Match production input size *and shape* — mono-only `star_detection_benchmark` missed `mean_luminance`'s 24.9ms.
- Drop cases re-measuring a bigger sibling's kernel; add ones that can *refute* (`image_stats/full_precision`).
- Bench whole-pipeline sums too: per-stage coverage missed `process_preview_frame` (190 of 300ms).

## Build & Test

**System prerequisite:** `nasm` (required by `turbojpeg-sys` for libjpeg-turbo SIMD).

```bash
cargo build --release
cargo test                                                          # fast unit tests
(cd web && npm ci && npm run build)                                 # only for the 2 `frontend_serving` tests on the *real* bundle (web/dist/ is git-ignored: skip locally, fail under CI=1); the rest use a fixture
cargo test --test integration_pipeline -- --ignored --test-threads=1 # integration (slow, ignored by default)
cargo bench --bench <name> -- --noplot                              # read **Benchmark sizing** first; --noplot is ~4x faster (no gnuplot: 95 s -> 22 s), same numbers
cargo run --release -- [port]
cargo run --release --features telemetry -- --telemetry
cargo run --release -- --static-dir web/dist                        # opt-in disk frontend (or NIGHT_AMPLIFIER_STATIC_DIR); default embedded bundle can't pick up web/'s Vite template

# Performance investigation
cargo run --release -- --span-timings                               # log per-stage durations on span close
cargo build --profile profiling                                     # release codegen with symbols, for `perf`

# Frontend: all npm commands run from web/, with nvm loaded
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm install && npm run dev      # dev server on :8844, proxies to :9955
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run build                    # production build to web/dist/
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run lint:fix
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run test:run
```

**Do not run `npm run format`** — Prettier rewrites the whole tree and pollutes the diff; the developer formats on
their own schedule.

**Always run `cargo test` after code changes**; for frontend changes also `npm run test:run`.

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

Axum: REST `/api/*`; WS `/ws/stream` + `/ws/eyepiece` (dynamic JPEG, `?source=guide` for the guide camera),
`/ws/eyepiece_quality` (lossless LZ4), `/ws/events` (JSON). State: `Arc<RwLock<_>>` in `AppState`; exact endpoints,
DTOs and events in source.

`GET /api/eyepiece/snapshot?circular=` is the only REST route returning image bytes: RGB8 PNG of `latest_raw_frame`
at its *own* size, on `spawn_blocking`. `circular=true` returns the **centre square** (the view is `100cqmin` +
`object-fit: cover`; masking the full rectangle left 55 % padding on IMX464) with an **opaque black** field stop
(viewers composite alpha onto white). Native size is costly — 26 MP with denoisers ≈ 933 MB scratch + two ≤77 MB
buffers, ~0.5 s — so `SNAPSHOT_SLOT` (`Semaphore(1)`) *refuses* rather than queues: 503 + `Retry-After: 2`, retried
by the frontend for 15 s. 404 (nothing rendered yet) is terminal.

The retry loop must `fetch`, and a `blob:` URL carries no `Content-Disposition`, so `fetchEyepieceSnapshot` returns
`{blob, filename}` and `utils/saveBlob.js` sets `<a download>` — without it the browser navigates to the blob and
drops the page's streams.

### Web Frontend (web/)

Vue 3 SPA, mobile-first, dark theme. Composables in `src/composables/`, components in `src/components/`. Vite proxies
`/api` and `/ws` to `localhost:9955` in dev.

## Camera Notes

- **Camera roles**: at most one `CameraRole::Main` and one `Guide`. Every handle, monitor, cancel token and reconnect
  guard lives in `AppState.camera_slots[role]`; every lifecycle entry point names a role. `connect` to a taken role
  goes through `vacate_role`: swap while idle/`Guiding`, `CameraRoleBusy` while `Capturing`/`WarmingUp`.
- **Cooler lifecycle** (`AppState.slot(role).handle`), `CameraPhase`: `Precooling → Idle → Capturing | Guiding → WarmingUp`. Ramp
  ≤5°C/min (`camera_session::ramp`; stepped by the monitor for a parked handle, by `guide_task` for its own). Warm-up
  ramps to 20°C and closes at sensor ≥10°C + duty ≤5% (or 5 min). `cooler_fast_mode` bypasses the ramp (UI warns).
- **Live cooler edits**: `Idle` → `apply_cooler_settings`; `Capturing`/`Guiding` → per-frame path; `WarmingUp` →
  cooler held off. The dew heater has no `CaptureConfig` field, so under `Guiding` it is a queued `CameraOp`.
- **Per-camera profiles**: keyed `"{provider}/{model}"` (+ `#guide`, so two bodies of one model don't collide).
  `apply_camera_profile_on_connect` clamps exposure/gain/binning to the camera's advertised range on *both* paths —
  `CaptureConfig::validate` rejects out-of-range values and a rejected config stops capture entirely.
- **Telescope profiles** (`camera_telescope_profiles`, keyed by camera *name*) feed `solver_telescope` and key Push-To's
  per-rig FOV cache. `ensure_camera_telescope_profile` seeds one on connect from the reported sensor (focal length only
  if the flat block describes that sensor; never overwrites an equipment-UI profile). Without it both bodies share the
  flat block's key: 2026-09-07 the guide rig got the main camera's 0.52° for a 1.45° field — no solve for 19 minutes.
- **Dual Sampling (Player One)**: `desired_sensor_mode()` (DeepSky/Comet → `LowReadoutNoise`, Planetary → `Normal`),
  overridable via `sensor_mode_override`. Main role only; the guide is never integrated, so it stays `Normal`.
- **Monitor thread**: a `std::thread` (tokio would let USB stalls poison the runtime) with one reusable
  `monitor::FfiWorker`; one per slot, bound to its role at spawn — never "whatever is connected".

### Guide camera — non-obvious and load-bearing

- **One thread, not the pipeline** (`capture::guide_task`): nothing stacked or queued. Started by `connect`, not Start
  Capture — solving and preview are wanted *while* framing.
- **Render gate**: post-processing/encoding run only while `guide_stream.has_viewers()`; solving and raw saving sit
  **above** both early exits. `guide_task::tests` assert unrendered frames were really exposed — keep that if it moves.
- **Two `FrameStream`s, two counters**: `JpegTierCache` serves a tier only while its counter matches, so a shared
  counter would invalidate the other camera's payloads every exposure. `/ws/stream?source=guide` picks at upgrade.
- **Per-role hardware settings**: flat `CaptureSettings` fields are main's, `CaptureSettings::guide_camera` the
  guide's — read via `profile_for(role)`. `POST /api/settings` carries `camera_role` (absent ⇒ main).
- **One solve source**: `solving::plate_solve_available(state, SolveSource)` keys on `guide_loop_running` (loop
  *exposing*), not presence — a guide camera stays registered through warm-up, and `connect` returns before a loop
  that may fail to start. `lifecycle::sync_solver_rig` names camera *and* optics together; one alone is worse than none.
- **The loop is the only path to its device**: it owns the handle, drains `slot.drain_ops()`, samples `status()` every
  2s and steps its own `RampState` to `config.target_temp_c` (else no temperature and an unramped setpoint).
- **Raw sessions resume numbering**: `slot.raw_session` parks `RawSessionResume { dir, next_frame }` — restarting at 1
  overwrote `frame_{:06}.fits`.
- **`selected_camera` = camera being configured**, not the capture target: captures use `camera_in_role(Main)`.

### Handle ownership — non-obvious and load-bearing

**Vendor closes take a device *index*, not a handle** — `POACloseCamera(0)` / `ASICloseCamera(0)` /
`SVBCloseCamera(0)` close whatever is at index 0 *now*, so a stuck call's late `Drop` can close a reconnected camera.

- Every shim handle holds a `camera::DeviceLease`; **every vendor close goes through `lease.begin_close()`** (one
  close, only for the lease still owning the slot). A `Drop` calling the SDK directly is a review flag.
- Never close an abandoned handle eagerly — stuck FFI can't be cancelled; the lease makes abandoning safe.
- `connect()` **probes the handle before reporting success** — `open()` returning proves nothing.
- **Every vendor call under the connect lock is bounded** (`camera_session::install`: list, open + probe,
  cooler/dew-heater seeding) via `InFlightCalls::run_bounded` on `pending_opens` under `OPEN_TIMEOUT` — inline seeding
  let a hung camera hold every Connect forever. A late result drops on its own thread *before* leaving the count, so
  nothing reopens past an unclosed handle.

### Camera identity (`camera::identity`)

`{provider}_{index}` ids follow USB enumeration order: 2026-09-07 a guide reconnect installed the *imaging* camera.

- SDKs exposing a serial before open (Player One `SN`, SVBony `CameraSN`, QHY id) get `{provider}_sn-{serial}`,
  resolved against a fresh enumeration. Legacy index ids still parse.
- `CameraProvider::identities()` enumerates **without opening** (ZWO/QHY `list_cameras` open every device — mid-recovery
  that takes the other role's lease).
- Recovery never trusts a position: `recovery_candidates` excludes the other role's device (serial or SDK `device_id`),
  a reopened handle must match, and it keeps its **recorded id** even if the index moved.
- So never compare ids to find a device: discovery matches serial, else current `index` + name (not `CameraInfo::id`);
  `connect` refuses the other role's device **before opening** (`install::refuse_device_of_other_role`) — closes go
  by id, so open-then-close already kills it.
- ZWO discovery skips opening a device `DeviceLease::is_open` holds (that open superseded the live lease); held or
  unopenable devices list from `ASIGetCameraProperty` (list *index*, not camera id), keeping positions aligned with
  `open(index)`.
- Discovery, connect and reconnect share one `DeviceCatalog`; `camera_session::recovery_tests` script a reordering bus.

### Device-loss classification

`CameraError::is_sdk_disconnected()` knows no vendor vocabulary — each shim classifies its **own
numeric/enum code** and tags the message via `camera::device_lost::mark` (matching vendor substrings
after the fact doesn't generalize: only PlayerOne renders errors symbolically, the rest print bare
integers).

`status()` reads go through `device_lost::tolerate_unsupported`, not `.unwrap_or(default)`, so an
unsupported parameter falls back while a lost device still propagates instead of reading as a fake
`Ok`.

### Fault detection and recovery

One detector (`server::camera_health`), threshold and streak (`consecutive_watchdog_timeouts`) serve all three
watchdog/monitor sites, so alternating faults still escalate; the streak ages out (`FAULT_STREAK_TTL`), never resets on
success. Recovery is a ladder — each rung runs only if the previous failed; the user hears nothing before
`reconnect::NOTICE_AFTER` (20 s):

1. **Stream restart.** Shims wait `CaptureConfig::stall_budget` (exposure + 3 s + transfer at 10 MB/s) from *entering*
   `capture()`, then stop the stream for the loop to retry. `capture_watchdog_timeout` derives from it (budget + 3 s;
   independent, every lost frame cost the handle). A fault is `STALL_ESCALATION` (3) stalls in a row (`StallTracker`,
   shared by both loops *and* `capture_probe_frame`). Given-up handles close off-thread (`release_faulted_handle`).
2. **Quiet suspend** (`camera_session::recovery`): `finalize_disconnect(DeviceFault)` keeps entry, selection, status,
   guide stream and solver rig; sets `CameraPhase::Recovering` (+ `CaptureState::Recovering` if resumable); spawns the
   supervisor. `end_capture_state` never overwrites `Recovering`, and on any other end clears resume plan + parked
   stack. A recovering guide slot holds solving.
3. **Supervisor** (`camera_session::reconnect`): drains abandoned `sdk_calls` (≤5 s), retries 2/3/5/10 s within 300 s,
   resumes the capture. Reopens hold the connect lock (`disconnect` waits on it), cap at `REOPEN_TIMEOUT` and re-check
   the slot; a timed-out open blocks further opens (`pending_opens`). Connect joins recovery; Disconnect/Stop ends it.
4. **Give-up** → `DisconnectCause::RecoveryFailed`: full teardown, then the first message.

- **`CameraSlot::recovery` (`None → Suspended → Installing → None`) is the only recovery record**, never the phase.
  A fault while `Installing` belongs to the *new* handle (acted on when install ends); `reconnect::release_flight`
  re-arms one that arrived mid-flight. Phase is per slot, not per model name (twins ended each other's recovery);
  `finalize_disconnect` ignores a camera its role no longer holds.
- **The pause exits only by compare-and-set**: `resume_capture` (`Recovering → Starting`, else `CaptureNotPaused`) or
  `AppState::end_paused_capture` (→ `Idle`; imaging Disconnect runs it first, so a late reopen can't restart a warming
  camera). A resume whose camera fails first gets `CameraRecovering`; pause + plan survive. The plan follows every
  settings update (resume restores, saves, announces them) and carries `next_frame`.
- Resumed stacking seeds `reset_detector_start` from the carryover — starting "off" discarded the stack on frame one.
- `finalize_disconnect` takes a `DisconnectCause`, not a bool: warm-up teardown must never reconnect.
- Connect and `finalize_disconnect` call `PushToService::set_active_camera` — the FOV cache can't tell same-format
  cameras apart and a stale FOV *fails* hinted solves. Only a *named, different* camera discards it (see Pro AGENTS.md).
- Debug builds inject simulator stalls: `NIGHT_AMPLIFIER_SIM_STALL_EVERY`/`_RUN` (`simulated::stall_injection`).

### Focus/Finder mode (`state::focus_mode`)

Forces six settings off; `focus_mode_snapshot` is the only record of their values. **Entering must be idempotent** —
re-snapshotting captures the forced `false`s and destroys the observer's values. Invariant:
`focus_mode == focus_mode_snapshot.is_some()` — drive it via `focus_mode::set`, never assignment; a persisted flag
without a snapshot loads as off. `update_settings` applies it *after* every other field (the mode wins); with no toggle in the request,
`reconcile` absorbs a stale client's write into the snapshot. `superpixel_debayer` is deliberately unmanaged (forcing
the cheap debayer either way costs frame rate). Hot-pixel rejection is no longer a setting (see raw-CFA stage).

**The mode and an accumulating stack are mutually exclusive**, every way in: `update_settings` answers 409 to
`focus_mode: true` when the request's *resulting* mode `conflicts_with_capture`; `focus_mode::leave_if_conflicting`
drops the mode on a stacking `CaptureService::{start,resume}_capture` and when a running live view switches to stacking
(announced as `FocusModeLeft`). `fpn_removal` runs pre-demosaic and `stacking_task` never resets on `sensor_correction`
changes, so integrated banding can't be removed. Exempt, since no affected frame reaches an accumulator: live view
(dropping the mode on its start had the observer re-enable it by hand twice on 2026-09-07), types without
`StackingType::uses_fpn_removal` (planetary — `build_cfa_pipeline` reads the same capability), and states taking no new
frames (`Idle`, `Stopping`). `update_settings` reads the capture state before its settings lock, so the capture loop
re-checks every snapshot (`AppState::settings_for_new_frame`). *Leaving* is never refused. Preview-stage win: `preview_pipeline/focus_mode_x6`
39.4 ms vs `full_x6` 76.8 ms.

## Push-To gating (`capture::solving`)

Two slots: `try_begin_solve` may block for a whole ASTAP ladder; `try_begin_watch` runs *during* one, so a slew is
noticed and the doomed search abandoned (closed, the movement detector was blind for minutes of full-sky search).
Separate cadence floors (1 s / 1.5 s) — the solve timestamp is stamped once per ladder, so sharing it lets the watch
free-run.

`plate_solve_available` (declines with no target or before the floor) is advisory; the `try_begin_*` compare-and-swap
decides. It stops the stacking thread cloning a frame handle for a doomed offer — a live second handle fails the render
task's `Arc::try_unwrap` and copies a full frame.

`PushToBlocker` says why nothing is happening, incl. normal pushed-scope states (moving, settling, trailing), via
`FrameOutcome::blocker` → `announce_blocker`: one event per transition, including the shutdown clear. The plugin must
not broadcast its own (bypasses de-duplication). The UI ranks a live blocker above the last verdict, so
`StatusBar.vue`'s branches must keep `solvingMessage`'s order, or a blocker inherits the last solve's tick and
`success` class.

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

Layout: `captures/raw/DD-MM-YYYY_HH-MM-SS-<mode>/frame_NNNNNN.fits` (Planetary: `capture.ser`) and
`captures/stacked/DD-MM-YYYY_HH-MM-SS-stacking.fits` (named after its raw session). `<mode>` (`live`/`wanderer`/`stacking`)
comes from `CaptureMode::session_dir_suffix`; a same-second collision inserts a counter before it, so
`from_session_dir_name` matches the name's end.

**A resumed capture rejoins its folder and must not overwrite it**: `frame_{:06}.fits` replaces existing files and
`SerWriter::create` truncates. So the resume plan carries `next_frame` (`task::FrameNumbers`), video writes
`capture_2.ser`, …, and the guide loop uses `RawSessionResume::next_frame`.

## Streaming Protocols

### Dynamic JPEG (SA10) — `/ws/stream`, `/ws/eyepiece`

Default format; TurboJPEG (SIMD) encodes in the render task, not the WebSocket handlers.

```
Magic "SA10" (4B, 0x53413130 LE) | Width u32 LE | Height u32 LE | Payload size u32 LE | JPEG bytes
```

#### Demand-driven resolution tiers

Clients send `{width, height}`; the tier follows the viewport's **shorter edge**, clamped 1080…2160 (fitting both
edges pushed portrait phones into 4K).

| Tier       | Bounding box  | Serves class | IMX464 (2712×1538) output |
|------------|---------------|--------------|---------------------------|
| `Hd1080`   | 1920×1080     | ≤ 1080       | 1904×1080                 |
| `Qhd1440`  | 2560×1440     | ≤ 1440       | 2539×1440                 |
| `Uhd2160`  | 3840×2160     | ≤ 2160       | 2712×1538 (no downsample) |
| `Original` | unbounded     | —            | 2712×1538                 |

The render task caches one payload per tier with clients (shared by non-downsampling tiers on sub-4K sensors);
handlers serve it on `frame_ready`, except a new client, which encodes once inline. `begin_frame`/`publish_frame`
keep publication race-free.

### Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`

Lossless (beyond 8-bit) path for the eyepiece quality view. SA09 is the chunked variant (parallel LZ4); the frontend
renders via WebGL with Canvas2D fallback.

```
Magic "SA08" (4B, 0x53413038 LE) | Width u32 LE | Height u32 LE | Compressed size u32 LE | LZ4 RGB8 payload
```

#### Client streaming resolution negotiation

Clients report `{width, height}` and are box-averaged down through the same `JpegTier`. Averaging *removes noise* in
proportion to the reduction (WebGL's fallback caps at ~1.45x): IMX533 payload 2.25x smaller at 8.26→6.76 sky-sigma;
IMX464 barely moves and needs denoising instead.

- Size to the **largest** requested tier; an unreported viewport gets the **4K cap**, not the floor.
- Report **canvas**, not window, size (binoview eyes are ~half-window each).
- Re-report on every reconnect (no server-side memory) — never memoize "same size, skip".

## Adding a Stacking Type

Add variant to `StackingType` (`src/stacking/config.rs`), update `StackingType::all()`, and implement capability
methods: `display_name`, `description`, `uses_star_registration`, `supports_stacking`, `supports_quality_weighting`,
`uses_aggressive_stretch`, `desired_sensor_mode`, `uses_fpn_removal` (also decides whether Focus/Finder mode may run
under its stack). No changes needed in `capture.rs`.

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

`RawFrame::to_cfa_frame` yields a still-mosaiced `CfaFrame`; the **stacking task** runs a `CfaPipeline` on it before
demosaic (`to_frame` = that + empty pipeline + bilinear, pinned by a test). Calibration (dark/flat) will use the same
seam; not wired in yet. Timed at `info_span!`: ~7ms + ~7.2ms/frame on IMX533/Pi.

- Both filters work one colour site at a time — mixing sites reads the mosaic pattern as signal.
- **`hot_pixels`**: gated on the *fraction* of centre amplitude the brightest neighbour carries (not a raw diff), so
  star cores survive. **Unconditional** — no setting, Focus/Finder can't remove it: the solver reads this frame and
  bilinear makes each hot pixel a star-sized blob (guide subs: 72 "stars" vs 25, no ASTAP solve at any FOV;
  `push_to_guide_hot_pixel_tests` in Pro). Sky and noise measured **every frame** from 4,096 samples/site — a
  32-frame cache kept a stale threshold across gain/exposure changes (1 correction instead of 165).
- **`fpn`**: levels each line against a narrow (±8) even-order average of its own neighbours; a whole-frame reference
  silently removed 5.2% of real flux.
- **Planetary gets hot-pixel rejection only** — FPN bands the disc, superpixel halves resolution.

### Frame memory layout (planar) — non-obvious and load-bearing

`Frame` stores samples **plane-major** (`idx = channel*w*h + y*w + x`) so filters read a channel contiguously. **Every
8-bit output format is interleaved** — crossing wrongly still compiles and greys the channels. Only FITS (NAXIS3=3)
stays planar.

Rules: use `planes()`/`channel_data()`/`get_pixel()`, never `frame.data()` with `* channels` math; build fixtures with
`set_pixel`, covered by `layout_tests` per format and traversal; 8-bit conversion always rounds via `sample_to_u8`
(16-bit truncates); dispatch per plane, never derive a channel from a flat rayon chunk index. `get_pixel` in a
whole-frame loop is a review flag (120ms/frame in `white_balance::block_medians`, 27ms on `planes()` + rayon).

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

Bayer (CFA) mosaic → full RGB; auto-detects RGGB/BGGR/GRBG/GBRG.

- **Bilinear**: fast, live preview. **VNG**: higher quality, no edge-transition colour artifacts.
- **Superpixel** (opt-in `superpixel_debayer`): one RGB pixel per 2x2 quad, invents no chroma noise. Only worth it when
  the sensor oversamples the display (IMX533 3008²→1504² still exceeds a 1440² eyepiece; IMX464 → 1356x769 doesn't).

**Invariant** (fixed GRBG bug): at a green pixel, red interpolates horizontally or vertically by **row only, never
column** — `get_rb_orientation` keys on `y & 1`. Keying on `x` misrouted GRBG's odd-row greens over a quarter of
every frame; `test_debayer_reproduces_constant_colour_planes` pins all four patterns.

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

`MasterStack` accumulates in O(1) memory — 16 B/pixel, 434 MB at 3008x3008x3 — so struct size is a hard constraint
(`offered` fits in `count`'s tail padding).

**Never estimate the clip threshold from samples that survived the clip.** `blend_incremental` once did, so an early
underestimate rejected the samples that would have widened it: on 71 real subs it discarded 15 % (2.5 sigma predicts
1.2 %), was 34 % noisier than a plain mean, and froze 0.95 % of pixels at 3 of 71 frames — ~45 % of integration lost.
`m2` is now a running mean of squared deviations over *every* offered sample, rejected ones winsorised to the
threshold: cosmic rays can't widen the window, yet a collapsed scale recovers geometrically (`k^2` × variance per
clipped sample — 12 frames at `CLIPPED_SCALE_WINDOW` vs 40 at the ordinary one). Coverage 83.7 % → 97.6 %; SNR cost
21 % → 1 %.

**Warm-up**: below `min_frames_for_rejection` the tight clip can't run; unguarded, a satellite trail in those 8 frames
was averaged in and widened the scale for good (33.5 sigma permanent error). A loose 8-sigma guard covers everything
past `WARMUP_MIN_OBSERVATIONS`; frames 0-2 are irreducible (no spread below three samples).

**`RejectionMethod` has four variants; the incremental path implements two.** `WinsorizedSigmaClip` = `SigmaClip` but
blends the clamped value (N frames stay N). `MinMax` would need per-pixel min/max (+650 MB at 3008x3008x3), so
`live_equivalent` substitutes sigma clipping and logs it — `add_frame_with_border_and_quality` routes only the two
clipping methods to the plugin and silently averages the rest.

- **The first sample has no mean to deviate from** — its "deviation" is the absolute level (on a 0.0024 sky with 2e-5
  sigma the scale seeds 120x too wide and clips nothing). `observe_scale` ignores it.
- **The mean is an estimate too**: test against `sigma^2 * (1 + 1/count)`, or the clip is tightest when the mean is
  least trustworthy.
- **Test on squared quantities**: a `sqrt` + divide per pixel over 27M pixels cost 2.9x; `incremental_pixel`'s tables
  hoist divides per frame and only clipped samples root. `rejection_benchmark`'s `blend_incremental` case guards it (the
  batch `compute_rejection` cases are not the live path).

### Phase 6: Background Extraction (Light Pollution Removal)

Removes uneven illumination gradients common in urban skies.

### Phase 7: Image Statistics (The Foundation)

Computes robust per-channel statistics.

### Phase 8: Auto-Color / Background Neutralization

Neutralizes color casts from light pollution.

### Phase 9: Black Point Calculation

`black_point = mode - k * sigma`, so the sky estimate must resolve far finer than the sky: a 71-frame stack's sky sigma
is ~3.4e-5 (2.2 ADU at 16 bits), `estimate_background_mode`'s bin 2.4e-4 (16 ADU). Reporting the bin centre made the
mode a step function of depth — it snapped one bin at 71 frames, moving the black point 16 ADU against a target 30 ADU
above sky; half the Dumbbell went black and deeper stacks rendered *worse*.

The binned peak still picks the region (rejecting nebulosity); the value is refined by re-binning its samples 512 ways
inside the winning bin (0.13 ADU) and interpolating the peak — 0.51 ms/frame vs 0.39 ms unrefined (sort + half-sample
mode: 1.40 ms; `black_point_benchmark` guards it). The bin *choice* must be right too: five-wide box smoothing turns a
sub-bin sky into a plateau whose first strict maximum sits two bins low, so refinement misses and falls back (43 ADU off
at 2 of 21 sky levels) — ties break on the raw histogram. Test by *sweeping* a sky across a bin in tenths at deep-stack
sigma (2e-5); two sample points found the quantisation but neither plateau position.

The same depth trap hits anything derived from `sigma` (`estimate_signal_fraction` stopped binning too) and the solver's
floor: floor the sky-above-black gap once and derive the black point from it — flooring only the solver's input had it
stretch for a sky 1.75x brighter, growing with depth. Pinned in `black_point_tests.rs` and `autostretch/logic.rs`.

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

The file layer formats span fields with its own `PlainFields` type: tracing-subscriber caches a span's formatted fields
per formatter *type*, so sharing `DefaultFields` with the coloured console wrote its escapes into the file (15,533 of
31,108 lines on 2026-09-07). The console colours only on a terminal.

**Startup system report** (`system_info`, called from `app::run`): INFO events for build (git describe, target,
`target-cpu`, rustc — exported by `build.rs`), host (OS, kernel, board, boot time: a reset between two logs shows as a
new boot time), CPU (clusters, compiled vs runtime-detected SIMD — release artifacts are per-CPU, so a feature compiled
but absent warns), memory (cgroup limit, frame-queue budget), process (paths, free space under the working dir, env
overrides; OTLP endpoint as presence only) and Linux (device-tree model, boot id, euid, governor, usbfs, SoC temp, Pi
under-voltage). Self-contained (std/`sysinfo`/`fs4`/`chrono`/`tokio`) so it compiles for every release target;
collected on `spawn_blocking` under a 5 s timeout. `build.rs` deliberately emits no `rerun-if-changed`, so a commit
not followed by a file change keeps the previous commit id.

### What the render and stacking threads deliberately skip per frame

One production trace: both workers at 97%, 34.7% of captured frames dropped for want of a stacking thread. A dropped
*capture* frame is signal lost for good, so these favour stacking.

- **Display copy only when wanted**: `MasterStack::compute()`'s copy (434MB read/108MB written) is gated by
  `want_display`; the frame always stacks.
- **Queue budget from the board**: `min(MemTotal/5, 1GiB)`, floor 64MiB, per channel from its own payload;
  capture→stacking is also **latency-bounded** at 2s of exposures (memory alone put 19 frames/2.9s ahead of stacking).
  All three report `pipeline.queue_depth`/`_capacity` under `--features telemetry` ("slow" vs "stalled once").
- **Preview may run binned** by the largest integer factor `PreviewResolution` allows — all-or-nothing at 2x, fixed per
  session, never from connected clients (2x2 moved `scale_lut` +25.7%, re-grading every viewer when a tab opened).
  Default `Native`.
- **Per-stack estimates (white balance, background, stats) are reused**, refreshed on *proportional* depth growth
  (MAD ~ 1/√N); live view never reuses.

### The accumulator layout

`IncrementalPixel` (16B/sample) makes a 3008² colour stack a 434MB accumulator, read+written
whole every frame — ~32GB/s at 26.7ms, already the memory ceiling (no arithmetic win left,
only traffic).

Two fixes are blocked on the same thing: dropping `m2` when rejection is off (halves to
8B/sample), and struct-of-arrays (`compute()` becomes a memcpy, not a gather — 4x win).
Blocker: `RejectionPlugin::blend_incremental`'s cross-crate `&mut [IncrementalPixel]`
signature — either fix breaks Pro and needs both repos moved together.

### Pipeline performance instrumentation

`--span-timings` logs every stage span's duration on-device. Per-frame/payload work belongs at `info_span!`, not
`debug_span!`, or it's invisible. Stage granularity only; cache instruments in a `OnceLock`, never rebuild per frame.

A span with large self time and no children is a **blind spot**; these split the largest residues in a production trace:

| Span | Inside | Separates |
|---|---|---|
| `wb_grid`/`wb_apply` | `background_neutralization` | estimate vs. application |
| `blend_pixels` | `add_frame` | Pro's rejection blend (was unspanned) |
| `resample`/`row_tail` | `frame_to_rgb8` | input-scaled gather vs. output-scaled tail |
| `publish_state` | render/stacking iteration | async-lock overhead off-tokio |

`camera_capture` (one opaque vendor call) has `call_us` + a **signed** `overhead_us`: negative means the frame was
already waiting (continuous `get_video_data` returns in under one exposure, so unsigned read `0`).
`process_preview_frame`'s render tail is fused — no per-sub-stage span.

`--features telemetry`: histograms `frame.{capture,debayer,stack,render,encode_jpeg}_ms`, counters
`frame.published/dropped/render_skipped`, per-channel `pipeline.queue_depth`/`queue_capacity` gauges. The UI shows the
**drop rate** (`AppState::drop_rate()` over `delivered_frames`): 40 drops ruin an evening at 30s subs, and are noise at 100ms.

Build `--profile profiling` for `perf`; rayon threads show as `tokio-rt-worker`.
