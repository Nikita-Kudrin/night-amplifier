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

## Fixture sets (`tests/integration/common.rs`)

Real-data sets live in `DEFAULT_FIXTURES` and download on demand; `tests/fixtures/` is gitignored.
A test wanting one calls `stack_depth_grain_tests::managed_session`, which **panics** when it cannot
be had — never `println!` + return, or the suite reports green with the assertion unrun.

A set cut but not yet uploaded is registered with `PENDING_UPLOAD` in place of the Drive id: the
download is skipped (three retries saving an HTML error page help nobody) and
`missing_fixture_message` tells whoever hits it to paste the real link. Registering the set with the
test that needs it is what stops the two drifting apart.

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
cargo check --lib --no-default-features --features telemetry       # CI guards it: nothing else compiles telemetry (1a243da broke it for 3 days)
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
| `camera/`                     | Traits + ZWO/PlayerOne/QHY/ToupTek/SVBony SDKs + simulator (see Camera Notes below)  |
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

Axum: REST `/api/*`; WS `/ws/stream` + `/ws/eyepiece` (JPEG at Streaming Resolution, `?source=guide` for the guide
camera), `/ws/eyepiece_quality` (lossless LZ4 at Eyepiece Streaming Resolution), `/ws/events` (JSON). State: `Arc<RwLock<_>>` in `AppState`; exact endpoints,
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

`useCatalogSearch` skips a programmatic query by *value* (`setQueryWithoutSearch`), never with a one-shot flag: clearing
a 1-character query armed the flag, so typing "M" then "M4" never searched and M1–M9 were unfindable. It also drops
responses from superseded searches, or a slow reply reopens the dropdown after a target was picked.

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
- **Vendor SDKs are dlopen'd, never linked or shipped** (`dlopen2::Container::load` in each provider's `sdk.rs`).
  rustc passes `-l` even for an unused `#[link]` block, so one breaks every build without that SDK. QHY publishes no
  redistribution grant, and the binary has no RUNPATH to find a copy beside it. Enforced by
  `camera::sdk_loading_tests` and the shared-library check in `scripts/build-dist.sh`.
- **QHY, ToupTek and SVBony bind eagerly** (`camera::sdk_library::load_eagerly`, `RTLD_NOW`): a lazily bound SDK with an
  unresolvable symbol kills the process at its first call. The Linux SVBony SDK uses libusb without declaring it, so
  `preload_libusb` loads it `RTLD_GLOBAL` first. ZWO and Player One still load lazily (not yet hardware-checked).
  A symbol only some SDK versions export goes through `optional_symbol` (QHY's `EnableQHYCCDMessage`), never the
  required API struct.
- **SDK strings are per-platform**: ToupTek's device strings and `Toupcam_Open` id are UTF-16 on Windows (`TChar`, with a
  const layout assertion in `touptek/ffi_types.rs`); other C string buffers are `c_char`, never `i8` (`u8` on Linux
  ARM). C `long` is 32 bits on Windows: widen SDK `long`s with `i64::from` and narrow with
  `ffi_safety::to_sdk_long`, never `as` (it wraps). CI's `rust-vendor-providers*` jobs are the only ones that compile
  vendor code — the Windows one first caught `c_long` returned as `i64` (Player One, ZWO; broken since `ed379af`).

### Guide camera — non-obvious and load-bearing

- **One thread, not the pipeline** (`capture::guide_task`): nothing stacked or queued. Started by `connect`, not Start
  Capture — solving and preview are wanted *while* framing.
- **Render gate**: post-processing/encoding run only while `guide_stream.has_viewers()`; solving and raw saving sit
  **above** both early exits. `guide_task::tests` assert unrendered frames were really exposed — keep that if it moves.
- **Two `FrameStream`s, two counters**: a payload is served only while its counter matches, so a shared
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
- ZWO discovery skips opening a device `DeviceLease::is_open` holds. A QHY device has **one handle per process**:
  `QhyHandle` claims the id atomically (`DeviceLease::try_acquire_unique_device`) *before* `OpenQHYCCD` — discovery
  skips a claimed id, connect waits `HELD_WAIT` (5 s) — and scan/open/init/close share one SDK lock. Held or unopenable
  devices list from pre-open data (`ASIGetCameraProperty`, QHY scan id), keeping positions aligned with `open(index)`
  (list *index*, not camera id). ToupTek lists a model-less device by name and reads its serial only after open.
- `RegistryCatalog` offers every vendor (`CameraRegistry::register_vendors`; registration order is list order) to
  discovery, connect and recovery alike; INDI is not (its own discovery path, never connectable).
  `AppState::new_for_testing` uses `RegistryCatalog::simulator_only()`, so tests never call installed SDKs.
- **Discovery is bounded per provider** (`CameraService::discover_cameras`): providers list concurrently, each under
  `DISCOVERY_TIMEOUT` (20 s) on `AppState::discovery_calls_for`; a refresh waits on, then skips, a provider still
  inside its SDK. A hung device froze `/api/cameras` and parked one more thread per refresh.
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
   independent, every lost frame cost the handle). A fault is `STALL_ESCALATION` (3) stalls in a row (`capture::stall`:
   both loops *and* `capture_probe_frame` go through `handle_stall`, which also logs what surrounded the stall). A camera
   whose in-place restarts failed `RESTART_DISTRUST_AFTER` (2) times in a row within 10 min escalates at its *first*
   stall (`camera_health::RestartHistory`, kept in `AppState` per role and name because the reopen rebuilds the
   tracker; a user `connect` forgets it): 2026-09-14 restarts cured at most 3 of 145 guide stalls, each costing a whole
   budget. Given-up handles close off-thread (`release_faulted_handle`).
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
- Field switches for USB stalls, logged by the system report: `NIGHT_AMPLIFIER_GUIDE_ACQUISITION=snap|video|auto` (guide
  camera only, `CaptureConfig::acquisition`) and `NIGHT_AMPLIFIER_USB_BANDWIDTH=1..100` (Player One
  `POA_USB_BANDWIDTH_LIMIT`, applied at open if inside the camera's advertised range, read back at `info` while set). The Player One shim logs `POAGetDroppedImagesCount` on a
  stall, read before the stop that resets it.

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

Loops offer frames through `offer_plate_solve` to two pipeline tasks (`capture::push_to_tasks`): `push-to-solve` and
`push-to-watch` threads, each fed by a one-slot channel and running the plugin under `rt.block_on`, so its synchronous
detection and FITS write never hold a runtime worker. A frame goes only to an *idle* consumer
(`QueueDepth::try_claim_idle`) and is dropped otherwise — never queued: spawning a task per offer kept 27 of 32 frames
alive, 5.7 s stale, while solve and watch were busy (`tests/push_to_offer_backlog_test.rs`). Detection shares the global
rayon pool at normal priority; niceness and a second pool were tried and removed (4.6x slower under load, and
`pre_exec` forced `fork` for ASTAP: ~50 ms per spawn at 3 GiB RSS).

`plate_solve_available` (declines with no target, before the floor, or with the lane's task busy) is advisory; the
`try_begin_*` compare-and-swap decides. It stops the loops converting or cloning a frame for a doomed offer — a live
second handle fails the render task's `Arc::try_unwrap` and copies a full frame.

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

#### Streaming resolution is a setting, not negotiated

Every client of a family gets the **same payload**, sized by a setting (`state::Resolution`: `Native`, `Uhd2160`
3840×2160, `Qhd1440` 2560×1440, `Hd1080` 1920×1080 — boxes, aspect kept, never upscaled):

| Family (`StreamKind`) | Endpoints                                 | Setting                                           | Default |
|-----------------------|-------------------------------------------|---------------------------------------------------|---------|
| `Jpeg`                | `/ws/stream` (`/`), `/ws/eyepiece`        | `streaming_resolution` (Streaming Resolution)     | 1440p   |
| `Lossless`            | `/ws/eyepiece_quality`                    | `eyepiece.stream_resolution` (no 1080p variant)   | 1440p   |

- **Why not per client**: per-viewport tiers cost one denoised conversion per distinct size on the render thread
  and unbounded concurrent first-frame encodes. Replaced by user decision; don't reintroduce viewport negotiation.
- `capture::stream_encoding` encodes each family once per frame, only while `viewer_count(kind) > 0`; equal output
  sizes share one conversion. `FrameStream` keeps one `(counter, Bytes)` slot per family.
- A changed setting applies from the **next rendered frame**: encoders read the *live* setting
  (`CaptureSettings::stream_resolution`), never the frame's snapshot, which is taken at exposure start — that landed a
  change one exposure late and flipped a joining client new → old → new. A client joining before that frame gets the
  current payload at its old size, like everyone else.
- `ws::image_stream::serve` handles all three sockets: registers a `ViewerGuard`, logs one `info` line (page, peer,
  resolution, output), sends the current frame, then every published one. Client text other than `ping` is ignored.
- **First-frame encodes** (family unwatched when the frame rendered) are serialised per family
  (`on_demand_encode_lock`): simultaneous arrivals share one encode. A client that leaves mid-encode is released
  only when that encode ends.
- Encode failures log + `send_error`; never `frame_rejected` (that feeds the camera's rejection rate). A failure
  repeating every frame is reported once per family (`FailureReports`): the UI re-raises every `error` event.
- Pinned end to end by `server::tests::image_stream_clients` (real sockets, real render task, settings endpoint).

### Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`

Lossless (beyond 8-bit) path for the eyepiece quality view. SA09 is the chunked variant (parallel LZ4); the frontend
renders via WebGL with Canvas2D fallback.

```
Magic "SA08" (4B, 0x53413038 LE) | Width u32 LE | Height u32 LE | Compressed size u32 LE | LZ4 RGB8 payload
```

#### Downsampling to the streaming resolution

The encoder area-averages down to the configured box. Averaging *removes noise* in proportion to the reduction, where
the browser's minification caps at ~1.45x: IMX533 payload 2.25x smaller at 8.26→6.76 sky-sigma. IMX464 is only 1.07x
over the 1440 box: sky sigma 10.2 levels through the whole-pixel box, 8.2 now. So pick the setting that matches the
screen, not a larger one.

**The kernel must give every output pixel the same noise** (`encoding::axis_taps`: footprint integrated over a 1 px
tent per source sample). A whole-pixel box at 3008→1440 averaged 2 samples on most lines and 3 on every ~11th: 18 %
less noise there (2D worst 1.5x; 2.0x on IMX464), a lattice at 18 arcmin through a 100 mm eyepiece lens. Fractional
box edges alone still vary 1.26x; the tent keeps ratios from 1.4x under 1.05x. Cost 2.9 → 4.3 ms/call
(`encoding_benchmark` `imx533_to_eyepiece_1440`).

**The tent softens near unity**, so a `[-a, 1+2a, -a]` sharpen on the output grid is folded into the taps: a 2.5 px
star kept 83 % of the box's peak at 1.07x without it. `a` = 0.15 to 1.1x, none from 1.9x (the tent alone beats the
box's worst-phase peak at 2.09x). Shift-invariant, so no lattice of its own; 1.07x noise non-uniformity 1.24 (box
2.0). Pinned by `a_near_unity_downsample_keeps_star_peaks` and the reference's 1.07x/1.42x cases. Its negative lobes
clip no ring beside bright stars (deepest +1.1 sigma vs a 2.8-sigma black point). Taps are built once per axis size
(`AxisTaps::cached`): rebuilding them per encode was 8.6 % of `imx533_to_eyepiece_1440`.

- The frontend uploads RGB rows unpadded, so WebGL needs `UNPACK_ALIGNMENT` 1: at the default 4 any width not
  divisible by 4 (IMX464 at 1440p: 2539 px) failed `texImage2D` and froze the previous frame.

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
luma-guided) and **wavelet** (à trous B3, up to 6 levels, MAD-thresholded luma). Run at stream
resolution, after resample/before tone curve — full-res then discarding 3/4 would be 4.5x the
memory traffic. ~14ms combined at 1440² (20-core x86).

- Off fuses per-row; either filter on stages the whole image as f32 first (cross-row access).
- Thresholds `DEFAULT_K = [0,3,2,1,0,0]` get weaker at finer scale on purpose — coarse-heavy
  denoising erases real nebula structure. The last two are the coarse pair, off at and below
  the dial's middle; `k[0]` and they are what the dial moves.
- The guided filter's regularisation is **`noise_k` sigmas of its own guide, measured per frame**,
  not a constant. It was a fixed `1e-4` in linear light against a sky whose guide variance is
  ~1e-9: every window read as flat, the filter degenerated into a ~40 px box blur of chroma, and
  star colour bled into halos that raised 32-64 px chroma noise **above** the unfiltered sky
  (2-4.6x on real IMX533 stacks) — the "blotches" reported at the eyepiece. Swept over four
  sessions; fine chroma converges by `k=3`. Pinned by `sky_blotch_tests.rs` and
  `guided.rs::a_faint_star_keeps_its_colour_to_itself`.
- `k[0]` (grain) is driven by the Background Grain dial through
  `DenoiseSettings::star_protection()`, fully spent at the default; ceiling reaches ~7x
  noise reduction — but of *fine* noise, which is a small share of what an observer sees.
  The dial has no off switch for the filter: a zero `luma_strength` is that, and it is the
  one the manual points at when nebulae turn to plastic. Focus/Finder mode holds the filter
  off by zeroing the strength, deliberately **not** by moving the dial, which would move
  the tone curve with it.
- **`strength` (Structure strength) scales only levels 2-4**, the mid scales it is named
  for. It used to scale all of them, and that was a trap: an observer running it at 0.2 had
  every position of the Background Grain dial quietly divided by five, and measured a 0.0 %
  change in visible sky noise across the dial's whole lower half. Two controls, one
  silently scaling the other, is not two controls.
- **Levels 5-6 are the coarse pair and need their own shrinkage.** Their support is wider
  than a star, so the smoothed plane carries a star's flux tens of pixels out and the
  detail plane goes negative just outside it. Two things make them safe, and the guard is
  `a_bright_star_keeps_no_ring`, which now runs at the top of the dial as well as the
  default:
  - **Non-negative garrote, not a soft threshold.** A soft threshold subtracts `t` from
    every surviving coefficient including the star's large ones, and that constant shift
    is what moves real light into a ring (measured: a -2.3 level trough at r=9-15 px with
    every star in a +1.2 level pool). The garrote shrinks by `t^2/d`, so `d >> t` is left
    almost untouched. It is continuous at `t`, so it does not bring back the blotches hard
    thresholding was rejected for.
  - **A mask read off the smoothed plane itself** (`MASK_SIGMAS`), not dilated out from a
    map of star positions. The disc a coarse level would light *is* the region the
    smoothing has carried flux into, so the smoothed plane already has it at the right
    size for every star with no radius to guess. A dilated point mask was measured and
    fails both ways: narrow it changes the ringing not at all, wide it protects a dense
    field entirely and the coarse levels recover nothing. The rule is nearly "smooth what
    is at or below the sky, never anything brighter".
  `COARSE_K` is half what the best grain figure wanted: doubling it buys 2 % more grain and
  takes the dense-field disc from +0.48 to +0.68 output levels, which is the whole of the
  guard's margin.
- **Three Star Fields ideas that measured well and looked wrong.** All three were caught
  by rendering a crop and looking at it, after the score table had already approved them:
  - *Crushing the coarse thresholds* (3 sigmas at every level, 5-6 on) scored best on every
    number and put visible dark contour worms across a wide field's background — zeroing a
    smooth gradient's detail coefficients leaves the reconstruction piecewise flat. The
    win it appeared to deliver came from the finest threshold, which has no such problem.
  - *Flattening the coarse gains* (0.8/0.5/0.4) to spread a globular's glow digs a -8
    output level moat around every star, for the same reason a coarse threshold does.
  - *Sharpening past ~1.15* rings, and the ceiling is set by the **shallowest** stack: 1.25
    keeps a +1.4 level margin on a 1852-frame globular and rings outright on the 35-frame
    CI fixture, where the profile turns back up 0.9 -> 1.9 at r=11 px.
  The score table could not see any of them; `star_field_score`'s `moat` column exists
  because of the second, and `a_bright_star_keeps_no_ring` now runs Star Fields for the
  third. **Look at the picture.**
- **Every per-level noise estimate goes through `statistics::select_median`, never
  `fast_median`** (`MAX_SIGMA_SAMPLES`). Since the sigma became per-level it is drawn twice
  a level — a dozen times a frame with the coarse pair on — and `fast_median`
  `par_sort_unstable`s anything from 4096 up, so that was a dozen parallel full sorts
  inside one pass. Selection plus a 16 384-sample cap took the 1440² pass from 14.2ms to
  10.6 (coarse off) and 21.0 to 13.8 (coarse on), output unchanged to 0.1 output levels.
  `black_point::clipped_centre` documents the same trap; it is the one to check first
  whenever a per-frame estimator appears.
- Skipped for `StackingType::Planetary` — lucky imaging needs the detail this removes, and
  note the Background Grain dial still moves the *tone curve* there, where nothing in the
  picture moves with it.

### Denoising cost

Denoising is **several times the cost of the encode it sits in** (IMX533 @1440p, 20-core
x86: 4.7ms without it; in `denoise_benchmark` the two filters measure 14.3ms a frame
together at the shipped dial position, and the wavelet alone goes 10.6 -> 13.8ms when the
dial's top half brings in levels 5-6 — **one group at a time**, which that file's note
explains). Two structures stop that from multiplying:

- **`ConversionCache`** shares one RGB8 conversion per distinct output size, keyed on
  `output_dimensions`, so both families at the same streaming resolution denoise once.
- **`DenoiseScratch`** is owned by the render thread, not allocated per pass — a 1440² pass
  would otherwise page-fault ~75MB (13 of the 20ms the filters add). Passed down explicitly
  rather than thread-local, since first-frame encodes run on pooled tokio blocking
  threads where thread-local would strand 75MB/thread.

Both spans report under `--span-timings`.

### The fused scale LUT's cache key is quantised *relatively*

`LutCacheKey` keeps 13 of f32's 23 mantissa bits, a step of ~0.012 % of the value at any magnitude.
It used to round `v * 10_000` — an absolute `1e-4` — and an MTF midtone on a deep-sky stack is around
`1e-3`, so the key carried barely one significant figure there. The Nebulae and Deep Sky profiles
solved to 0.001053 and 0.001085 on the 106-sub IMX533 set: 3 % apart, same key, so whichever
rendered *second* silently reused the first profile's tone curve — target core 141 → 144 output
levels and sky 18 → 19, decided by nothing but render order within the process. Live view changes
stretch profile, eyepiece intensity and target background mid-session, and each of those is this
collision. It also invalidated the first round of the 2026-09-18 brightness measurements, which is
how it was found. The black point is still deliberately *out* of the key (subtracted per pixel, no
effect on the table); the relative step is what absorbs the jitter the cache exists for.
`the_lut_cache_separates_curves_it_can_tell_apart` sweeps four decades.

### The f32 -> 8-bit boundary (`render::output::quantize`)

Every displayed byte crosses this boundary once, via `sample_to_u8` — kept as one helper
because parallel 8-bit conversions have drifted by an LSB here before.

`DisplayOutput` (both off by default):
- **`pedestal`**: maps `[0,1]`→`[pedestal,1]` — autostretch clamps ~0.8% of samples to exactly
  0, which OLEDs show as speckle.
- **`dither`**: sub-LSB ordered dither before rounding (replaced a post-round version with
  visible crosshatch). Indexed in **output**, not input, coordinates, or resampling would
  average it away. Matrix is **8x8**: 4x4's ~7 arcmin period is still eye-resolvable. The tile
  repeats every 8 px but its *energy* does not live there — measured, the dispersed-dot matrix
  holds 93.8 % of its power in the top eighth of the spectrum and 0.3 % below half Nyquist,
  where void-and-cluster blue noise of the same tile ran 61 % / 2.4 %. Blue noise was tried on
  the theory that the 8 px repeat sat in the eye's best band and was **rejected** on that
  measurement; `the_dither_keeps_its_energy_near_nyquist` is the guard any replacement must beat.

`black_point_sigma` alone is scale-invariant: the MTF solve pins `mtf(k*sigma) = target_background`,
so displayed grain is `T(1-T)/k` whatever sigma is and a 100-frame stack looks as grainy as one
(4.2 output levels at 1 sub, 4.4 at 8). `autostretch::depth_grain_gain` scales `k` by
`N^s`, where `s` is `AutoStretchConfig::grain_split`: grain falls as `N^-s`, faint-signal contrast
rises as `N^(1/2 - s)`. The stack depth reaches the solve through `AnalysisContext::stack_depth`,
so **any** renderer of a stack has to pass it — `render_stacked_png` takes it from the context that
holds the frame, never from `stacked_count`, and the live-vs-export parity test only catches a
caller that forgets it entirely. The eyepiece slider still interpolates `black_point_sigma`
*upward*, not down.

- **The grain split is a user-facing dial, defaulting to 1/8 rather than the 1/4 an even split
  would give.** The tone curve buys a calmer sky at exactly 1:1 in target contrast, and the wavelet
  buys the same sky for almost nothing — measured on three real sessions, target contrast is *flat*
  across `star_protection`. So the curve should spend as little as will do. At 1/4 and 114 subs the
  black point sat 2.83 sigmas wider and cost that same 2.83x: M27's core rendered 47 output levels
  against 89. At 1/8 the three sessions give the target back 1.5-1.7x (M27 47 → 78, globular
  98 → 150, M31 159 → 203) with sky grain within a few percent either way. Guarded by
  `stack_depth_grain_tests`, which reads the exponent from `depth_grain_gain` rather than restating
  it.
- **One dial, `DenoiseSettings::background_grain`, spends three levers, and which *scales*
  each reaches is what orders them.** It replaced a toggle and two sliders, one of which
  (the curve's share) had no UI at all. An observer reads grain at **8-128 px**, and on a
  1440p stream of a deep IMX533 stack the 16-32 and 32-64 px bands are the two largest —
  so that is the band a lever has to reach to count.
  - *Below the middle*, in two segments. The **bottom quarter** moves `star_protection()`
    (wavelet level 1, 1-2 px) alone and holds the curve at `MIN_GRAIN_SPLIT`, so every
    position in it costs the same — nothing — in target brightness. From 25 % to the
    middle the curve rises to `DEFAULT_GRAIN_SPLIT` as well. Going *down* from the default
    the expensive lever is given back first, which is what makes a low dial mean
    "brightness, please". A straight ramp over the whole lower half was tried and reported
    from the field: at dial 15 % it left the curve 30 % of the way up and took 5-9 % of the
    target with it (M27 core 148 -> 141 output levels, outer 99 -> 90). Level 1 is nearly
    free but it is **fine speckle**: the whole lower half moves 8-128 px noise under 2 %.
  - *Above the middle*: `coarse_denoise()` drives wavelet levels 5-6, the only mechanism
    that reaches 16-64 px. Across six real sessions it takes 8-128 px noise down 9-27 %
    while the target dims 0-1.2 %.
  - The curve's split **stops at `DEFAULT_GRAIN_SPLIT`** and no longer runs to
    `MAX_GRAIN_SPLIT`. Reaching 1/4 cost 38-43 % of target brightness for 37-39 % of the
    grain — the 1:1 exchange the curve always offers, and not a trade any position of a
    user-facing control should make. The coarse levels took over that range at roughly
    15:1.
  `0.5` is the middle, and it does **not** reproduce any earlier build — an earlier
  version of this section said it did, and the test under it asserted the claim against
  itself. The build before the dial defaulted `star_protection` to `1.0` (finest scale
  *untouched*) and split the curve at `1/4`; the middle spends that scale in full and
  splits at `1/8`. On the 106-sub IMX533 set those are 141 against 161 output levels of
  target core. The middle is the tuned trade, not a compatibility point, and
  `the_dials_middle_is_the_tuned_trade` pins the three numbers it resolves to.
  There is **no settings migration**: the `luma` switch and the `star_protection` slider
  are gone from the file format, not read and translated. Reading them was worse than
  dropping them — Focus/Finder mode forces `luma: false`, so a file saved with the mode on
  migrated the dial to `0.0`, and nothing ever put it back because the dial is
  deliberately unmanaged.
- **`MIN_GRAIN_SPLIT` is 1/12, not zero**: at zero the wavelet cannot take over (its
  thresholds are noise-relative, so it removes a fraction of the noise and never pins
  absolute grain), displayed sky grain then *rises* with depth (1.41 -> 2.30 output levels
  over 106 subs) and target-to-grain peaks at 64 subs and falls back — `MAX_GAIN_DEPTH`'s
  give-back failure from the other end.
- **`MAX_GAIN_DEPTH` is 64.** At 1/4 this was load-bearing: the split is only affordable while the
  stack's own noise falls as `sqrt(N)`, and on the 106-sub IMX533 set sigma falls as `N^0.41` to 32
  subs and `N^0.19` from there, so past that the target paid (at a cap of 256 the rendered target
  peaked at 32 subs and gave back 77 → 69 levels by 106). At 1/8 the gain stays under even that
  tail, so the cap is no longer what protects the target; it is kept because a 3-5 hour session at
  5 s reaches thousands of subs and nothing is gained by widening the black point across them.
  `stack_depth_grain_tests` asserts in levels, not percent — a 96 px block median quantises to whole
  levels, and a percentage bound loose enough for one is loose enough for the defect (4.6 % slipped
  through).
- **What does not work**: `sky_shadow` (`black_floor` negative) darkens the sky by a gain
  read from a local mean, so it darkens *less* around every star — each one sits in a lit
  disc, +2.3 output levels at r=21 px. Coarse wavelet levels (5-6) were rejected on the
  same signature and have since been **made to work**, but only with the garrote and the
  smoothed-plane mask the denoise section describes; a plain soft threshold at those
  scales still digs a -2.3 level trough at r=9-15 px, and an interscale mask dilated out
  from star positions still fails both ways. See the star radial profile and surround
  method in `render_brightness_tests` before reaching for any of them again.
- **Star Fields keeps `ToneMappingAlgorithm::Asinh`, and that is a product decision.**
  The mode exists to show a field of stars; nebulosity and galaxy structure are explicitly
  not its job. What asinh costs is measured and should not need re-deriving: it pins the
  rendered star peak at ~160 output levels however it is tuned (MTF reaches 220). Raising
  `target_background` lifts the star count only by lifting the sky with it (0.16 gives a
  sky of 40 output levels); bounding `max_stretch` to keep highlights linear makes both
  worse. A per-profile `ContrastConfig` cannot help — strength is already at its 1.0
  ceiling everywhere.
- **Star Fields denoises differently, and that is where its wins come from** — see
  `STAR_FIELD_FINE_BOOST` and `STAR_FIELD_GAIN`. On a 181-frame 35 mm IMX464 field it
  takes detected stars from 7809 to 15384 per megapixel above sky+20 and 3127 to 5909
  above sky+60, with fine-scale noise down 45 %; on long-focal sets it is roughly neutral
  (a 1852-frame globular loses 6 % of its faintest for 35 % less speckle). The mode now
  beats Deep Sky on a wide field by 2.8x, which is the first time it has been the best
  choice for anything.
- The factor the solve really uses is `AutoStretchResult::adaptive_sigma`, not the setting: the
  signal-fraction gates scale it down and the depth gain up. A caller placing a black point of its
  own (`per_channel_black_point`) must use that, or it subtracts a different gap from the one the
  curve was solved for. `MAX_EFFECTIVE_SIGMA` bounds the product, since two clamps multiplied are
  not a stated ceiling.

### The darkening half of the black floor (`render::output::{sky_shadow, shadow_floor}`)

`EyepieceSettings::black_floor` is **signed**: positive is `DisplayOutput::pedestal` (panel-relative), negative
darkens the sky (sky-relative). At `-5%`: sky 64 %/65 % darker, target excess *up* (IMX533/IMX464 fixtures).

- **Default form is spatial** (`SkyShadow`): a gain from a 3x3 mean of stretched luminance (or the pixel's own
  excess over one sky), smoothstep 1.05→2.0 sky. Every pointwise roll-off keeping a faint target's excess keeps
  equal noise excursions: the softplus knee it replaced pinned 20-35 % of sky pixels at the pedestal and raised
  relative grain 1.7-2x (blocky clumps at a pixel-resolving eyepiece). A 5x5 guide lit a square around each star.
- The sky is **measured in-kernel**: the solver's anchor missed IMX464 by 1.3x and put the whole sky inside the
  shoulder (grain +40 %). It is the *darkest* histogram peak holding >= 20 % of guide samples, not the median: a
  1.6-sky nebula over 60 % of the frame was the median and was darkened like the sky (contrast halved). The 20 %
  keeps a registration border's spike out; a peak under half the anchor is skipped (a roof over 35 % of the frame
  was measured as the sky and the real sky went undarkened). A tie of >= 25 % at the low end (clamped zeros) sizes
  the bins from what lies above it. Selection, not a sort (a 92k-sample sort was 2 ms of a 4.6 ms encode).
- **"Darker sky" is the pointwise clip** (`ShadowFloor`), anchored to `target_background` after contrast; it rides
  the scale LUT or, with saturation boost, the row tail's table.
- Applied in exactly **two** places that must agree (encoder after the row tail; `auto_stretch_frame` last).
  Order is always `stretch → saturation → contrast → floor`.
- **It is streamed** (`encoding::sky_shadow_rows`): 32-row chunks plus a context row each side, the sky from 32
  fixed rows x 256 columns rendered first. Staging the image cost 10.3 ms against 4.3 plain; streamed, 6.0. Denoise
  on feeds its staged image in as a row source (whole-image guide planes were ~208 MB at 26 MP). Both are pinned to
  the test-only `apply_sky_shadow_interleaved` (`sky_shadow_streaming_matches_staged`, `..._after_denoise_...`).
- Three gates: sign, auto-stretch on, and not `StackingType::Planetary`. Slider end stop -5 % = 90 % darker (`sky_shadow::MAX_DARKENING`). The reach is calibrated to where that saturates, so it moved from -6 % when the S-curve strength went to 1.0 and lowered `NOMINAL_SKY_LEVEL`.

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
  batch `compute_rejection` cases are not the live path). The plain mean (rejection None) has
  `stacking_benchmark`.
- **Non-finite samples are skipped like borders** — one NaN left a pixel NaN for the session. The plain mean checks
  `is_finite` (unmeasurable in `stacking_benchmark`); the clip folds it into `!(d^2 <= limit)`, since NaN/±Inf fail
  `<=` — a separate up-front `is_finite` cost 10 % of `blend_incremental`.
- **The settings toggle lands mid-stack** (settings are re-applied every frame) and no longer opens a gap. The plain
  mean maintains `m2` too — the render reads it as a per-pixel noise map — so a first switch to clipping inherits a warm
  scale and is gated from frame 0; frames 0-2 are irreducible only at the *start of a session*. Switching back on keeps
  the old scale: no gap, but a sky that moved while it was off loses frames, as a real brightness step does under
  clipping: 10/15/19/24 frames at 30/100/300/1000 sigma. Pro's `master_stack_tests::robustness` pins both.
- **The plain mean's scale update is not the rejector's.** `observe_scale_guarded` *drops* a sample past 8 sigmas
  instead of winsorising it: `CLIPPED_SCALE_WINDOW`'s geometric widening is a collapse-escape for a path that rejects,
  and with nothing rejecting it is positive feedback — a sustained 1000-sigma level step left the scale 238,000x too
  wide. A scale that collapses there is deliberately not climbed out of: the map reports that pixel unmeasured and
  takes a block median, and a later rejection pass brings its own escape.

### Phase 6: Background Extraction (Light Pollution Removal)

Removes uneven illumination gradients common in urban skies.

**Crowded targets stay out of both models** (`background::target_disc`, used by bilinear and Pro's RBF): brightness
pruning measures on the nodes, so a frame-filling halo was its own reference (~50 % of a globular's glow taken at
256 px). Crowding seeds a disc (`NodeSample::scatter`, plane-detrended so gradients never seed), sized by the ring
excess over an outer surface, capped at 0.45 frame / 24 outside nodes, refilled from that surface before
interpolation: 5-10 % taken. Gradients, satellite trails and a Milky Way band find no disc. The pruning channel's
node values come from the disc's samples (one clip: grid -11 %, RBF -8 %).

The surface is a **plane unless a quadratic halves the outer nodes' residual RMS** (`MAX_CURVED_RESIDUAL`).
Vignetting (flats aren't wired in) is a dome a plane reads as excess: the disc ran to its cap and 2.78 of a
7.66 sigma rise was cut out of the model. A quadratic always, though, follows the halo's own wing on the field crop
(5/10 % -> 18/36 % taken). Ratios: vignetting 0.09, horizon glow 0.29, that halo 0.83.

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

Both histogram passes pick a *bin*, so the sky level is taken from the **samples** the refined peak
points at (`clipped_centre`, a median within 2.5 robust sigmas). Without it the peak stops moving
with the data once the sky is narrow: bit-identical at 32, 64 and 106 subs of one session, 1.2 ADU
above the sky, after jumping 2.1 ADU between 16 and 32 while the sky moved 0.2 — four output levels
of background step in one stack update, which at the eyepiece is the whole field pumping.

That window's spread comes from the samples **below** the peak only, it is their *lower quartile*
rather than their median, and every order statistic here is `statistics::select_nth`/`select_median`
rather than `fast_median`. All three are load-bearing:

- A spread over *every* sample is robust only while the contaminant is a minority, and a
  frame-filling halo is not — the window then sizes itself around the target, swallows it, and the
  median lands inside it (+7 ADU at 69 % cover, +349 on a 75 % ramp, against a peak 1.4 ADU off). A
  target is brighter than its sky, so the sky's lower half is the half it cannot reach. Guarded
  synthetically by `a_frame_filling_target_does_not_drag_the_sky_estimate` and on real sky by
  `sky_estimate_tests` (real stack, synthetic target, so the answer is known).
- The one-sided spread is only safe *upwards*. A population **darker** than the sky — a corner the
  background model over-subtracted and clamped to zero, a vignette, a partly illuminated frame —
  lands at the far end of that same list and drags the window with it. With the spread taken as a
  median the estimate went -1.43 ADU off at 40 % of the frame clipped, -4.95 at 50 % and -150.73 at
  60 %: the whole sky, the window having grown wide enough to put its own median among the zeros.
  `CENTRE_SPREAD_QUANTILE` (the lower quartile, `0.3186 * sigma` for a half-normal) survives to
  60 % and costs nothing where there is no contaminant — swept from 0.06 to 8.2 histogram bins of
  sky sigma the answer stays inside 0.01 sigma either way. A fixed cap on the window was the other
  candidate and is what that sweep rejects: capping at `refine_peak`'s own four bins reads 12.75 ADU
  low at the wide end. `a_population_darker_than_the_sky_does_not_drag_the_estimate` is the guard.
- `fast_median` `par_sort_unstable`s anything ≥ 4096. Three of those per frame over ~50k samples
  took `estimate_background_mode` 0.50 → 1.62 ms — past the 1.40 ms sort `refine_peak` exists to
  avoid. Selection plus a `CENTRE_MAX_SAMPLES` (8192) stride is back at 0.51 ms.

`MIN_EFFECTIVE_MEDIAN` is `1e-5`, a numerical guard only: at `1e-4` it, not the solve, set the black
point past ~16 subs on an IMX533, which is where the pre-`depth_grain_gain` "deep stacks look
smoother" behaviour actually came from. The **unlinked per-channel path has its own pair**
(`unlinked_effective_median`), and they are not interchangeable: a channel above the black point has
a real gap and is floored by the same numerical guard, while a channel at or below it has no gap at
all and takes `UNLINKED_BELOW_BLACK` (`1e-4`, which sets how hard such a channel is stretched and
moves the colour of every cast sky). One literal used to serve both, so a real gap under 6.5 ADU —
every channel of a deep stack — was silently replaced by the below-black answer.

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

**`strength` is 1.0 and `midpoint` 0.2, and the midpoint below the sky is what makes full strength
affordable.** The sky lands on the curve's compressive half and the target on its expansive one, so
the curve brightens the target and darkens the sky in one pass rather than trading one for the other.
Measured against 0.8 on four sessions and all three stretch profiles: target +6-9 % everywhere, sky
~2 output levels darker, octave-band sky noise within ±6 % (and mostly *down* on Deep Sky and Star
Fields), star radial profile unchanged in shape. Contrast is the only free brightness lever here —
the tone curve's own grain split costs target contrast 1:1. Lowering the midpoint below 0.2 was
measured and **rejected**: it moves the sky onto the expansive half and washes the background out
(sky 23 → 32 output levels, p1 11 → 16 at 0.1).

`NOMINAL_SKY_LEVEL` in `stage_config.rs` is *derived* from this curve —
`sky_level_after_contrast(0.08, default)`, 0.045 — and the darker-sky slider's whole calibration
hangs off it, so it moved with the strength. `the_nominal_sky_level_matches_the_shipped_curve` makes
the next change to the curve fail there instead of silently mis-scaling the slider.

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
- **Preview may run binned** by the largest integer factor `preview_resolution` allows — all-or-nothing at 2x, fixed per
  session, never from connected clients (2x2 moved `scale_lut` +25.7%, re-grading every viewer when a tab opened).
  Default `Native`.
- **Per-stack estimates (white balance, background, stats) are reused**, refreshed on *proportional* depth growth
  (MAD ~ 1/√N); live view never reuses.

### The accumulator layout

`IncrementalPixel` (16B/sample) makes a 3008² colour stack a 434MB accumulator, read+written
whole every frame — ~32GB/s at 26.7ms, already the memory ceiling (no arithmetic win left,
only traffic).

**`m2` is no longer droppable when rejection is off**: it is the render's per-pixel noise map
(`MasterStack::noise_field`), maintained on both paths, so that idea is off the table on its
merits rather than merely blocked. Struct-of-arrays (`compute()` becomes a memcpy, not a gather
— 4x win) is still wanted and still blocked by `RejectionPlugin::blend_incremental`'s cross-crate
`&mut [IncrementalPixel]` signature, which needs both repos moved together.

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
