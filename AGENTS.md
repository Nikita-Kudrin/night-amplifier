## Project Overview

Night Amplifier is an EAA (Electronically Assisted Astronomy) live stacking and auto-stretching engine in Rust.
Processes
astronomical images via calibration, star detection, alignment, stacking, and rendering.

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
- YAGNI (You Aren't Gonna Need It): Do not build features, abstractions, or infrastructure for hypothetical future use
  cases.

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
Proactively update documentation files (\*.md) or README files after changes.
After making big changes, run backend and frontend tests.

## Code architecture

When designing new features or refactoring, adhere to the following architectural principles:

- Clean Architecture / Hexagonal Architecture: Keep the core business logic (domain) isolated from external concerns (
  UI, databases, third-party APIs). Use interfaces/ports to decouple components.
- Domain-Driven Design (DDD): Group code by feature or domain (e.g., user, billing, inventory) rather than technical
  role (e.g., controllers, models, services).
- Separation of Concerns (SoC): Ensure each module, class, and function has a single, well-defined responsibility.
- Asynchronous Communication: Favor event-driven architectures (Pub/Sub, message queues) for long-running processes or
  cross-service communication to reduce tight coupling.
- Design for Failure (Resilience)

# Test Guidelines

If you can't fix the test, don't try to simplify if by removing the idea of the test.
Tests might run a minute or two - you should wait for them to finish. Benches migth run even longer.

## Build & Test Commands

```bash
cargo build          # Debug build
cargo build --release # Release build
cargo test           # Run all unit tests (fast, excludes integration tests)
cargo test <name>    # Run specific test

# Integration tests (slow, use real fixture images from tests/fixtures/)
# These are ignored by default and must be run explicitly:
cargo test --test integration_pipeline -- --ignored --test-threads=1

# Run server (default port 8080)
cargo run --release

# Run server on custom port
cargo run --release -- 3000

# Run with OpenTelemetry tracing enabled (requires telemetry feature)
cargo run --release --features telemetry -- --telemetry

# Build with OpenTelemetry support
cargo build --features telemetry

# Frontend development (run from web/ directory)
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm install && npm run dev

# Frontend production build
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run build

# Frontend linting and formatting (run from web/ directory)
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run lint      # Check for issues
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run lint:fix  # Auto-fix ESLint issues
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run format    # Format with Prettier

# Frontend tests (run from web/ directory)
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm test          # Run tests in watch mode
cd web && (. "$HOME/.nvm/nvm.sh" 2>/dev/null || true) && npm run test:run # Run tests once
```

**Note:** All `npm` commands must be run from the `web/` directory.

**Important:** Always run `cargo test` after making any code changes to ensure nothing is broken. For frontend changes,
also run `cd web && npm run test:run` to verify frontend tests pass.

## Code Architecture

### Core Modules (src/)

- **frame/** - `Frame` struct holding image data as normalized f32 [0.0, 1.0]. Supports RGB8, RGB16 (LE/BE), and
  Bayer8/16 input formats. Row-major, interleaved channels. Expanded into submodules for format handling and factory.

- **debayer/** - `Debayerer` for converting Bayer (CFA) raw data to RGB. Supports all four standard patterns (RGGB,
  BGGR, GRBG, GBRG). Implements bilinear (fast) and VNG (quality, SIMD-optimized) algorithms. Features auto-detection of
  Bayer patterns.

- **calibration/** - `MasterDark` (thermal noise subtraction), `MasterFlat` (vignetting correction), `Calibration`
  pipeline. Math: `calibrated = (raw - dark) / flat`. Includes SIMD-optimized implementations.

- **detection/** - `StarDetector` with `DetectionConfig`. Uses background estimation (median/MAD), local maxima
  detection, hot pixel rejection, and Center of Mass centroiding for sub-pixel accuracy. Includes adaptive thresholding.

- **registration/** - `ImageRegistration` using triangle matching. Generates scale-invariant triangle descriptors,
  matches between frames, votes for star correspondences, computes affine transform (rotation, scale, translation).
  Supports adaptive registration and RANSAC verification.

- **stacking/** - Stacking module for frame accumulation, organized into submodules:
    - `mod.rs` - Module exports and documentation
    - `config.rs` - `StackingConfig`, `WeightingConfig`, `FrameQuality` for stacking configuration
    - `compute.rs` - Pixel computation utilities bridging stack and rejection modules
    - `pixel_stats.rs` - `PixelStats` for per-pixel running statistics (AoS layout for cache efficiency)
    - `rejection.rs` - SIMD-optimized outlier rejection algorithms (SigmaClip, WinsorizedSigmaClip, MinMax) with
      weighted variants
    - `tile_context.rs` - `TileContext` for parallel tile processing during compute phase
    - `warp.rs` - Affine transformation with bilinear interpolation for frame alignment
    - `weighting.rs` - Frame weighting utilities for quality-based stacking (FWHM/SNR normalization)
    - `stack.rs` - `MasterStack` accumulator with frame-major storage and quality-based weighting
    - `stack_tests.rs` - Unit tests for MasterStack
    - `stacker.rs` - `Stacker` combining warping and stacking steps with quality metric support
    - `pipeline.rs` - `StackingPipeline` for high-level streaming frame processing

- **background/** - `BackgroundExtractor` for light pollution removal. Grid-based median estimation with star rejection,
  bilinear interpolation for smooth gradient model. Features adaptive background subtraction with:
    - `gradient_only` mode (default) - subtracts only gradient variation, preserving base signal
    - `reference_percentile` - configurable reference level (default 10th percentile)
    - `aggressiveness` - controls subtraction strength (0.0-1.0, or auto-detect)
    - Presets: `for_nebulae()` (conservative), `for_light_pollution()`, `adaptive()` (auto-detect)

- **render/** - Comprehensive rendering module for final output:
    - **pipeline/** - High-level rendering orchestration
    - **stretch/** - Asinh and MTF non-linear stretch functions, shadow saturation boost
    - **white_balance.rs** - Background neutralization functions
    - **black_point.rs** - Black point calculation and subtraction based on robust statistics
    - **autostretch/** - Automatic stretch factor solver with Newton-Raphson/bisection hybrid
    - **output/** - S-curve contrast, gamma correction, 8-bit RGB output with optional dithering

- **statistics/** - `ImageStats` and `ChannelStats` for computing robust per-channel median and MAD. Used for
  auto-stretch and background neutralization. Sampling-based for performance on large images.

- **camera/** - Generalized camera interface (traits) supporting native SDKs (ZWO, PlayerOne) and a high-fidelity
  simulator. Handles exposure, gain, and configuration logic. Cooled cameras expose `has_cooler`,
  `min_temp_c`, `max_temp_c` on `CameraInfo` and live `temperature_c` / `cooler_power` / `cooler_on`
  on `CameraStatus`. Cooler activation is driven through `CaptureConfig.cooler_enabled` /
  `target_temp_c`, applied each frame inside `Camera::capture()` so live UI edits take effect
  without restarting capture. The simulator models temperature with a first-order lag toward the
  target (tau ≈ 3s) so the cooler UI can be tested without hardware.

  **Camera lifecycle (pre-cool / warm-up):** the camera handle is opened at connect time and held
  long-term in `AppState.active_camera` (see `src/server/camera_session/`). If `cooler_enabled`
  and `target_temp_c` are set in settings, connecting begins pre-cooling immediately; the
  lifecycle phase (`CameraPhase`) transitions `Precooling → Idle` once the sensor settles within
  `PRECOOL_TOLERANCE_C` for two consecutive samples. Disconnect with a live cooler enters
  `WarmingUp`, disables the TEC, and only closes the USB handle once the sensor reaches
  `WARMUP_THRESHOLD_C` (default 10 °C) or `WARMUP_TIMEOUT` (5 min) elapses — this prevents dew
  condensation on a still-cold sensor. Capture start transfers the handle to the capture thread
  (phase → `Capturing`); on exit, the handle returns to the session. Starting capture during
  warmup cancels warmup and re-enables the cooler per current settings. The background monitor
  runs on a dedicated `std::thread` (not a tokio task) so USB stalls can never poison the runtime.

- **plugins/** - Registry and traits for professional features (Push-To, Advanced Rejection, Comet Stacking). Allows the
  Community version to interact with the Pro version when available.

- **error.rs** - `StackError` enum with `thiserror` derive. Includes FFI boundary error handling.

- **ffi_safety.rs** - FFI safety utilities for handling C/C++ library boundaries. Provides `catch_ffi_panic` for
  wrapping calls to camera SDKs and cfitsio. Includes buffer validation, dimension overflow checks, and cleanup guards.

- **logging.rs** - `LogConfig` and `init_logging` for configurable logging. Uses `tracing` ecosystem with
  `tracing-appender` for file rotation (daily rotation). Supports configurable log levels, console/file output,
  development/production presets, and optional OpenTelemetry integration.

- **telemetry.rs** - Optional OpenTelemetry integration for distributed tracing and metrics. Exports traces and metrics
  via OTLP to collectors like Jaeger or Prometheus. Enabled by default in debug builds, disabled in release.
  Configurable
  via CLI flags (`--telemetry`, `--no-telemetry`, `--otlp-endpoint`) or environment variables
  (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME`). Provides a `metrics` submodule for recording collection sizes
  and memory allocation:
    - `master_stack.memory_bytes` - Memory usage of the stacking buffer
    - `master_stack.frame_count` - Number of frames in the stack
    - `master_stack.frame_qualities_count` - Number of frame quality entries
    - `master_stack.pixel_count` - Number of pixels in master stack
    - `disk_writer.queue_depth` - Current disk writer queue depth
    - `disk_writer.queue_capacity` - Maximum queue size
    - `catalog.entries_count` - Number of catalog entries loaded
    - `catalog.index_size` - Size of catalog indices (designation, messier, alias)
    - `server.cameras_count` - Number of connected cameras
    - `server.event_subscribers` - Number of active event subscribers
    - `server.latest_frame_size` - Size of latest rendered frame in bytes

- **planetary/** - Native planetary stacking engine for high-frame-rate imaging (Moon, planets, Sun). Uses
  correlation-based alignment (no stars needed), quality-based frame selection, and percentile stacking. Supports
  various quality metrics (Laplacian, Sobel, Tenengrad) for automatic best-frame selection.

- **ser/** - SER video file format support (Simple Extensible Recording). Standard format for planetary imaging with
  uncompressed frames, per-frame timestamps, and Bayer pattern support. Compatible with AutoStakkert, PIPP, RegiStax.

- **disk_writer/** - Asynchronous disk writer with queue for saving captured frames. Uses bounded channel to queue write
  requests without blocking capture loop. Monitors queue depth and warns about slow disk performance.

  **Image Storage Formats:**

  | Output                  | Format | Bit Depth          |
    |-------------------------|--------|--------------------|
  | Raw frames              | FITS   | 16-bit unsigned    |
  | Stacked image           | FITS   | 32-bit float       |
  | Stacked image (preview) | PNG    | 8-bit              |
  | Planetary frames        | SER    | 16-bit unsigned    |

- **app.rs** - Shared application entry point (`app::run()`) used by both Community and Pro binaries. Handles CLI
  argument parsing, logging setup, and server startup. Pro binary passes a plugin-registration closure.

- **push_to/** - Community-side trait definitions for the Push-To navigation system, split into three sub-traits:
  `PushToSolverPlugin` (plate solving, direction), `PushToCatalogPlugin` (catalog search, targets),
  `PushToInstallerPlugin` (ASTAP/catalog installation). `PushToSystemPlugin` is a compound super-trait.
  Actual implementations reside in the Pro repository.

### Server Module (src/server/)

- **mod.rs** - `Server` and `ServerConfig` for the main web server. Uses Axum with REST API and WebSocket support.
- **state.rs** - `AppState` for shared server state. Thread-safe with `Arc<RwLock<T>>` for settings.
- **api.rs** - REST API handlers for capture control, settings management, and camera operations.
- **ws.rs** - WebSocket handlers for live image streaming (`/ws/stream`) and server events (`/ws/events`).
- **dto/** - Data transfer objects for API request/response serialization, split by domain:
    - `mod.rs` - Core DTOs (ApiResponse, Settings, Camera, Capture)
    - `push_to.rs` - Push-To navigation DTOs (position, direction, catalog)
    - `install.rs` - ASTAP and catalog installation DTOs
- **events.rs** - Server event types for WebSocket broadcasting.
- **capture.rs** - Capture session management and frame processing logic.
- **encoding.rs** - Frame encoding (RGB8+LZ4) for WebSocket streaming.
- **error.rs** - Server-specific error types.
- **util.rs** - Server utility functions.

### Web Frontend (web/)

Vue 3 SPA for remote camera control and live image streaming. Mobile-first responsive design with dark theme.

- **src/main.js** - Vue app initialization
- **src/App.vue** - Root component with responsive layout (sidebar + content)
- **src/assets/main.css** - Dark theme CSS variables and base styles
- **src/composables/** - Reactive composables (api, useWebSocket, useAppState, useError, usePanZoom, useWebGLRenderer,
  useCanvas2DRenderer)
- **src/components/** - Vue components (CameraPanel, CaptureControls, SettingsPanel, PushToPanel, LiveView, StatusBar,
  PushToSetupOverlay, AstapInstallOverlay)
- **src/components/ui/** - Reusable UI components (BasePanel, BaseSlider, BaseToggle, BaseAlert, ButtonGroup)
- **src/utils/** - Utility functions (pixelConversion for RGB8 to RGBA8 conversion)
- **src/constants/** - Shared constants

### Key Types

| Type                     | Purpose                                                                                 |
|--------------------------|-----------------------------------------------------------------------------------------|
| `Frame`                  | Image container with f32 normalized pixels                                              |
| `PixelFormat`            | Input format enum (Rgb8, Rgb16, Rgb16Be, Bayer8, Bayer16, Bayer16Be)                    |
| `CfaPattern`             | Bayer CFA pattern enum (Rggb, Bggr, Grbg, Gbrg)                                         |
| `DebayerConfig`          | Configuration for debayering (pattern + algorithm)                                      |
| `DebayerAlgorithm`       | Debayer algorithm enum (Bilinear, Vng)                                                  |
| `Debayerer`              | Converts single-channel Bayer data to RGB                                               |
| `PatternDetectionResult` | Auto-detected CFA pattern with confidence score                                         |
| `Star`                   | Detected star with sub-pixel position, flux, SNR                                        |
| `BackgroundStats`        | Background estimation result (median, MAD, threshold)                                   |
| `AffineTransform`        | 2D transform (rotation, scale, translation)                                             |
| `RegistrationConfig`     | Configuration for image registration                                                    |
| `Triangle`               | Triangle descriptor for star pattern matching                                           |
| `TriangleMatcher`        | Matches triangle patterns between star lists                                            |
| `Calibration`            | Combines MasterDark and MasterFlat                                                      |
| `Stacker`                | High-level stacking engine with warping and quality weighting                           |
| `MasterStack`            | Per-pixel accumulator with rejection and weighted averaging                             |
| `StackingConfig`         | Configuration for stacking (rejection method, sigma, weighting)                         |
| `WeightingConfig`        | Quality-based frame weighting configuration (FWHM/SNR weights, power, min_weight)       |
| `FrameQuality`           | Quality metrics for a frame (FWHM, SNR) used for weighted stacking                      |
| `RejectionMethod`        | Enum: None, SigmaClip (Pro), WinsorizedSigmaClip (Pro), MinMax (Pro)                    |
| `BackgroundConfig`       | Configuration for background extraction (grid size, star rejection, gradient_only mode) |
| `BackgroundExtractor`    | Extracts smooth background gradient                                                     |
| `BackgroundModel`        | 2D interpolated background for subtraction                                              |
| `RenderConfig`           | Configuration for stretch, gamma, white balance                                         |
| `Renderer`               | Final rendering pipeline with stretch/gamma                                             |
| `BlackPointConfig`       | Configuration for black point calculation                                               |
| `AutoStretchConfig`      | Configuration for autostretch solver (target, sigma, limits)                            |
| `AutoStretchType`        | Enum: Fixed (constant target), Adaptive (image content-based)                           |
| `AutoStretchResult`      | Solver output (stretch factor, black point, convergence)                                |
| `ImageStats`             | Per-channel statistics (median, MAD, sigma)                                             |
| `ChannelStats`           | Single-channel median/MAD with black point helpers                                      |
| `StatsConfig`            | Configuration for statistics sampling                                                   |
| `ContrastConfig`         | S-curve contrast adjustment parameters                                                  |
| `OutputConfig`           | Final output conversion settings (contrast, gamma, dither)                              |
| `SaturationBoostConfig`  | Shadow saturation boost settings (enabled, strength, shadow_peak, upper_limit)          |
| `Server`                 | Web server for remote control and image streaming                                       |
| `ServerConfig`           | Server configuration (bind addr, CORS, static dir, stream settings)                     |
| `AppState`               | Shared server state with thread-safe access                                             |
| `CaptureState`           | Enum: Idle, Starting, Capturing, Stopping, Error                                        |
| `StackingType`           | Enum: DeepSky, Planetary - with capability methods (see Adding a Stacking Type)         |
| `StackingTypeInfo`       | API response with stacking type metadata and capabilities                               |
| `CaptureSettings`        | Capture configuration (exposure, gain, stacking options)                                |
| `CaptureSession`         | Active session info (frame counts, timestamps)                                          |
| `ServerEvent`            | WebSocket event types (StateChanged, FrameReady, Error, etc.)                           |
| `ApiResponse<T>`         | JSON response wrapper with success/error handling                                       |
| `LogConfig`              | Logging configuration (level, directory, output options, telemetry)                     |
| `LogGuard`               | Guard that keeps logging worker thread alive                                            |
| `LoggingError`           | Error type for logging initialization failures                                          |
| `TelemetryConfig`        | OpenTelemetry configuration (OTLP endpoint, service name, enabled flag)                 |
| `TelemetryGuard`         | Guard that shuts down OpenTelemetry on drop                                             |
| `TelemetryError`         | Error type for telemetry initialization failures                                        |
| `FitsMetadata`           | Metadata for FITS headers (exposure, gain, camera, timestamps)                          |
| `DiskWriter`             | Background task for async frame writing                                                 |
| `DiskWriterHandle`       | Handle for queuing frames to disk writer                                                |
| `DiskWriterConfig`       | Configuration for disk writer (base dir, queue size, enabled)                           |
| `WriteRequest`           | Request to write a frame (raw or stacked)                                               |
| `FrameType`              | Enum: Raw (individual frames), Stacked (final result)                                   |
| `DiskWriterError`        | Error type for disk writing failures                                                    |
| `SettingsPersistence`    | Saves/loads capture settings to/from JSON file                                          |
| `PersistedSettings`      | Serializable settings structure for JSON persistence                                    |
| `PlanetaryStacker`       | Planetary stacking engine with quality selection                                        |
| `PlanetaryConfig`        | Planetary stacking settings (selection %, method, quality metric)                       |
| `PlanetaryStackMethod`   | Enum: Mean, Median, Percentile, WeightedMean                                            |
| `QualityMetric`          | Enum: Laplacian, Sobel, Tenengrad, StdDev                                               |
| `ScoredFrame`            | Frame with quality score and alignment offset                                           |
| `AlignmentRoi`           | Region of interest for correlation-based alignment                                      |
| `PlanetaryStackStats`    | Statistics about planetary stack (counts, quality range)                                |
| `SerReader`              | SER video file reader                                                                   |
| `SerWriter`              | SER video file writer                                                                   |
| `SerHeader`              | SER file header (dimensions, color, timestamps)                                         |
| `SerColorId`             | Enum: Mono, BayerRGGB/GRBG/GBRG/BGGR, RGB, BGR                                          |
| `InstallProgress`        | Enum: Starting, Downloading, Extracting, Completed, Failed                              |

### Plugin Systems

Certain advanced features are implemented via a plugin system to allow the Community version to remain functional while
delegating performance-critical or professional logic to the Pro repository.

- **REJECTION_PLUGIN** - Handles `SigmaClip`, `WinsorizedSigmaClip`, and `MinMax` rejection methods.
- **PUSH_TO_PLUGIN** - Compound plugin (`PushToSystemPlugin`) combining three sub-traits:
    - `PushToSolverPlugin` - Plate solving and direction calculation
    - `PushToCatalogPlugin` - Celestial catalog search and target management
    - `PushToInstallerPlugin` - ASTAP binary and catalog installation
- **COMET_PLUGIN** - Handles specialized comet detection and tracking logic.
- **BACKGROUND_PLUGIN** - Extends background extraction with advanced models like RBF.

| `FfiError`               | FFI boundary error (Panic, NullPointer, Timeout, BufferOverflow)                        |
| `FfiResult<T>`           | Result type for FFI operations |
| `FfiCleanupGuard`        | RAII guard for FFI resource cleanup on error paths |

### Design Principles

1. **f32 normalization** - All pixel math uses [0.0, 1.0] range to prevent overflow
2. **Parallel processing** - Rayon for multi-core (calibration, detection)
3. **No allocations in hot paths** - Pre-allocated buffers where possible
4. **ARM friendly** - Optimized for Raspberry Pi 5
5. **FFI safety** - All C/C++ library calls wrapped with `catch_ffi_panic` to handle panics from vendor SDKs

### Adding a New Stacking Type

The `StackingType` enum (`src/stacking/config.rs`) uses capability methods for extensibility.
Adding a new stacking type only requires changes to the enum itself - no changes needed in `capture.rs`.

**Steps to add a new stacking type:**

1. Add the new variant to `StackingType` enum:

   ```rust
   pub enum StackingType {
       DeepSky,
       Planetary,
       YourNewType,  // Add here
   }
   ```

2. Update `StackingType::all()` to include your new type

3. Implement the capability methods for your new type:
    - `display_name()` - Human-readable name
    - `description()` - When to use this type
    - `uses_star_registration()` - Does it use star-based frame registration?
    - `supports_stacking()` - Can frames be added incrementally?
    - `supports_quality_weighting()` - Does it support FWHM/SNR weighting?
    - `uses_aggressive_stretch()` - Does it need aggressive auto-stretch?

4. The capture loop automatically uses these capability methods - no changes needed there

**Example capability methods for a hypothetical "Lucky Imaging" type:**

```rust
StackingType::LuckyImaging => false,  // uses_star_registration: uses correlation
StackingType::LuckyImaging => false,  // supports_stacking: batch only
StackingType::LuckyImaging => false,  // supports_quality_weighting: uses selection
StackingType::LuckyImaging => false,  // uses_aggressive_stretch: bright targets
```

### Logging

Environment variable `RUST_LOG` can override log levels (e.g., `RUST_LOG=debug`).

### Settings Persistence

Capture settings are automatically saved to a JSON file (`settings.json`) in the server's working directory.
Settings are loaded on server startup and saved whenever they are updated via the `/api/settings` endpoint.

### Disk Writer (Async Frame Storage)

**Directory Structure:**

```
captures/
├── raw/
│   └── DD-MM-YYYY_HH-MM-SS/
│       ├── frame_000001.fits
│       ├── frame_000002.fits
│       └── ...
└── stacked/
    └── DD-MM-YYYY_HH-MM-SS.fits
```

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

### Server API Endpoints

| Endpoint                       | Method | Description                   |
|--------------------------------|--------|-------------------------------|
| `/api/capture/start`           | POST   | Start capture session         |
| `/api/capture/stop`            | POST   | Stop capture session          |
| `/api/capture/status`          | GET    | Get capture status            |
| `/api/settings`                | GET    | Get current settings          |
| `/api/settings`                | POST   | Update settings               |
| `/api/cameras`                 | GET    | List available cameras        |
| `/api/cameras/{id}`            | GET    | Get camera info               |
| `/api/cameras/{id}/connect`    | POST   | Connect to camera             |
| `/api/cameras/{id}/disconnect` | POST   | Disconnect from camera        |
| `/ws/stream`                   | WS     | Live image stream (binary)    |
| `/ws/events`                   | WS     | Server events (JSON)          |
| `/api/push-to/status`          | GET    | Get Push-To status            |
| `/api/push-to/target`          | POST   | Set target (name or coords)   |
| `/api/push-to/target`          | DELETE | Clear current target          |
| `/api/push-to/direction`       | GET    | Get push direction            |
| `/api/push-to/catalog/search`  | GET    | Search object catalog         |
| `/api/push-to/catalog/messier` | GET    | Get all Messier objects       |
| `/api/push-to/catalog/ngc`     | GET    | Get NGC objects               |
| `/api/push-to/catalog/ic`      | GET    | Get IC objects                |
| `/api/push-to/config`          | POST   | Update Push-To config         |
| `/api/astap/status`            | GET    | Get ASTAP installation status |
| `/api/astap/databases`         | GET    | Get available database types  |
| `/api/astap/install`           | POST   | Start ASTAP installation      |
| `/api/catalog/status`          | GET    | Get OpenNGC catalog status    |
| `/api/catalog/install`         | POST   | Start OpenNGC catalog install |

### Settings API

The `/api/settings` endpoint accepts these fields:

| Field                       | Type | Default | Description                             |
|-----------------------------|------|---------|-----------------------------------------|
| `exposure_us`               | u64  | 1000000 | Exposure time in microseconds           |
| `gain`                      | i32  | 0       | Gain value                              |
| `offset`                    | i32  | 10      | Offset (black level)                    |
| `bin`                       | u8   | 1       | Binning factor                          |
| `auto_stretch`              | bool | true    | Enable auto-stretch for preview         |
| `autostretch_type`          | str  | "fixed" | Autostretch type: "fixed" or "adaptive" |
| `target_background`         | f32  | 0.15    | Target background brightness (0.0-1.0)  |
| `stacking`                  | bool | true    | Enable stacking                         |
| `max_stack_frames`          | u32  | 0       | Max frames to stack (0 = unlimited)     |
| `rejection_sigma`           | f32  | 2.5     | Sigma for outlier rejection             |
| `background_subtraction`    | bool | true    | Enable background subtraction           |
| `save_raw_frames`           | bool | false   | Enable saving raw frames to disk (FITS) |
| `save_stacked_image`        | bool | false   | Enable saving stacked image (FITS+PNG)  |
| `saturation_boost`          | bool | false   | Enable shadow saturation boost          |
| `saturation_boost_strength` | f32  | 0.5     | Saturation boost intensity (0.0-1.0)    |
| `memory_limit_mb`           | u32  | 4000    | Memory limit for stacking (MB)          |
| `simulated_preload_images`  | u32  | 5       | Number of images to preload for sim cam |
| `cooler_enabled`            | bool | false   | Activate camera TEC during capture      |
| `target_temp_c`             | f64? | null    | Target sensor temperature in Celsius    |

### WebSocket Events

Events sent via `/ws/events`:

| Event Type                    | Fields                                                                                | Description                        |
|-------------------------------|---------------------------------------------------------------------------------------|------------------------------------|
| `state_changed`               | `state`                                                                               | Capture state changed              |
| `frame_captured`              | `frame_number`, `stacked_count`                                                       | New frame captured                 |
| `frame_rejected`              | `frame_number`, `reason`                                                              | Frame was rejected                 |
| `settings_updated`            | -                                                                                     | Settings were changed              |
| `camera_connected`            | `name`                                                                                | Camera connected                   |
| `camera_disconnected`         | `name`                                                                                | Camera disconnected                |
| `camera_status_updated`       | `name`, `temperature_c`, `cooler_power`, `cooler_on`, `target_temp_c`                 | Cooled camera status sample (~2s)  |
| `camera_phase_changed`        | `name`, `phase` (`idle`/`precooling`/`capturing`/`warming_up`/`disconnected`)         | Camera lifecycle phase transition  |
| `error`                       | `message`                                                                             | Error occurred                     |
| `disk_writer_warning`         | `queue_depth`                                                                         | Disk write queue exceeds threshold |
| `disk_writer_warning_cleared` | -                                                                                     | Queue warning cleared              |
| `plate_solving_started`       | `target_name`                                                                         | Plate solving started              |
| `position_solved`             | `ra_degrees`, `dec_degrees`                                                           | Plate solve succeeded              |
| `position_solve_failed`       | `reason`                                                                              | Plate solve failed                 |
| `push_direction_updated`      | `angle`, `distance`, `hints`                                                          | Push direction recalculated        |
| `target_changed`              | `name`, `ra`, `dec`                                                                   | Target was set                     |
| `target_cleared`              | -                                                                                     | Target was cleared                 |
| `astap_install_starting`      | `component`                                                                           | ASTAP installation starting        |
| `astap_install_progress`      | `component`, `bytes_downloaded`, `total_bytes`, `percent`, `stage`, `overall_percent` | Download progress update           |
| `astap_install_extracting`    | `component`, `progress`, `stage`, `overall_percent`                                   | Extraction progress update         |
| `astap_install_completed`     | `component`, `stage`, `overall_percent`                                               | Component installation completed   |
| `astap_install_failed`        | `component`, `error`                                                                  | Installation failed                |
| `catalog_install_starting`    | -                                                                                     | Catalog installation starting      |
| `catalog_install_progress`    | `file_name`, `bytes_downloaded`, `total_bytes`, `percent`                             | Catalog download progress          |
| `catalog_file_completed`      | `file_name`                                                                           | Catalog file downloaded            |
| `catalog_install_completed`   | `object_count`                                                                        | Catalog installation completed     |
| `catalog_install_failed`      | `error`                                                                               | Catalog installation failed        |

### RGB8+LZ4 Image Streaming

The `/ws/stream` WebSocket endpoint streams frames in a high-performance RGB8+LZ4 format. This provides 8-bit per
channel precision with extremely fast LZ4 compression, significantly reducing memory footprint and encoding/decoding
overhead compared to 16-bit formats, while maintaining excellent visual quality for typical displays.

#### Binary Protocol Format

```
+--------+--------+--------+----------------+------------------+
| Magic  | Width  | Height | Compressed Size| LZ4 Compressed   |
| (4B)   | (4B)   | (4B)   | (4B)           | RGB8 Data        |
+--------+--------+--------+----------------+------------------+
  0x53413038  u32 LE  u32 LE    u32 LE         variable length
  "SA08"
```

**Header (16 bytes):**

- Bytes 0-3: Magic number `0x53413038` ("SA08" ASCII, little-endian)
- Bytes 4-7: Image width (u32, little-endian)
- Bytes 8-11: Image height (u32, little-endian)
- Bytes 12-15: Compressed data size (u32, little-endian)

**Payload:**

- LZ4-compressed RGB8 pixel data (includes size prefix from `compress_prepend_size`)
- Decompressed format: 3 bytes per pixel (R, G, B)
- Total decompressed size: width × height × 3 bytes

#### WebGL & Canvas Rendering

The frontend typically uses WebGL to render frames to a canvas, with a fallback to Canvas2D for broader compatibility.
8-bit stream processing removes the need for client-side pixel format conversions, improving rendering latency and
extending battery life on mobile devices.

#### Performance Characteristics

| Metric              | Value (1920×1080) | Notes                           |
|---------------------|-------------------|---------------------------------|
| Raw RGB8 size       | 6.2 MB            | width × height × 3 bytes        |
| Typical compressed  | 1-3 MB            | Depends on image content        |
| Compression ratio   | 2-5×              | Dark astro images compress well |
| Encode time (Rust)  | ~3ms              | LZ4 is extremely fast           |
| Decode time (JS)    | ~5ms              | Direct to ImageData compatible  |
| Max sustainable FPS | 60+               | Excellent network utilization   |

### Web Frontend Development

```bash
cd web
npm install       # Install dependencies
npm run dev       # Dev server on :5173 (proxies to backend :8080)
npm run build     # Production build to web/dist/
npm test          # Run tests in watch mode
npm run test:run  # Run tests once
```

The Vite config (`vite.config.js`) proxies `/api` and `/ws` to `localhost:8080` during development.
For production, run `npm run build` and the Rust server serves static files from `web/`.

## Testing

### Backend (Rust)

Run with `cargo test`. Key test patterns:

- Synthetic star generation with Gaussian profiles
- Hot pixel rejection verification
- Calibration math validation
- Transform roundtrip tests
- Warping with identity/translation/rotation transforms
- Sigma clipping and outlier rejection validation
- Background gradient detection and removal
- Asinh stretch normalization and color preservation
- Robust statistics with outlier resistance (median/MAD)
- White balance neutralization for colored backgrounds
- Autostretch solver convergence and accuracy validation
- Bayer pattern recognition and debayering (bilinear, VNG)
- Server endpoint testing with Tower's `ServiceExt::oneshot`
- WebSocket event serialization and broadcasting
- Concurrent state access and thread safety
- FITS file writing (mono, RGB, metadata headers)
- Disk writer queue management and warning thresholds
- Session directory creation and naming format
- Save frames setting toggle via API
- Push-To catalog search and filtering
- Celestial coordinate parsing and conversion
- Spherical geometry direction calculations
- Plate solver WCS file parsing and ASTAP integration

### Frontend (JavaScript/Vue)

Run with `cd web && npm test`. Uses Vitest + Vue Test Utils + happy-dom.

Test files are co-located with source files (`*.test.js`) or in `__tests__/` directories:

- `src/composables/*.test.js` - Composable tests (api, useWebSocket, usePanZoom)
- `src/components/*.test.js` - Component tests (StatusBar, CaptureControls, CameraPanel, SettingsPanel)
- `src/components/__tests__/LiveView/` - LiveView tests split by feature (canvas, webgl, mouse, touch, rgb16, etc.)
- `src/utils/*.test.js` - Utility tests (pixelConversion)

Key test patterns:

- Mock `fetch` for API tests
- Mock `WebSocket` class for WebSocket tests
- Use `vi.mock()` to mock composables in component tests
- Provide mock inject values via `global.provide`
- Use `flushPromises()` for async operations
- Use `vi.useFakeTimers()` for debounced operations

## Full Image Processing Pipeline

The image processing engine implements a comprehensive, multi-phase linear and non-linear mathematical pipeline designed
to extract the maximum possible signal from noisy astronomical data:

### Phase 1: Sensor Data Acquisition & Calibration

Implemented in `calibration.rs` / `frame.rs`. Corrects for sensor imperfections:

- **Master Dark Subtraction**: Removes thermal noise and amp glow by subtracting a stacked reference dark frame.
- **Master Flat Division**: Corrects for vignetting, dust motes, and uneven sensor illumination:
  `calibrated = (raw - dark) / flat`.
- Applies math purely in 32-bit floating-point precision.

### Phase 2: Debayering (Demosaicing)

Implemented in `debayer.rs`. Converts mono Bayer pattern (CFA) data into full RGB color:

- Auto-detects patterns (RGGB, BGGR, GRBG, GBRG).
- Bilinear Algorithm: Fast interpolation for live preview or less critical data.
- VNG (Variable Number of Gradients): High-quality interpolation avoiding color artifacts on edge transitions.

### Phase 3: Star Detection & Centroiding

Implemented in `detection.rs`. Isolates and locates reference stars in the frame:

- Estimates local background statistics using Median and MAD.
- Thresholds image to find local maxima while rejecting isolated hot pixels.
- Calculates sub-pixel precision coordinates using a Center of Mass (CoM) algorithm within a search window.
- Calculates quality metrics: FWHM (sharpness) and SNR.

### Phase 4: Image Registration (Alignment)

Implemented in `registration.rs` and `planetary.rs`. Computes frame-to-frame shifts to counteract tracking errors and
target movement. Supports multiple alignment strategies based on the celestial target:

- **Deep Sky (Stars)**: Adaptive registration generates scale/rotation-invariant triangle patterns, matches them using
  RANSAC, and computes an `AffineTransform`.
- **Planetary (Correlation)**: Uses surface feature cross-correlation within an ROI to align high-framerate
  planetary/lunar frames where stars are absent.
- **Comet (Centroid)**: [Pro] Employs a specific `CometDetector` using an ROI around the comet's nucleus to compute the
  center of mass centroid for alignment, enabling the stack to track the moving comet while stars trail.

### Phase 5: Live Stacking & Rejection

Implemented in `stacking/`. Accumulates aligned frames to dramatically improve Signal-to-Noise Ratio (SNR):

- **Deep Sky (MasterStack)**: Warps frames via Bilinear Interpolation using the `AffineTransform`. Frames are weighted
  based on their FWHM/SNR relative to the reference frame. Outliers (satellite trails, cosmic rays) are rejected using
  specialized algorithms in the Pro version (e.g., `SigmaClip`, `WinsorizedSigmaClip`) via the `REJECTION_PLUGIN`.
- **Planetary Stacking (Lucky Imaging)**: Employs percentile stacking (e.g., top 10%-30% of frames) based on
  high-frequency sharpness metrics like Laplacian, Sobel, or Tenengrad.
- **Comet Stacking**: [Pro] Bypasses traditional weighting and uses highly aggressive `WinsorizedSigmaClip` to
  ruthlessly reject the trailing star field, cleanly isolating the comet signal.

### Phase 6: Background Extraction (Light Pollution Removal)

Implemented in `background.rs`. Removes uneven illumination gradients common in urban skies:

- Uses a grid-based mapping approach to estimate the local background across the entire image.
- Rejects stars from the grid samples to prevent localized dimming.
- Computes a smooth 2D background model via bilinear interpolation.
- Performs subtraction while preserving valid signal, utilizing modes like `gradient_only` or `adaptive` aggressiveness.

### Phase 7: Image Statistics (The Foundation)

Implemented in `statistics.rs`. Computes robust per-channel statistics:

- **Median**: Robust center estimate (unaffected by bright stars)
- **MAD** (Median Absolute Deviation): Robust noise estimate
- **Sigma**: MAD scaled to Gaussian equivalent (σ = 1.4826 × MAD)
- Uses sampling for performance (default: 100K pixels)
- Parallel per-channel computation

### Phase 8: Auto-Color / Background Neutralization

Implemented in `render.rs` (`compute_white_balance` method). Neutralizes color casts from light pollution:

- Calculates per-channel medians using grid-based sampling
- Computes scaling multipliers to align RGB medians to average
- Uses green channel as reference (most stable in astronomy)
- Formula: `multiplier[c] = reference_median / channel_median[c]`
- Result: Sky background becomes neutral gray

### Phase 9: Black Point Calculation

Implemented in `render.rs` (`calculate_black_point` function). Establishes the dark reference level:

- Formula: `BlackPoint = Median - (c × Sigma)`
- Where `c` is the sigma factor (1.5 conservative, 2.0 default, 2.5 aggressive)
- Places black point just below the noise floor
- Clips darkest ~2-5% of background noise while preserving signal
- `BlackPointConfig` provides presets: `conservative()`, `default()`, `aggressive()`

### Phase 10: Shadow Saturation Boost (Optional)

Implemented in `render/stretch.rs`. Selectively enhances color saturation in faint signal regions:

- Applies a bell-shaped multiplier curve that peaks at a specific shadow luminance (`shadow_peak`).
- Bypasses the pure black noise floor to prevent amplifying chromatic noise.
- Fades out before the midtones (`upper_limit`) to preserve natural star and core colors.
- Extremely effective for emission nebulae (Ha/OIII) that lose perceived color saturation during stretching.

### Phase 11: Core Tone Mapping (The Stretch)

Implemented in `render.rs` (`asinh_stretch_frame` function). Color-preserving non-linear stretch:

- Computes luminance: `L = 0.2126×R + 0.7152×G + 0.0722×B`
- Stretches luminance: `L' = asinh(L × stretch) / asinh(stretch)`
- Computes scale: `s = L' / L`
- Applies to all channels: `R' = R×s, G' = G×s, B' = B×s`
- Preserves RGB ratios to maintain natural star colors
- **Asinh (Inverse Hyperbolic Sine)**: The default for astrophotography. Linear near zero (preserves faint detail) and
  logarithmic for large values (compresses bright stars).
- **MTF (Midtones Transfer Function)**: A rational histogram transformation formula used in PixInsight / Siril. Uses a
  midtone balance parameter `m` (where `m < 0.5` mathematically boosts shadows).

### Phase 12: Autostretch Heuristic Solver

Implemented in `render/stretch.rs`. Automatically calculates the optimal parameter (`stretch_factor` for Asinh, or
`midtone` for MTF) that maps the background median to a target brightness, making the algorithm truly "automatic"
regardless of telescope F-ratio or exposure time.

#### The Math

- **Asinh**: We solve for `stretch_factor` such that when `input = adjusted_median`, `output = target_background` (
  default 0.15). Uses a hybrid Newton-Raphson/Bisection solver.
- **MTF**: Solves algebraically for `m` based on the target background.

#### Key Types

| Type                | Purpose                                                               |
|---------------------|-----------------------------------------------------------------------|
| `AutoStretchConfig` | Configuration for target brightness, black point sigma, solver limits |
| `AutoStretchResult` | Computed parameters, black point, median values, convergence info     |

#### Configuration Presets

- `AutoStretchConfig::default()` - Target 0.15, sigma 2.0 (balanced)
- `AutoStretchConfig::dark_sky()` - Target 0.10, sigma 2.5 (dramatic contrast)
- `AutoStretchConfig::preserve_faint()` - Target 0.20, sigma 1.5 (more shadow detail)
- `AutoStretchConfig::light_polluted()` - Target 0.12, sigma 2.8 (aggressive noise clip)

#### Pipeline Steps

1. Compute image statistics (median, MAD, sigma per channel)
2. Calculate black point: `BP = Median - (c × Sigma)`
3. Solve for tone mapping parameter linking `adjusted_median → target_background`
4. Subtract black point from frame
5. Apply chosen Tone Mapping algorithm with the computed parameter

### Phase 13: Final Output Mapping & Contrast

Implemented in `render/output.rs`. Final aesthetic adjustments and conversion for display:

#### S-Curve Contrast (`ContrastConfig`)

Luminance-preserving contrast adjustment using a parametric S-curve:

```text
f(x) = x + strength × (x - midpoint) × (1 - x) × x × 4
```

- **Strength** (0.0-1.0): Amount of contrast boost
- **Midpoint** (0.1-0.9): Where the curve inflection occurs
- Presets: `ContrastConfig::subtle()`, `moderate()`, `strong()`

#### Output Configuration (`OutputConfig`)

Controls the final f32 → u8 conversion:

- **Contrast**: Optional S-curve adjustment
- **Gamma**: Final gamma correction (default 1.0)
- **Dither**: Ordered (Bayer) dithering to reduce banding

#### Key Functions

| Function                                       | Purpose                                    |
|------------------------------------------------|--------------------------------------------|
| `apply_s_curve(value, config)`                 | Apply S-curve to single value              |
| `apply_contrast_frame(frame, config)`          | Apply contrast to entire frame (in-place)  |
| `frame_to_rgb8(frame, config)`                 | Full conversion with contrast/gamma/dither |
| `frame_to_rgb8_simple(frame)`                  | Fast path: no adjustments                  |
| `frame_to_rgb8_with_contrast(frame)`           | With moderate contrast boost               |
| `finalize_for_display(frame, contrast, gamma)` | Complete pipeline convenience function     |

#### Performance

- Parallel processing with Rayon
- Pre-computed gamma LUT for fast lookup
- Typically <15ms for 1920×1080, <30ms for 4K
