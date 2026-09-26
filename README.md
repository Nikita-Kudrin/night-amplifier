# Night Amplifier

Live stacking Web application for Electronically Assisted Astronomy - https://skycontrast.com/software/night-amplifier

|                                Mobile View                                |                             Desktop View                             |
|:-------------------------------------------------------------------------:|:--------------------------------------------------------------------:|
| <img src="manual/mobile_device_view.png" height="450" alt="Mobile View"/> | <img src="manual/desktop_view.png" height="450" alt="Desktop View"/> |

## Supported Platforms

| OS                        | Architecture | Supported                                                 |
|---------------------------|--------------|-----------------------------------------------------------|
| Linux                     | x86_64       | ✅                                                        |
| Linux                     | ARM64        | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| Raspberry Pi5, Orange Pi5 | ARM64        | ✅                                                        |
| Windows                   | x86_64       | ✅                                                        |
| Windows                   | ARM64        | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| macOS                     | x86_64       | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| macOS                     | ARM64        | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |

[![CI](https://github.com/Nikita-Kudrin/night-amplifier/actions/workflows/ci.yml/badge.svg)](https://github.com/Nikita-Kudrin/night-amplifier/actions/workflows/ci.yml)

## Camera SDK Support

Player One, ZWO, ToupTek, QHYCCD, SVBony and INDI cameras, plus a simulator. Camera SDKs are optional and loaded at
runtime; install the ones you need as described in [System dependencies](system-dependencies.md), which also lists what
AI denoising needs to use a GPU or an NPU.

## Features

- **Stacking modes** - Deep Sky, Planetary (![Testing](https://img.shields.io/badge/🚀_Testing-green)).
- **Background subtraction** - grid-based model removing light-pollution gradients.
- **Auto stretching** - colour-preserving stretch with automatic background neutralization.
- **Cooled camera control** - target-temperature setpoint, pre-cooling/warm-up.
- **Guide camera** - a second camera on a guide scope free-runs, plate-solves from its own optics and can be previewed
  instead of the main image. Start/Stop act on the selected camera, so each runs independently.
- **Eyepiece view** - `/eyepiece` is monocular, with fullscreen, pinch zoom, auto-hiding controls and PNG download
  (round or uncropped); `/eyepiece_quality` streams losslessly and follows the binocular/monocular setting.
- **Sensor corrections** - hot-pixel rejection and row/column pattern removal on the raw mosaic, before demosaic.
- **Noise reduction** - guided-filter colour smoothing and scale-selective grain removal at your viewing resolution.

> [!NOTE]
> Pro features, in 'Night Amplifier Pro':
> - **Push-To Navigation** (via ASTAP) - real-time directions for centering objects by hand ![Testing](https://img.shields.io/badge/🚀_Testing-green)
> - **Comet Stacking** - aligns frames on the comet's nucleus ![Testing](https://img.shields.io/badge/🚀_Testing-green)
> - **Advanced Outlier Rejection** - removes satellites, planes and hot pixels (Sigma clipping, Winsorized, Min-Max)
> - **Advanced Background Extraction** - Radial Basis Function background model
> - **AI Denoising** - a neural network on the fastest NPU or GPU found by a one-time hardware benchmark

## Frame Storage

Raw frames are saved as FITS for the modes enabled under **Settings → Storage → Save Raw Frames** (Live view, Wanderer,
Stacking, Guide camera; each independent, all off by default). The finished stack is saved in Stacking mode only. The
guide camera writes to its own folder while its loop runs — select it and press Stop to end both.

**Image Storage Formats:**

| Output                  | Format | Bit Depth       |
|-------------------------|--------|-----------------|
| Raw frames              | FITS   | 16-bit unsigned |
| Stacked image           | FITS   | 32-bit float    |
| Stacked image (preview) | PNG    | 8-bit           |
| Planetary frames        | SER    | 16-bit unsigned |

**Directory Structure:**

One directory per session, named by start time and mode:

```
captures/
├── raw/
│   ├── DD-MM-YYYY_HH-MM-SS-live/        # Live view session
│   ├── DD-MM-YYYY_HH-MM-SS-wanderer/    # Wanderer session
│   ├── DD-MM-YYYY_HH-MM-SS-guide/       # Guide camera, runs alongside the others
│   └── DD-MM-YYYY_HH-MM-SS-stacking/    # Stacking session
│       ├── frame_000001.fits            # Individual raw frames (Deep Sky, Comet)
│       ├── frame_000002.fits            # Planetary writes one capture.ser instead
│       └── ...
└── stacked/
    └── DD-MM-YYYY_HH-MM-SS-stacking.fits # Final stacked result (same name as session)
```

Switching mode mid-capture opens a new directory; sessions starting in the same second get a counter before the suffix
(`DD-MM-YYYY_HH-MM-SS_2-stacking`).

**FITS Metadata:** Each file includes standard FITS headers:

- `EXPTIME` / `EXPOSURE` - Exposure time in seconds
- `GAIN` - Camera gain setting
- `OFFSET` - Camera offset (black level)
- `INSTRUME` - Camera name
- `DATE-OBS` - ISO 8601 timestamp
- `FRAMENUM` - Frame number in sequence
- `NCOMBINE` - Number of stacked frames (for stacked results)
- `XBINNING` / `YBINNING` - Binning factor
- `CCD-TEMP` - Sensor temperature in Celsius (cooled cameras only)
- `SET-TEMP` - Target sensor temperature in Celsius (cooled cameras only)
- `SOFTWARE` - "Night Amplifier"

**Slow Disk Warning:** if disk I/O can't keep up, the web UI warns once the write queue exceeds 5 frames and shows its
depth.

## Distribution Builds

A single self-contained binary with the web UI embedded — no external files needed.

### Pre-built Downloads

[Releases](https://github.com/Nikita-Kudrin/night-amplifier/releases) has Linux (x86_64, ARM64) and Windows (x86_64,
ARM64) binaries, including optimized Raspberry Pi 5 / Orange Pi 5 builds. On desktop Linux, the `.AppImage` is easiest
(make it executable, double-click). Otherwise extract the `.tar.gz` for your platform and run:

```bash
tar xzf night-amplifier-*.tar.gz
cd night-amplifier-*/
./night-amplifier          # Default port 9955 (or 8844 for distribution builds)
./night-amplifier 3000     # Custom port
```

### Building Locally

Local builds are optimized for your specific CPU architecture (`-C target-cpu=native`), which is critical for maximum
performance on devices like Raspberry Pi 5 or Orange Pi 5.

```bash
./scripts/build-dist.sh
```

The archive is created in `dist/`. Extract and run `./night-amplifier`.

### Cross-compile for ARM64

```bash
# Requires: cargo install cross
./scripts/build-dist.sh --cross --target aarch64-unknown-linux-gnu --target-cpu cortex-a76
```

### Build Options

| Flag                 | Description                          |
|----------------------|--------------------------------------|
| `--target <triple>`  | Rust target triple (default: host)   |
| `--target-cpu <cpu>` | CPU optimization (default: `native`) |
| `--cross`            | Use `cross` for cross-compilation    |
| `--no-frontend`      | Skip web frontend build              |
| `--appimage`         | Also create an AppImage              |

## Building from Source

**System prerequisites:**

- **nasm** — required by `turbojpeg-sys` (libjpeg-turbo SIMD acceleration)

```bash
sudo apt-get install nasm   # Debian/Ubuntu/Raspberry Pi OS
sudo dnf install nasm       # Fedora
brew install nasm           # macOS
choco install nasm          # Windows

cargo build --release       # or `cargo build` for a debug build
```

## Running Tests

```bash
cargo test
```

Camera SDK installation and udev rules: see [System dependencies](system-dependencies.md#camera-sdks).

### Web Server

Remote camera control and live image streaming.

```bash
# Run the server (default port 9955)
cargo run --release

# Run on a custom port
cargo run --release -- 3000

# Run with OpenTelemetry tracing enabled
cargo run --release --features telemetry -- --telemetry
```

The server provides:

| Endpoint                       | Method | Description                                                                          |
|--------------------------------|--------|--------------------------------------------------------------------------------------|
| `/api/cameras`                 | GET    | List available cameras                                                               |
| `/api/cameras/{id}/connect`    | POST   | Connect to a camera                                                                  |
| `/api/cameras/{id}/disconnect` | POST   | Disconnect from a camera                                                             |
| `/api/cameras/{id}`            | GET    | Get camera info                                                                      |
| `/api/capture/start`           | POST   | Start capture session                                                                |
| `/api/capture/stop`            | POST   | Stop capture session                                                                 |
| `/api/capture/status`          | GET    | Get capture status                                                                   |
| `/api/settings`                | GET    | Get current settings                                                                 |
| `/api/settings`                | POST   | Update settings                                                                      |
| `/ws/stream`                   | WS     | Live image stream (dynamic JPEG)                                                     |
| `/ws/eyepiece`                 | WS     | Eyepiece image stream (dynamic JPEG)                                                 |
| `/ws/eyepiece_quality`         | WS     | Lossless image stream (LZ4 compressed RGB8) at Eyepiece Streaming Resolution         |
| `/ws/events`                   | WS     | Server events (JSON)                                                                 |

> [!IMPORTANT]
> The following endpoints require the **Night Amplifier Pro** plugin to be installed and configured:

| Endpoint                       | Method | Description                                                         |
|--------------------------------|--------|---------------------------------------------------------------------|
| `/api/push-to/status`          | GET    | Get Push-To navigation status                                       |
| `/api/push-to/target`          | POST   | Set target by name or coordinates                                   |
| `/api/push-to/target`          | DELETE | Clear current target                                                |
| `/api/push-to/direction`       | GET    | Get push direction to target                                        |
| `/api/push-to/catalog/search`  | GET    | Search by any name, Messier/Caldwell number or catalog identifier   |
| `/api/push-to/catalog/messier` | GET    | List M1–M110 in number order                                        |
| `/api/push-to/catalog/ngc`     | GET    | List NGC objects                                                    |
| `/api/push-to/catalog/ic`      | GET    | List IC objects                                                     |
| `/api/astap/status`            | GET    | Get ASTAP installation status                                       |
| `/api/astap/install`           | POST   | Start ASTAP installation                                            |
| `/api/catalog/status`          | GET    | Get OpenNGC catalog status                                          |
| `/api/catalog/install`         | POST   | Start OpenNGC catalog install                                       |

The Vue 3 frontend is embedded in the binary; `--static-dir web/dist` serves it from disk instead.

#### Push-To Solve Lifecycle

A plate solve is offered a frame at most once a second, once the view has settled. Two independent checks decide: the
**movement detector** (has the star field changed enough to re-solve?) and the **solve gate** (should we try at all?)
— conflating them caused some of the worst Push-To bugs. The gate's three states:

| State       | Set by                                      | Cleared by                                           |
|-------------|---------------------------------------------|------------------------------------------------------|
| open        | success, startup                            | —                                                    |
| backing off | a failed solve (5 s, doubling, capped 120s) | the delay expiring, or any of the "arming" events    |
| suppressed  | a user cancel, clearing the target          | a new target, slewing the scope, an equipment change |

A cancel keeps the last solved position (still the best guess and what drives the guide arrow) until fresh user intent;
a failure discards it and schedules a retry. Changing focal length, sensor or binning calls `restart_solve`, which also
resets the movement detector — otherwise the field looks unchanged and nothing re-solves against the new optics.

Push-To events on `/ws/events`:

| Event                     | Meaning                                                             |
|---------------------------|---------------------------------------------------------------------|
| `plate_solving_started`   | A solve began; carries the target name                              |
| `plate_solving_progress`  | Which strategy in the ladder is in flight                           |
| `position_solved`         | A solve **on this frame** succeeded — never a cached position       |
| `position_solve_failed`   | The sky could not be matched                                        |
| `plate_solving_cancelled` | Abandoned on request; the last position is still good               |
| `plate_solving_restarted` | Abandoned because something it depended on changed; a retry follows |
| `push_to_blocked`         | Why solving is idle (`reason: null` clears the notice)              |
| `push_direction_updated`  | Sent only when the direction actually changes                       |

#### Live Stream Scaling

Each stream is encoded once per frame in the render task, not once per connected client, at a size chosen in
Settings: **Streaming Resolution** for the live view and eyepiece view (JPEG), **Eyepiece Streaming Resolution** for
the lossless eyepiece view — 1080p (JPEG only), 1440p, 4K or Native, both 1440p by default. Every client of a stream
receives the same frame, so ten phones cost one encode per frame, and a stream nobody is watching costs nothing.

#### Adding New Camera Providers

To add support for a new camera manufacturer:

1. Create a new module in `src/camera/` (e.g., `zwo.rs`)
2. Implement the `Camera` trait for your camera handle
3. Implement the `CameraProvider` trait for discovery/factory
4. Add the feature flag to `Cargo.toml`
5. Register in `CameraRegistry::register_defaults()`

### OpenTelemetry

Optional tracing and metrics for debugging and performance monitoring, built with `--features telemetry`:

```bash
cargo run --release --features telemetry -- --telemetry                                            # default endpoint http://localhost:4317
cargo run --release --features telemetry -- --telemetry --otlp-endpoint http://192.168.1.100:4317  # remote collector
cargo run --release --features telemetry -- --no-telemetry                                         # force off
```

**Environment variables:**

| Variable                      | Default                 | Description             |
|-------------------------------|-------------------------|-------------------------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://localhost:4317` | OTLP collector endpoint |
| `OTEL_SERVICE_NAME`           | `night-amplifier`       | Service name in traces  |

**Full observability stack:** `docker compose -f docker-compose.telemetry.yml up -d` starts Jaeger (traces), the
OpenTelemetry Collector, Prometheus (metrics) and Grafana (dashboards); then run the server with `--telemetry` as above.

| URL                        | What you see                                                       |
|----------------------------|--------------------------------------------------------------------|
| **http://localhost:16686** | Jaeger UI — distributed **traces** (spans, request timelines)      |
| **http://localhost:9090**  | Prometheus UI — raw **metrics** queries (gauges, counters)         |
| **http://localhost:3000**  | Grafana UI — **metric dashboards** and graphs (login: admin/admin) |

> **Traces vs Metrics:** Jaeger shows only traces (spans). For gauges such as `master_stack.memory_bytes` or
> `disk_writer.queue_depth`, use Prometheus or Grafana; Prometheus names replace dots with underscores
> (`master_stack_memory_bytes`).

### Web Frontend

Vue 3 interface for camera control from any browser: mobile-first, dark theme for night use, real-time WebSocket
streaming (dynamic JPEG for WiFi, LZ4 for the lossless eyepiece), WebGL rendering with Canvas2D fallback,
pinch-to-zoom, and Push-To and Comet Stacking panels (Pro). All npm commands run from `web/`:

```bash
npm install
npm run dev       # development server, proxies API to localhost:9955
npm run build     # production build to web/dist/, which the server binary embeds
npm test          # tests in watch mode (npm run test:run: single run)
npm run lint      # check for issues (npm run lint:fix: auto-fix)
npm run format    # format with Prettier
```

## License

Copyright (c) 2026- Nikita Kudrin (+ Night Amplifier contributors)

Licensed under GNU Affero General Public License as stated in the LICENSE:

Copyright (c) 2026- Nikita Kudrin & other Night Amplifier contributors

This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public
License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later
version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied
warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more
details.

You should have received a copy of the GNU Affero General Public License along with this program. If not,
see https://www.gnu.org/licenses/