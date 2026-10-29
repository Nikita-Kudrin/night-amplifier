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
- Follow standard Rust conventions for backend; the frontend is strict TypeScript (`exactOptionalPropertyTypes`,
  `strictTemplates`, type-aware promise lint): no `.js` in `web/src`, relative imports without an extension, no
  `any`, and a fire-and-forget promise marked `void` (it must handle its own errors).
- Write tests for new or changed functionality.
- Don't write obvious comments — code should be self-describing; comment only non-obvious behavior.
- Keep comments, doc comments, and prose summaries (module docs, AGENTS.md sections) to 5-10 lines per point: state
  the non-obvious "why" and any load-bearing numbers, cut restated context and hedging. If an explanation is still
  ballooning past that, split it or link out rather than let it sprawl. Exempt: runnable doctest/example code,
  wire-format/byte-layout tables, and other structured reference data (tables, one-fact-per-line lists) — tighten
  wording there without breaking the structure that makes them scannable. `tests/comment_budget.rs` enforces it:
  10 prose lines a comment block, 15 for `//!` module docs (Pro runs the same file).
- Clippy is the linter (`[workspace.lints]` in Cargo.toml, limits in `clippy.toml`): defaults plus functions ≤150
  lines, nesting ≤6 blocks, and a `// SAFETY:` comment on every `unsafe` block and impl. CI denies warnings; a
  justified exception is a local `#[allow]` beside its reason, never a crate-wide one. CI lints on x86_64, aarch64
  and Windows (Pro adds macOS): a C `long`/`char` cast clean on one is a lint on another, and Linux x86 saw none.
  `rust-toolchain.toml` pins the toolchain in all three repos (`rust-version` too, but in License), as each stable
  release adds lints: bump it on purpose, fixing what it flags.
- Avoid expensive operations; never run extremely heavy ones on the fly.
- Optimize imports and remove unused code you created. No crate-wide lint `allow`s (in both repos): `dead_code` and
  `unused_imports` (and clippy's argument count) are allowed only on vendored SDK binding modules, and an import
  only a `#[cfg(test)] use super::*` child needs belongs in that child, or the non-test build warns. CI's vendor job
  builds with `-D warnings`.
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
- Plugin system: performance-critical / Pro-only logic lives behind traits, collected in one `plugins::Plugins`
  value (rejection, background, planetary, saturation, denoise, AI denoise, comet, Push-To's solver, catalog and
  installer, and the field-objects port background protection solves stacks through) so Community works standalone. The binary `install`s its set via `app::run`; `AppState::plugins` carries
  it to stacking, rendering and Push-To, so a test hands in its own set (`always_licensed` for fakes) instead of
  filling process-wide slots. Accessors are licence-gated per call: a lapsed licence degrades the next frame.
  A Pro default is keyed on the build (`best_available`, `ships_*`) and resolved per frame (`resolve_rejection`,
  `resolve_background_algorithm`, the boost gated in `get_render_pipeline_config`): an unresolved RBF failed every
  lapsed frame's background, and a boost Community could not run held the contrast out of the LUT.
- Layers are crates, so Cargo enforces them: `night_amplifier` (root: `server`, `app.rs`, binary; re-exports the
  others at their old paths) → `night_amplifier_session` (`crates/session`: use cases, `AppState`, events) →
  `night_amplifier_core` (`crates/core`: the domain). A `cfg(test)` shortcut in session (e.g. `DISCOVERY_TIMEOUT`)
  only exists for session's own tests — test it there, not through the server. Plugin traits speak domain types — `push_to::{types, events}` (the `PushToEvents` port), `stacking::CometSettings`,
  `render::denoise::DenoiseSettings` — and the session maps them to `ServerEvent`. Pro never imports `server`, and
  `session` only in tests.
- Errors at a boundary are typed: plugin traits return `StackError`/`PushToError`, `LiveStacker` a `LiveStackError`,
  use cases an `ApiError` (Push-To's a `PushToError`), and the server maps each to HTTP once (`HttpStatus`). Routes used to pick
  their own status, so a missing Push-To plugin was a 404 on one route and a 500 on the next; it is 403 everywhere.
- `AppState` is aggregates that keep their locks private (`CameraRoster`, `CaptureControl`, `SessionStats`,
  `SettingsStore`, `CaptureResume`, `PushToState`): std locks never held across an `await`, so capture threads read
  without `block_on`, and a decision taken on an earlier read goes through the aggregate's `transition`
  (compare-and-set): a bare read lets two concurrent Starts both pass. Bare locks stay private. The capture state
  has no plain setter outside tests (a Stop during startup was overwritten by `Capturing`), and Start refuses
  `Stopping`: it reset the cancel flag the stopping pipeline polls, which then captured on behind `Idle`.
- f32 normalization: all pixel math uses [0.0, 1.0] to prevent overflow.
- Pixel constants have one home, never a retyped literal (Pro too): `frame::{luminance, LUMA_REC709, sample_to_u8,
  sample_to_u16}`, `render::simd::luminance_x4`, `statistics::MAD_TO_SIGMA`. `LUMA_REC601` only where a consumer was
  built on it (simulator, AI denoise). The ~45 copies these replaced were bit-identical; keep the summation order.
- Rayon for multi-core processing; no allocations in hot paths (pre-allocated buffers where possible).
- ARM friendly (optimized for Raspberry Pi 5); FFI safety — all C/C++ calls wrapped with `catch_ffi_panic`.

# Test Guidelines

If you can't fix a test, don't simplify it into not testing the idea. Tests can take a minute or two; benches longer.
**Never run benchmarks alongside other tests/tasks** — it skews the numbers. One benchmark run per **machine**
at a time, counting other worktrees, checkouts and sessions: check `pgrep -af '[_]benchmark-|[c]argo bench'` comes
back empty before starting, and wait otherwise. Real-data fixtures live in
`DEFAULT_FIXTURES` (`tests/integration/common.rs`), download on demand, and count as present only if they hold
the files and bytes registered from their archive (`Contents`: a hand-cut IC 59 passed for the published set, and
a stack cut from it put IC 63 0.46 of a frame off); a test wanting one calls
`stack_depth_grain_tests::managed_session`, which **panics** rather than skip. A pending set registers its
contents too. Coordinates measured on a stack name its size (`NamedTarget::frame`).
Measurement instruments (`tests/integration/instruments.rs`) are shared with Pro via `#[path]` — no `crate::`.
A stacking test replays a fixture through `instruments::Replay` (simulated camera → `convert_captured_frame` →
`stack_frame`; a SER in the directory is one frame per frame it holds, its mosaic served raw), never a re-wired
copy of the pipeline: the old batch `StackingPipeline` scored 59 % on a set
production stacks at 87 %. A whole session at shipped settings is `instruments::replayed` (with `stack_snapshots`/
`accumulator_snapshots`, always on `load_sub`): stacked once a run into `target/tmp/replay-cache/<run>` (`replay_cache`;
nextest's run id, else a fresh directory per process: a bare pid repeats in a container), read back bit for bit by
every other test, never by another run — an earlier build's stack would pass for this one's. Its key names each
plugin slot's type (`Plugins::implementations`), not the plugin's state. An entry is about a frame per depth; a run's
entries stay until the next run. CI runs every module reading it on one runner (`REPLAY_MODULES` in `ci.yml`); the
other partitions set `NA_REPLAY_CACHE=elsewhere`, which fails a test that reaches it (`off` turns it off; any other
value moves it). A test rendering a stack's depths renders every one through `instruments::replay_held`
(one `PreviewAnalysis` and `StackCrop` per order, cut by `cut_for_render` as the render task holds and cuts them,
so its crop moves are probed), never a fresh analysis per depth:
fresh renders gave M33 back 4 levels production never shows, and hid the deep dumbbell's 3 at an auto-crop change.
Server tests fake cameras with `camera::testing::FakeCamera` (builder, plus shared `CameraControls` to script or
count calls at runtime), not a hand-written `Camera` impl; Pro's tests get it, `FakeCatalog` and
`AppState::new_for_testing` via `test-support`.

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
cargo test --no-fail-fast                                           # fast unit tests, all three crates
cargo clippy --workspace --all-targets --features telemetry,api-schema -- -D warnings  # as CI lints
cargo test --test integration_pipeline -- --ignored --test-threads=1 # integration (slow, ignored by default)
cargo bench --bench <name> -- --noplot                              # see Benchmark sizing
cargo run --release -- [port]
cargo run --release --features telemetry -- --telemetry
cargo run --release -- --span-timings                               # per-stage durations on span close
UPDATE_API_TYPES=1 cargo test --features api-schema --test api_types # regenerate web/src/composables/api.{types,settings,events}.ts
```

**Every run goes to the end**: `.config/nextest.toml` (both repos) sets `fail-fast = false` for every nextest
run, CI's archive runs included, and `cargo test` takes `--no-fail-fast` (no config key; it stops at the first
failing binary otherwise). A fail-fast run once left 114 of Pro's 156 integration tests unrun behind one failure.

**Clippy while editing**: point the IDE's on-save check at clippy (rust-analyzer `check.command = "clippy"`, or
RustRover's external linter). It costs about a second over `cargo check` (2.0 → 2.8 s after touching one core file).

**CI steps both repos repeat are composite actions in `.github/actions/`** (system deps, npm, benchmarks, disk
space); Pro runs them from its Community checkout (branch of the same name, else `main`), so a change to one lands
in Community first. Pro's own `checkout-siblings` does that checkout; step timeouts stay at the call site.

Frontend commands (install/dev/build/typecheck/lint/test) run from `web/` — see README. There is no formatter:
format by hand as the surrounding file does (Prettier was removed — no config reproduced the code, it rewrote 193
of 262 files and the generated ones). Always run `cargo test --no-fail-fast` after changes, `npm run test:run` and `npm run typecheck` too.
A frontend test that reaches `fetch` unmocked fails (`src/testSetup.ts`) even where the code swallows the error:
happy-dom sends `/api/...` to `localhost:3000`, so one sent 40 real requests a run that anything on that port would
have answered mid-test.

## Core Modules (crates/core/src/, crates/session/src/, src/)

| Module             | Purpose                                                                       |
|--------------------|--------------------------------------------------------------------------------|
| `frame/`           | `Frame` with normalized f32 pixels; format conversion                         |
| `fits/`            | FITS read/write; NAXIS layout                                                  |
| `debayer/`         | RGGB/BGGR/GRBG/GBRG; Bilinear + VNG + Superpixel                               |
| `cfa/`             | Raw-CFA stage before demosaic: hot pixels, row/column FPN                      |
| `render/denoise/`  | Denoise plugin boundary: settings, config data, buffer pool, `ai_compute`      |
| `calibration/`     | Master dark/flat: `(raw - dark) / flat`                                       |
| `detection/`       | Star detection, CoM centroiding, FWHM/SNR                                      |
| `registration/`    | Triangle matching + RANSAC → `AffineTransform`                                 |
| `stacking/`        | `MasterStack` accumulator, rejection, warping                                  |
| `background/`      | Gradient model: node layout, sky/target classifier, catalog protection, edge profiles |
| `render/`          | Stretch, autostretch solver, white balance, S-curve; `display/` the f32 → RGB8 kernels and PNG |
| `camera/`          | Traits + vendor SDKs + simulator                                               |
| `planetary/` / `ser/` | Correlation alignment + percentile stacking; SER video read/write           |
| `disk_writer/`     | Async bounded-queue frame writer                                               |
| `plugins/` / `push_to/` | Pro-delegated trait definitions, incl. Community-side Push-To (impl in Pro) |
| `crates/session/`  | Application layer: `AppState`, capture/guide threads, camera lifecycle, services, events |
| `server/`          | Axum REST + WebSocket adapter over `session` (api, ws, dto, assets)            |
| `app.rs` / `parallel.rs` | Shared `app::run()`; `balanced_chunk_len` rayon partitioning, both shared with Pro |
| `ffi_safety.rs` / `native_library.rs` | `catch_ffi_panic`; eager dlopen of vendor libraries             |
| `logging.rs` / `telemetry.rs` | `tracing` + optional OpenTelemetry (OTLP)                           |

### Server & Web Frontend (src/server/, web/)

Axum: REST `/api/*`; WS `/ws/stream` + `/ws/eyepiece` (JPEG), `/ws/eyepiece_quality` (lossless LZ4), `/ws/events`
(JSON). `GET /api/cameras` carries `phase`/`warmup_remaining_s` for every client, not just the one that acted.

**Every `/ws/events` event is a topic or a notice** (`session::events::sync`). A topic is state the server can always
restate, *including "none"* (`target_cleared`, `camera_recovery_cleared`), and is changed before its event is sent —
one aggregate records and announces under one lock (`PushToAnnouncer`, `Installs`, `DiskWarning`, `RecoveryStatus`).
A notice (`error`, `focus_mode_left`) may be lost. `ServerEvent::topic()` has no wildcard arm, so a new event does not
compile until classified. On connect and after a `Lagged` client (whose backlog is dropped, debug log only, no toast)
`send_sync` sends every **Value** topic's snapshot then `synced {cause, dropped}`, on which the client re-reads every
**Refetch** topic (settings, AI compute, camera list) via `resyncOn` — the list as `?discover=false`, the last
enumeration's devices: one per sync opened every idle QHY/ZWO camera on each reconnect and lag of every page. Before this, only 4 of 37 events were
restatable: a lost install end froze the installer and a lost `push_to_blocked` was never recovered, by any client.
Sticky topics need an end: `Installs` runs one install per kind (409 `InstallInProgress`; a refused second one once
failed the first), and a page that is installing reads a sync's `idle` as interrupted, not done (a restart showed
"installed"). A recovery give-up stays until a connect into the role or `DELETE /api/cameras/recovery/{role}` — the
warning's dismiss — since a page-only dismissal came back on every sync, all night.
`api.events.ts` is generated with every event's topic and kind; an event that flattens an enum
(`*_install_status`) is a union of its branches. Every handler table is `satisfies EventHandlers`, so a handler for
an event the server never sends, or a field it does not carry, does not compile; a handler may be async (`dispatch`
logs a rejection as it logs a throw, as `useWebSocket` does for `onMessage`);
`useEventStream.catalogue.test.ts` fails a state event no table handles (type-level, and once wired) and a refetch
topic not re-read on sync.
**Eyepiece sockets follow the operator**: `/ws/stream?source=` is fixed per socket (`/`), but both eyepiece sockets
show `AppState::viewed_camera` (set by `/`'s Guide toggle via `PUT /api/view/camera`, snapshot route too) and switch
in place; their viewers have no control. A guide *disconnect* falls back to Main only after the roster says so
(`WarmingUp`, else the entry gone — before it, a racing select strands the view); a recovered fault keeps it.
`ViewedCamera` announces each change from under its lock, so racing toggles reach clients in order.
`GET /api/eyepiece/snapshot?circular=` is the only REST route returning image bytes; native size is costly, so
`SNAPSHOT_SLOT` *refuses* rather than queues (503 + `Retry-After`).

Vue 3 SPA, mobile-first, dark theme, proxying `/api`/`/ws` to `localhost:9955` in dev. Pro-only controls stay
visible and locked (`BaseProLock`) on `/api/capabilities`. `useCatalogSearch` skips a programmatic query by
*value*, never a one-shot flag — a flag-based version once left M1-M9 unfindable after clearing a 1-character query.
Sockets: `useWebSocket` (reconnect, ping; no answer within `livenessTimeout` — two ping intervals, five minutes for
image streams, since a lossless frame on weak Wi-Fi outlasted a minute and was cut mid-frame forever — drops a
half-open socket with close code 4000, which the server logs beside each frame slower than 10 s to send),
`useEventStream`, `useImageStream`.
**`/ws/events` has one dispatch path**: `useEventStream` is the socket plus `on(type, fn)`, every message handed to
its type's subscribers in order — the domain tables in `composables/events/` first, then the stores, which subscribe
through `connectEvents` from `App.vue` (a `lastEvent` watcher saw one of two events landing in a tick). **One writer
per ref**: Push-To's target is written only by `events/pushToEvents.ts` (the panel hands it answers via `aimAt`),
settings only by `settingsStore`: `applySettings` (`useApplySettings`) adopts the POST's answer, and `refresh` never
runs beside a write, coalesces, and drops an answer older than the one adopted — an edit is one POST and at most one
GET (it was a POST and 2-3 GETs). A refresh asked during a request reads once more after it. Every `api.ts` call has a deadline (15 s; 120 s for connect, capture
start/stop, installs, snapshot): one hung POST held every settings refresh behind it. Stores are `createX()` + a module default + `__resetForTests()`; a composable's
lifecycle hooks go through `lifecycle.ts` (`tryOn*`), which a bare call or a module store skips. Polling only while
the socket is down: AI compute follows `ai_compute_changed` and the saved choice, a failed first load retries on sync.
**`api.types.ts`, `api.settings.ts`, `api.events.ts` (and the tests' `src/test/api.examples.ts`) are generated** from the Rust wire types, the code that clamps settings and `ServerEvent::examples()`
(`tests/api_types.rs`, feature `api-schema`, which derives `schemars::JsonSchema` only in that build); CI fails a
stale one. `DEFAULT_SETTINGS` *is* the server's default, and a control's range comes from `serverLimits(path, ui)`,
which may narrow but never widen it (`constants/serverSettings.test.ts`), and takes only a path the server clamps
(`ClampedPath`); constants live by domain behind `constants/index.ts`, help copy per panel in `constants/help/`. A settings panel section is its own component that
mirrors its keys of the panel's `settingsMirror` (`useSectionMirror`, which leaves keys edited but not yet sent out
of syncs) and emits `apply(key, value)`; one that must await the save takes it as a prop. The server replaces the
`eyepiece` group whole and two sections edit it, so they emit only changed fields and the panel (`applyEyepiece`)
sends them over its copy — a section's stale copy reverted the other's typed screen size. An emptied number field
sends nothing and reads its last value again (`''` was a 422, `Number('')` a 0). Only a typed number debounces (`SETTINGS_DEBOUNCE_MS`); a slider release or a
select sends at once. Cooler, dew heater and binning save through `useCameraHardwareSettings`, which names `camera_role` (binning
without it rebinned the imaging camera while the guide was selected).
**Debounce through `useDebouncedCall`**, which flushes on unmount: Vue drops an `emit` from an unmounted component,
so a hand-rolled timer lost an edit made just before its panel closed; a one-shot delay goes through `useTimeouts`,
and a window/document listener through `useEventListener` (`useClickOutside`), both cleared on unmount.
**Both views draw through `useFrameRenderer`**: WebGL2 → WebGL1 → Canvas2D per canvas, a JPEG decoded once for every
eye, one decode at a time with the newest waiting frame next (dropping every overtaken decode froze a view whenever
decoding was slower than the frame rate), the last image held for a restored context or a new canvas. A canvas hands out
one context type for life, so `useWebGLRenderer` probes each GL version on a scratch canvas first — a WebGL2 shader
failure on the real one once left it blank, with no Canvas2D fallback. The sidebar is **one tab at a time**
(`useSidebarTabs`: Main, Push-To, Settings), each mounted on first visit and then only hidden (`v-show`) —
remounting dropped typed coordinates and refetched. A hidden panel's watchers keep running, so mounting writes only
to record the selected camera, never optics (`useTelescopeSetup` once rewrote the telescope block on every Push-To
open), and a kept-mounted panel **follows** server state rather than snapshotting it — a profile map read once wrote
the main scope's focal length over the guide's seeded profile on the next camera click. A stored profile is shown
**whole** (a seed's missing focal length stays missing), a connect broadcasts `settings_updated`, and a sync
arriving mid-save is held until the save settles: each gap once posted the previous scope's optics or a stale value.
The camera list lives on Main, so a section editing the selected camera names it (`SelectedCameraTag`; Equipment,
with none selected, names the profile it edits), and a debounced edit is bound to that camera.

**Shared homes**: App state is provided once (`provideAppContext`) under typed `InjectionKey`s (`Symbol.for`, so a
re-imported module names the same keys), read through one accessor per key in `appContext.ts` (`useSettings()`,
`useCapabilities()`, `useSelectedCamera()`, …), each with its one default — a script-side read of the provided readonly
ref once locked comet stacking for every Pro user. Tests provide by name: `mountWithDefaults`, or `contextProvides({…})`
in a mount's `global.provide`. `api.ts` rejects with an `ApiError` carrying the HTTP status; a caller to whom a
Pro feature is optional uses `ignoreProOnly`, never a bare `catch {}`. Formatters, coordinates (rounded *before*
splitting), FOV (`2·atan`, as Pro's solver) and view geometry live in `utils/`. Styles take tokens from `main.css`
`:root` (colour, type scale, radii, z-order, shadows) and its shared classes; `src/styles.test.ts` fails an undefined,
self-referencing or fallback `var()` and any colour literal outside `:root` (an undefined `--danger` once blanked nine
components' error icons). A `.btn-x.active` rule needs its `:hover:not(:disabled)` twin (the test checks): a touch
screen leaves `.btn:hover` stuck on the tapped button, which hid the Settings and Push-To tabs' active colour. Test helpers live in `src/test/`: `mockApi` (an override of the wrong shape does not compile), fixtures typed by `api.types.ts`,
`createEventStreamStub` held to `useEventStream` and `serverEvent(type, fields)` typed by `api.events.ts` and
complete (its fields over the event's required ones, a nullable one `null`: a handler reads them without a fallback),
`settings(overrides)` (a partial group over its default; the panel's `settingsMirror` is a plain copy), `mountWithDefaults`, `stubFetch`, `renderStubs` (canvas contexts as a
browser hands them out); tests sit beside their source (`X.test.ts`, `X.<aspect>.test.ts`). Wire enums the UI
spells (`CAMERA_PHASES`, `STACKING_TYPES`) are constants held to its `*_VALUES`. A wire value the UI compares
against is a Rust enum, never a `String` (`DirectionHint`, `CaptureStatus.state`, an install event's `stage_id`);
the server sends no display text the UI does not show.

## Camera Notes

- **Camera roles**: at most one `Main` and one `Guide`, held per role by `AppState::roster` (`CameraRoster`):
  entry, selection, phase, status and fault records under one `std` lock, so a disconnect is one transaction and
  capture threads read a phase without `block_on`; a decision taken on an earlier read moves the phase with
  `transition` (compare-and-set). Every handle/monitor/cancel-token lives in `slot(role)`. `connect` to a taken role
  swaps while idle, else `CameraRoleBusy`.
- **Cooler lifecycle**: `Precooling → Idle → Capturing|Guiding → WarmingUp`, ramped ≤5°C/min. **Disconnect** stops
  capture first and cuts an in-flight sub short (≤15 s, stack saved) before closing the cooler — Stop alone once let
  a 300 s sub outlast it; a watchdog ends every warm-up by deadline regardless of the monitor.
- **Per-camera profiles** are keyed `"{provider}/{model}"`; **telescope profiles** by camera *name*, seeding the
  solver's FOV — without a seed, two bodies of one model share one FOV and ASTAP fails to solve (once cost 19 min).
  A settings write restarts a solve only if the **solver camera's** resolved optics or binning change
  (`optics_change`, `AppState::solver_camera`): the UI re-posts the selected camera's profile as the flat block,
  which restarted the solve on every camera click. Profile writes **upsert**, and the UI sends only the profiles it
  changed: a whole map fetched before a connect's seed erased it, and the guide solved on the imaging scope's optics.
- **Vendor SDKs are dlopen'd, never linked or shipped**: an unused `#[link]` breaks every build without that SDK,
  and QHY grants no redistribution. QHY/ToupTek/SVBony bind eagerly, since a lazy unresolved symbol kills the
  process at first call. C `long` is 32-bit on Windows, widen with `i64::from`.
- **Guide camera is one thread, not the pipeline**: nothing stacked/queued, started by `connect` so solving/preview
  work *while* framing. Post-processing runs only while `guide_stream.has_viewers()`; solving/raw saving sit above
  both early exits. Raw saving is a debug-mode diagnostic (`saves_guide_raw_frames(state.debug_mode())`): outside
  it a saved switch is ignored, not cleared. `AppState::debug_mode` is set once by the server and is what
  capabilities report as `debug_logging`, since `tracing::enabled!` answers per crate and the UI could hide a switch the
  session still obeyed. Two `FrameStream`s, two counters, so one camera's payloads can't invalidate the other's;
  both encode JPEG and lossless per watched family (`stream_encoding::encode_watched`).
- **QHY and ToupTek handles are pointers** the close frees: every shim call goes through `open_sdk`, which refuses
  once the lease has closed. QHY's frame calls take no length and may write `GetQHYCCDMemLength` bytes, past the
  ROI's frame; a smaller buffer reads through a scratch copy. ToupTek refuses a buffer under its RAW frame.
- **Vendor closes take a device *index*, not a handle** — a stuck call's late `Drop` can close a reconnected
  camera; every close goes through `lease.begin_close()`, and an abandoned handle is never closed eagerly.
  `{provider}_{index}` ids follow USB enumeration order, so a guide reconnect can install the *imaging* camera —
  recovery keeps a device's **recorded id** even if the index moved.
- **Vendor capture loops share `camera::exposure::ExposureLoop`**, INDI's too: a vendor implements `SdkExposure`
  (apply, start, abort, poll, plus its quirks) and the loop owns re-apply, stream/single switching, cancel and stall.
  The stall clock starts on entering `capture`, re-apply included (`CaptureConfig::stall_budget` says why). A stream
  is always stopped *before* a config is applied — SVBony's header demands it for the ROI, ToupTek's for the
  resolution, and both INDI drivers do it. INDI frames arrive as self-describing FITS BLOBs (`Poll::Delivered`),
  watched from before the exposure is triggered; its XML speaks the protocol's spelling (`Idle`, `On`, `Also`).
- **Fault detection**: one detector with a per-role+name streak serves all watchdogs, so alternating faults still
  escalate. Recovery is a ladder (stream restart → quiet suspend → supervisor retry → give-up), silent before 20 s;
  a failed hand-off always resumes the monitor. A reopen is not a recovery until a **frame** arrives: opens that
  fault before their first frame are counted per role+name (`camera_health::FramelessVerdict`); the 2nd reopens with
  the same-provider sibling closed and **held closed until the first frame** (`reopen_hold`, deadline-bounded; never a
  capturing Main or one warming up after a Disconnect), the 3rd gives up once the run is 60 s old — 2026-10-12's
  Ares-C PRO reopened 20 times in silence beside an open Neptune-C II, and streamed once that was closed. A Connect,
  a Start, or a reopen no capture judges ends the run. A monitor call hands its handle back only if the slot is
  neither `Suspended` nor past that **monitor's generation** (`claim_monitor_epoch`, retired by a suspension):
  closing on `Installing` closed the live reopened camera. Only a *named, different* camera discards the FOV cache,
  since a stale FOV *fails* hinted solves.
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
100/101 are RGB/BGR. **Its header counts its frames before the stack is done**: the storage task finalizes the
container as it ends (`finish_container`; `EndSession` waits for every pipeline thread, and 2026-10-24's 26.5 GB Moon
kept a count of 0 behind a stacker until the power went), and the writer rewrites the count every 32 frames and on
every flush. `SerReader` trusts a header the file size agrees with, else reads the whole frames the file holds. Layout: `captures/raw/DD-MM-YYYY_HH-MM-SS-<mode>/frame_NNNNNN.fits` (or `capture.ser`) and
`captures/stacked/...-stacking.fits`. **A resumed capture rejoins its folder and must not overwrite it**: the
resume plan carries `next_frame`, and video writes `capture_2.ser`, etc. **A session is a name, not a folder**:
the worker creates the folder with the first frame (eager creation once left 23 empty `-live` folders in a night),
so `DiskWriterHandle` remembers issued names to keep same-second sessions apart, and a session start **probes**
`raw/` with a throwaway mkdir — the folder was the only check a read-only/full disk refused Start on. Failed writes
go to the UI once per episode (`FailureSink`), never just a log line per frame. The main capture opens one only
when *it* saves (`main_capture_saves`); guide saving keeps the writer on but files into the guide's own session.
**A write is not on disk until synced** (`core::durable`): 2026-10-16's Orange Pi lost power seconds after a Stop
and kept a 0-byte stack, PNG and three subs, all logged "written". The worker records every path it writes, per
`SyncGroup` (`Shared`: the handle's session, the imaging camera and its stack; `Owned`: the guide loop's), and
on `DiskWriterHandle::sync(group)` hands them to a `disk-sync` thread — on the writer's own, the guide's stop held
the imaging camera's frames until its queue dropped subs. Each camera syncs when *it* stops — the main pipeline
after `end_session` and before `Idle` (a Stop hands the camera back first, so a Disconnect's warm-up never waits
on the card), the guide loop after its hand-back — announcing `storage_sync` per role (a Value topic: `syncing` →
`synced`/`failed`, `idle` again when it starts; a run counter drops a sync's verdict that a restart overtook).
Syncing at the end, not per file, keeps a slow SD card from backing up the queue. The final stack *waits* for a
queue slot (`queue_stacked_*`); a raw frame is turned away. **Every wait on the writer is bounded by progress**:
`Unresponsive` after `STALL_PATIENCE` (60 s) with no write or sync step, or a pulled drive held the capture in
`Stopping` for ever. SIGTERM/Ctrl-C runs `CaptureService::shut_down` (stop, sync, 75 s budget) before exiting.
Config files (`settings.json`, Pro's state, AI-compute store, licence) go through `durable::write_atomic`: temp,
sync, rename, one at a time process-wide (writers share `<name>.tmp`; overlapping ones exposed a 0-byte file), a
snapshot of shared state built under that lock (`write_atomic_with`). Settings save off the runtime and coalesce
(`SettingsWriter`). A directory `fsync` refused with `EINVAL`/`ENOTSUP` (FUSE exFAT/NTFS) counts as done.

## Streaming Protocols

### Dynamic JPEG (SA10) — `/ws/stream`, `/ws/eyepiece`

Default format; TurboJPEG (SIMD) encodes in the render task, not the WebSocket handlers. **Quality follows the
denoisers**: 95 below 1440p, 90 above, but 95 at every size once denoise is on — q90 would erase what the dither
and denoiser add.

```
Magic "SA10" (4B) | Width u32 LE | Height u32 LE | Payload size u32 LE | JPEG bytes
```

An eyepiece socket switched to a camera with no frame yet gets the text `no_frame` (on the image socket, so ordered
with frames), or the old camera's picture would stay up as the new one's. `FrameStream::clear` publishes a
payload-less version, so open sockets get `no_frame` too; Main is cleared on a fresh Start and on disconnect (never
on a quiet suspend), or the last session's frame passed for live for three minutes of a frameless capture.

**Streaming resolution is a setting, not negotiated**: every client of a family gets the **same payload**
(`streaming_resolution` / `eyepiece.stream_resolution`, both default Native, each read only by its family) — per-viewport tiers were tried and
reverted (one denoised conversion per distinct size plus unbounded concurrent first-frame encodes). **No stream exceeds
`MAX_STREAM_EDGE` (3840 px a side)**, Native included (`Resolution::bounding_box`; processing, snapshot and saved files
stay native): a 26 MP frame blacked out GPUs with a 4096 texture limit and cost ~940 MB of denoise scratch. The browser
reads it as the generated `SERVER_MAX_STREAM_EDGE` and also cuts a source to the GPU's `MAX_TEXTURE_SIZE`, keeping the
canvas at the frame's size so geometry stays in stream pixels.

### Lossless LZ4 (SA08/SA09) — `/ws/eyepiece_quality`

Lossless (beyond 8-bit) for the eyepiece quality view; SA09 chunks it for parallel LZ4; WebGL with Canvas2D
fallback.

```
Magic "SA08" (4B) | Width u32 LE | Height u32 LE | Compressed size u32 LE | LZ4 RGB8 payload
```

**Downsampling** area-averages down to the configured box (`render::display::axis_taps`: a 1 px tent per source sample,
not a whole-pixel box, which once averaged a different sample count on different lines and left a visible
lattice). WebGL needs `UNPACK_ALIGNMENT` 1 for the frontend's unpadded RGB rows.

## Adding a Stacking Type / Settings Persistence

Add a `StackingType` variant (`crates/core/src/stacking/config.rs`), update `all()`, and implement its capability methods
(`display_name`, `uses_star_registration`, `uses_fpn_removal`, etc.). A type with its own accumulator also gets a
`LiveStacker` impl (`crates/session/src/capture/context/`) and a `create_live_stacker` arm; the stacking task drives every mode
through that trait, and a carried stack only resumes under its own `kind()`.

**`CaptureSettings` is the one settings schema**: `settings.json` (in the server working directory, loaded on
startup, saved on `POST /api/settings`), the `/api/settings` answer (minus the Focus/Finder snapshot) and the
defaults are all its serde. A new setting is a field there, one in `UpdateSettingsRequest` and a name in
`SettingsService::apply`'s copy list. A key missing from a file takes `Default`; a key an older build named
differently is rewritten by `settings_persistence::migrate`; ranges live in `CaptureSettings::sanitized`, which load,
update and save all run. `SettingsService` checks a request whole before applying any of it — a refused one changes
nothing — then runs the reactions its `SettingsDelta` names, outside the write lock.

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

**Deep Sky**: scale/rotation-invariant triangle patterns + RANSAC → `AffineTransform`, then **refit over every
detected star** (`registration::refine`): the ladder fits from ≤30 stars with 6–8 px inlier slack, which left
corners 1–4 px off and a stack's corner stars up to 19 % fatter. `FrameGate` still judges the **ladder's** fit: the
refit locks onto one image of a multi-image sub, which then slipped past the residual gate and lit the sky around
every star.

The ladder sees **clutter-suppressed** lists (`registration::clutter`, one detection per 25 px): noise on a bright
nebula makes maxima brighter than a thin sub's stars (Cat's Eye 0.7 s: 27 of the top 30), and every fit was a
coincidence. No credible rung → **voting** (`registration::translation`): shifts at turns within ±0.6° of the
session's last rotation (`register_near`'s prior), since on shifts alone an alt-az field that had turned 0.3° lost
every sub only voting registers. **Credible** (`Support`) = pairs within 1.5 px against chance at the *local*
density (Poisson ≤ 1e-6) and scale within 1 %, the stronger of the full and suppressed lists (each fails a real
set: nebula maxima, M42's fat stars). It replaced a 25 % match share that a thin sub's true fit (10–19 of 200
detections) never reaches. A vote also pays for the ~10^4 offsets it searched (`Support::searched`): unpriced, 1
in ~100 unrelated sparse fields passed. A refit with < 8 pairs holds scale at 1 (a thin fit carried 1.0034).

**Planetary**: surface cross-correlation in a window half the frame's short side (on the reference's centroid with
auto-tracking), searched **coarse to fine** (`planetary::alignment`): exhaustive only on the coarsest 2^k level of a
`LuminancePyramid`, then a climb to a 3x3 local maximum on each finer one, whose neighbours the subpixel fit reuses
(fixed ±2 px boxes could hand back their own edge); the reference's windows are built once (`AlignmentReference`).
The exhaustive ±50 px search over 1504² took 134 s a frame on the Pi (2026-10-24's Moon) and Stop waited behind it;
now ~8 ms on 20 x86 cores. The search starts at the **last correlated shift**, then ±200 px around the centroids'
estimate: that Moon drifted 255 px in 80 s, and 77 % of its frames matched on the edge of a ±50 px search around 0.
A match on the search's or the frame's edge (`TargetOutOfReach`) or under 0.3 (`SurfaceNotFound`) is **not
stacked**, and the status bar says which; only a flat window stacks by centroids. Pro's multi-point gets the global
shift as its `Search` (±8 px a point) and the session's pyramids, building none.

**The window keeps 16 px from the frame's edges** (an eighth of a short side at most): clamped against one, a
frame that had not moved peaked where its outward neighbour lay off the frame, so a planet off to one side lost
every frame that had not moved in. Its NCC sums are taken about the reference's mean: one-pass f32 `E[x²]-E[x]²`
placed a 0.07 %-contrast surface 3 px off at NCC 0.99. Guarded on the `moon-drifting-imx533-ser` fixture
(`planetary_tests`, oracle shifts in `planetary::FRAMES`) and `planetary_moon_benchmark`.

**Comet** [Pro]: centroids the nucleus so stars trail while the comet stacks sharp.

A sub that **will not register** is explained before it is called movement (`FrameGate::explain_unregistered`):
doubled stars (a bump: companion offsets that *agree*, `detection::doubled_star_share`, judged against the
session's own baseline — a dense field scores ~55 % on every sub) or bloated ones are soft verdicts Wanderer holds
through. The baseline is the share's **lower quartile**, so a rough night's doubled subs can't become its normal.
Doubled needs same-field proof (collapsed, the sub registers); bloated has none, so `BLOATED_RUN_LIMIT` (5) in a row
is movement — nothing unregistered updates the star size, and a new field of fatter stars held the stack forever.
Registered doubled subs still stack: dropping them cost 6.6 % sky noise for 2.5 % smaller stars. Wanderer restarts
on **two** movement verdicts in a row (`capture::wanderer`): a thin session's fits sit near the bar, and one
marginal sub used to wipe its stack.

### Phase 5: Live Stacking & Rejection

`MasterStack` accumulates in O(1) memory (16 B/pixel, 434 MB at 3008²x3). **Never estimate the clip threshold from
samples that survived the clip**: an early underestimate once rejected the samples that would have widened it,
discarding 15% of real subs. `m2` is now a running mean of squared deviations over *every* offered sample, rejected
ones winsorised to the threshold — cosmic rays can't widen the window, but a collapsed scale still recovers
geometrically. `RejectionMethod` has four variants but the live path implements only two (`MinMax` would need
per-pixel min/max, +650 MB); the plain-mean path maintains `m2` too, so a settings toggle lands mid-stack without
opening a gap.

**The warp re-anchors its row walk** every `ANCHOR_EVERY` (32) pixels (`stacking::warp`): stepping a whole 3008 px
row in f32 drifted samples up to 0.37 px (a 1.2e-4 rad turn is half an ulp past 2048), now 0.005; computing every
pixel from its index cost 10 % of a rotated warp. Registration is held to a least-squares oracle on the same subs
(`registration_keeps_what_a_least_squares_fit_would`), which seeing cannot fail as it fails a sub-vs-stack size.

**A stack's first six subs are on probation** (`context::probation`): sub 1 used to become the
reference unjudged (a bumped one: stars +40 %), and Pro cannot clip a pixel's first three samples.
They stream as before but are held at f16 (54 MB each at 3008²x3); at sub 6 the reference is
elected by **consensus** (median pairs the others line up on it; never FWHM, which reads a smeared
sub *sharper*), sub 1 keeping it within 90 %. Subs pairing under half the best are `Outvoted`, the
rest face the gate against the agreeing subs' medians (three clouded subs of six dragged the whole
batch's down until the clear ones read bloated), and the stack is rebuilt once
(`MasterStack::restack`, set-wide weights, Pro's `reject_batch`); no FWHM re-base follows, unless
nobody was elected. A restack refuses before clearing or not at all (plugin resolved once, the
plain mean if it errs), a batch drop counts as `offered`, and `StackedFrame::stack_generation`
tells the render cache. Held subs are capped by `MemAvailable` (`probation::affordable`).
`early_subs_tests` / Pro `early_subs_regression` score it on `early-bad-subs-imx533` and injected
defects against an oracle and the legacy stream (`StackSettings::probation_subs` 0).

**Auto crop** (`stacking::crop`, setting `auto_crop`, off = nothing cut anywhere) is the largest rectangle
whose coverage cells all hold ≥ 0.95 of the stack, kept while they hold ≥ 0.85, and none before depth 20
(`MIN_CROP_DEPTH`: shallower, 0.95 is every sub, and one early bump cut the view for 14 subs). The subs'
common footprint lost 44 % of a frame to one bumped sub, and the raw rectangle moved on 51 of 64 drifting subs. It is
decided once per snapshot in the stacking task and travels as `StackedFrame::crop`, so live view, eyepiece
and saved FITS/PNG agree. It is cut **before the render pipeline**: the thin border inflated the sky MAD
the stretch is solved from by 13-20 %. The bin is resolved on the uncropped size, a crop change keeps
`NoiseTrend` (rebased), and solving keeps the uncropped frame. The render task cuts and bins in one pass
(`cut_display_frame`): `make_mut` on a stack a solve shared copied all of it (4.1 -> 42.7 ms). Its
`stack_crop_changed` names the live frame size the cut arrives at, and the live view waits for that frame:
the two travel on different sockets, and reframing an old frame still in flight lost the zoom.

**A crop move is not a change in the noise** (`capture::analysis`, `NoiseTrend::move_window`). The frame MAD reads
the target's structure too, so a move that cut sky rose it +14 % in one sub (the 106-sub dumbbell at 41) and gave
back 3 levels of target contrast by 64. Each move is measured **two-sided**: the stack's rows on the crop it left
(`render_task::cut_for_render` hands them over) and the frame's on the one it entered, through the last window's white
balance and model (`BackgroundModel::windowed`, ≤ 3.5 % past it; `every_nth_row`), ~400k pixels a side on rows of the
stack's grid, so the overlap reads the same pixels (~10 ms a move). Their ratio sets the window, kept for the stack
across settings and Processing Resolution changes, and by the saved PNG with the crop it was measured on
(`StackNoiseWindow`). Measured against the trend instead, each move took the trend's error once its exponent froze
(0.61 by 300 subs; the dumbbell 0.81, now 0.87). The frame still refits on schedule and that step is served, not
bridged: refits of consecutive subs read 1.3-2.8 ADU on Fireworks and 6.1-8.7 on M31.

### Phase 6: Background Extraction (Light Pollution Removal)

**One node classifier for both models** (`background::classify`; monotone cubic (PCHIP) here, Pro's TPS through the
same completed grid): a robust polynomial ladder (degree <= 4) rejects nodes 3 sigma above it, **a rejected node
whose climb up the excess ends at the frame edge is glow** and readmitted, targets grow through their wing (1 sigma,
3x3 mean, <= 2 lines) and by their own radius (1-3 lines); holes refill from the fit, never above the node's own
value, stepping down a degree where it would (a frame-filling M27 rendered black) and flat where the sky is one
corner. Nodes hug both edges plus an inset line at 3.5 % (edge falloff), and the model subtracts the **whole**
gradient: the 0.7 the stretch profiles set, a brightness prune and a crowding disc that landed off-target on every
real stack once left M57's moon band at 22 output levels and a 35 mm corner glow at 70 (now 4 each). **Pro
plate-solves each stack once** (`capture::field_solve`, depth >= 6, 90 s, never blind) and catalog footprints
(`FieldProtection`, a generation in the analysis key) stay out of the sky. It solves on the **stack's** rig (the
imaging camera's, not a guide holding Push-To, whose FOV refused every stack) and leaves Push-To's state alone:
sharing it left a Push-To spinner up and sent Push-To's next bad frame blind. Push-To comes first — a Push-To solve
cancels it — and a cancel (or Stop) retries on the next sub, never gives the stack up. Real-data guard: Pro's
`background_gradient_regression` on `background-gradient-stacks` and `background-capture-stacks`;
`background_gradient_diagnostics` cuts both.

**Edge profiles** (`background::edges`) take what either model leaves along the frame's edges: per edge, a profile
across its outer 20 %, the median of five segments along it (a target touching the edge fills one or two), each
segment's least-trimmed-squares line removed first (the grid follows smooth slopes; a plain fit through a band left
the edge 3 sigma low). 2026-10-24's M57 and Iris carried an *instrument* band at 0.85-0.90 of the sensor width (same
place on every target that night), which bilinear nodes cut and the old 0.01 RBF ridge flattened. Off for a single
frame (`edge_profiles`, from `showing_stack`): live view re-estimates every frame and a sub's noise buries a band;
~3.5-5.5 ms an estimate at 2712x1538 on 20 x86 cores. Keep 12 nodes: denser grids rejected the band as a target,
since the classifier's reach constants count node lines. `background_injection_regression` scores injected
gradients and faint nebulae on every stack, as `out(base + g + n) - out(base) - n`, so the stack's own unknown
background cancels.

**What the profiles must not take**: samples inside the classifier's catalog footprints (`protected_regions`; a
protected 15 sigma filament along an edge lost 14.6), and a **thin-coverage strip** by the edge, found as the leading
steps noisier than 1.25x the band's inner half (`THIN`; ring-2's registration border read 4.5, auto-cropped stacks
0.87-1.13) and left as the grid has it: profiled through, it printed a line 8 output levels brighter. Steps are 0.5 %
of the frame, joined linearly — held whole they jumped 1.5 sigma between neighbouring rows. The model is read at
sampled pixels (`BackgroundModel::sampler`), not rendered into a buffer: 20 threads page-faulting a fresh 6.5 MB
plane were a quarter of the estimate. Guarded by `no_line_is_printed_along_an_edge` and
`the_edge_correction_has_no_steps` (Pro), which see the outer 1 % every other measure leaves out.

### Phase 7-8: Statistics & Auto-Color

Robust per-channel statistics (Phase 7) feed background neutralization of light-pollution colour casts (Phase 8).

### Phase 9: Black Point Calculation

`black_point = mode - k * sigma`, so the sky estimate must resolve far finer than the sky itself. Reporting a
histogram bin's *centre* made the mode a step function of depth, once snapping a whole bin and blacking out half a
target — **the binned peak picks the region; the value is refined** by re-binning inside the winning bin. The sky
level comes from the **samples** the refined peak points at, whose own spread uses the samples **below** the peak
only (their *lower quartile*) — safe *upwards* against a frame-filling target and *downwards* against a population
darker than the sky.

**Where a target fills the frame the mode is the target** (M33 at 250 mm: its disk), and the real sky at the edges
sinks under a black point that closes in with depth (10 -> 5 output levels by 34 subs). Open: a real-sky anchor
(8b43910, reverted 2026-10-28) lifted M33 3 levels on average but coarsened its disk and lost its outer halo.

### Phase 10-12: Saturation Boost, Tone Mapping & Autostretch Solver

Optional shadow saturation boost, then the core asinh/MTF stretch. **Asinh** solves for `stretch_factor` such that
`input = adjusted_median` maps to `output = target_background` (default 0.15), via Newton-Raphson/Bisection. **MTF**
solves algebraically for `m`. Steps: compute statistics → black point `= Median - c×Sigma` → solve the tone
parameter → subtract black point → apply the stretch.

**The gap's sigma stays the frame's MAD**, though under structure it lies flat (M33's read 1.39x the sky's noise at
34 subs, the 106-sub M27 crop's 1.85x) and the depth gain then spends depth on nothing. Counting the gap in the sky's
noise was tried and dropped: calibrated on six sky-dominated sessions (0.88-1.05x the MAD), it put the grain back on
the curve (M33 0.59x a sub's, the deep dumbbell 0.60x against 0.46x), but the structure's MAD was also what kept
bright targets in range — M27's core burned (197 -> 251 levels) and M31 at 250 mm, whose MAD is 4.5x its faint
disk's noise, rendered core/mid/outer at 241/241/241 against 239/181/117. It needs a re-tuned depth curve and a way
to tell a frame with no sky from one with a sky ring.

### Phase 13: Final Output Mapping & Contrast

Everything below runs in the streaming encoders at display resolution, not in the pipeline proper.

**Spatial denoising** (Pro's denoise plugin): guided chroma + à trous wavelet luma, tuned in Pro
(`plugins::denoise`). Community owns the boundary — `DenoiseConfig` (`Default` is always `OFF`, byte-identical
output without the plugin) and the gates: Planetary is refused before the plugin is asked; the switch is unmanaged
by Focus/Finder mode. The **grain-split dial** (`background_grain`) spends three levers in order — below the
middle only the cheap finest wavelet level moves (fine speckle), above it the coarse pair (levels 5-6) takes over,
the only mechanism reaching 16-64 px noise; the dial's own tone-curve split never runs past `DEFAULT_GRAIN_SPLIT`
(1/8, not an even 1/4), since spending more there costs target brightness 1:1 with the grain removed. **Detail**
(levels 2-4 local contrast) ships on by default, held off the sky/stars by the plugin.

**The AI denoiser** (Pro's AI denoise plugin) is display-referred, not linear — it runs after the
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
apart collide, so whichever rendered *second* silently reused the first's curve. **The table is indexed by the
luminance's float bits** (`stretch::scale_lut_position`: 512 entries an octave from 2^-24 to 1, 12289 entries,
0.2 % apart at every level; within 0.15 of a level of the exact curve at any solve, `stretch::tests`). Spread evenly,
its 8192 entries stepped 2 ADU of a 14-bit sensor: once the gap fell under a step the sky was interpolated from the
curve's limit at zero and sank with depth (Cat's Eye 10.9 -> 6.2 levels by 27 subs, Ring-2 and Globular 64 too),
its grain with it. A table is a `ScaleLut` (`scale_lut_from`), whose fixed length rules out one of another
layout (a short one read silently wrong); `asinh` takes a series under 0.01, as f32's `ln` loses it.

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
INFO events, self-contained, collected on `spawn_blocking` under a 5 s timeout. Its build line carries `APP_VERSION`
(the release's, from `build-dist.sh`), Community's `git` and Pro's `pro_git` (Pro's own `build.rs`, via
`app::PRO_GIT`). PlayerOne logs its SDK version on load and every bus rescan made while a camera is open.

A dropped *capture* frame is signal lost for good, so render/stacking favour stacking: the display copy is gated
by `want_display` but the frame always stacks; the capture→stacking queue is **latency-bounded** at 2s of
exposures. **Except after Stop**: queued frames stack for `STOP_DRAIN` (5 s, from the first Stop, `cancelled_for`),
then are taken off unstacked and the stack saved (`capture::stop_drain`; their raw files and the SER are kept) —
2026-10-24 sat 42 minutes in *Stopping* behind 19 frames. A capture that ends on its own still drains whole. Per-stack estimates (white balance, background) are reused and refreshed only on proportional depth
growth.

**A static sky must render the same frame after frame**: live view re-solves everything every frame, which is what
holds it still; stacked view keeps one consistent snapshot and carries its MAD forward along a noise trend instead
of re-measuring fresh each frame; signal-fraction gates are **bands**, not steps, so a field living in one still
renders its own small jitter rather than a hard curve-flip. `IncrementalPixel` (16B/sample, 434MB at 3008²x3) is
read+written whole every frame — already the memory ceiling.

`--span-timings` logs every stage span's duration (per-frame work belongs at `info_span!`, not `debug_span!`, or
it's invisible); `--features telemetry` adds histograms/counters/queue-depth gauges, and in debug mode the UI
shows the **drop rate** over delivered frames (beside LiveView's FPS and resolution). Build `--profile profiling` for `perf`.
