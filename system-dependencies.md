# System dependencies

Night Amplifier is one binary and needs nothing else to run: capture from the simulator,
stack, stretch and stream work out of the box. What is listed here unlocks hardware — a
camera brand, a GPU or an NPU for AI denoising — and is installed on the system, never
shipped with Night Amplifier: vendor libraries are loaded from where their own installers
put them, and several must match the driver installed with them.

| For                                  | Linux                                                          | Windows                         | macOS          |
|--------------------------------------|----------------------------------------------------------------|---------------------------------|----------------|
| A camera brand                       | Its SDK + udev rules ([Camera SDKs](#camera-sdks))              | Its SDK / driver                | Its SDK        |
| AI denoising on a GPU *(Pro)*        | A Vulkan driver ([GPU](#gpu-dedicated-or-integrated))           | The graphics driver             | Nothing        |
| AI denoising on an NPU *(Pro)*       | The NPU's runtime and driver ([NPU](#npu))                      | Intel NPU: OpenVINO + driver    | Nothing        |
| AI denoising on the CPU *(Pro)*      | Nothing                                                        | Nothing                         | Nothing        |

## Camera SDKs

Camera SDKs are loaded at runtime and are **optional**: without one installed the binary still runs, with that brand
disabled. They are not shipped with Night Amplifier — install each from its vendor. Every provider is a default Cargo
feature (`playerone`, `zwo`, `qhy`, `touptek`, `svbony`, `indi`).

| Provider   | SDK Required                                                         | Supported                                                 |
|------------|----------------------------------------------------------------------|-----------------------------------------------------------|
| Player One | [Player One SDK](https://player-one-astronomy.com/service/software/) | ✅                                                        |
| ZWO (ASI)  | [ZWO ASI SDK](https://astronomy-imaging-camera.com/software-drivers) | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| ToupTek    | [ToupTek SDK](http://www.touptek.com/download/)                      | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| QHYCCD     | [QHYCCD SDK](https://www.qhyccd.com/download/)                       | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| SVBony     | [SVBony SDK](https://www.svbony.com/downloads)                       | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| INDI       | [INDI server](https://indilib.org/download.html)                     | ![Testing](https://img.shields.io/badge/🚀_Testing-green) |
| Simulated  | Loads PNG/TIFF/FITS/SER from directories                             | ✅                                                        |

### Camera SDK setup (Linux)

Each SDK is needed only to use its brand. Install udev rules (USB permissions) and the shared library as shown per
vendor below, then **unplug and replug the camera**; `ldconfig -p | grep <library>` confirms the library is found.
Player One, ZWO, QHYCCD and SVBony SDKs need **libusb-1.0**; Player One also lists **libclang** (bindgen):

```bash
sudo apt-get install libusb-1.0-0 libclang-dev   # Debian/Ubuntu/Raspberry Pi OS
sudo dnf install libusb-1.0 clang-devel          # Fedora
sudo pacman -S libusb clang                      # Arch Linux/Manjaro
```

#### Player One

From the extracted [Player One SDK](https://player-one-astronomy.com/service/software/):

```bash
sudo install 99-player_one_astronomy.rules /lib/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger
sudo cp libPlayerOneCamera.so /usr/local/lib/ && sudo ldconfig
ldconfig -p | grep PlayerOne
```

#### ZWO

From the extracted [ZWO ASI SDK](https://astronomy-imaging-camera.com/software-drivers):

```bash
sudo install lib/asi.rules /lib/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger
sudo cp include/ASICamera2.h /usr/local/include/                   # header, required for building
sudo cp lib/x64/libASICamera2.so /usr/local/lib/ && sudo ldconfig  # ARM (Raspberry Pi): lib/armv8/libASICamera2.so
ldconfig -p | grep ASICamera
```

#### QHY

From the extracted [QHYCCD SDK](https://www.qhyccd.com/download/):

```bash
sudo install sdk/linux/mac/rules/85-qhyccd.rules /lib/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger
sudo cp sdk/linux/mac/lib/libqhyccd.so* /usr/local/lib/ && sudo ldconfig
ldconfig -p | grep qhyccd
```

#### ToupTek

Needs `libtoupcam.so` / `libtoupcam.dylib` / `toupcam.dll`: the Linux SDK from the
[ToupTek Download Page](http://www.touptek.com/download/), or up-to-date binaries for all architectures from the
[INDIGO repository](https://github.com/indigo-astronomy/indigo/tree/master/indigo_drivers/ccd_touptek/bin_externals/libtoupcam).
Install it to a library path (e.g. `/usr/local/lib/`), add udev rules for your camera (often shipped by the
manufacturer or INDI), then replug. Check: `ldconfig -p | grep toupcam`.

#### SVBony

Needs `libSVBony.so` / `libSVBCameraSDK.dylib` / `SVBony.dll` from the [SVBony Downloads Page](https://www.svbony.com/downloads).
Install it to a library path (e.g. `/usr/local/lib/`), add udev rules for your camera, then replug. Check:
`ldconfig -p | grep SVBony`.

## AI denoising compute units (Pro)

On its first start on a computer, Night Amplifier Pro measures every NPU and GPU it can use on four IMX533 frames
(**Benchmarking hardware…**, once) and runs AI denoising on the fastest one that is not the CPU; the CPU is the
fallback. **Settings → Advanced → AI compute** lists every option — greyed out, with the reason, where something below
is missing — and lets you force one. The result is kept in `ai_compute.json` beside `settings.json`; it is measured
again when the computer, a driver or a runtime changes, or when that file is deleted.

Nothing below is needed for AI denoising itself: without any of it the CPU runs the network.

### GPU (dedicated or integrated)

- **Linux** — a Vulkan driver and loader:
  - Intel and AMD graphics: `sudo apt install libvulkan1 mesa-vulkan-drivers` (Debian, Ubuntu, Raspberry Pi OS; installed
    by default on desktop images).
  - NVIDIA: the proprietary driver, which includes Vulkan (`sudo ubuntu-drivers install` on Ubuntu). The open `nouveau`
    driver has no Vulkan before Mesa 24.1 (NVK); such a card is listed as "present, but no Vulkan driver".
  - Check: `vulkaninfo --summary` (package `vulkan-tools`).
- **Raspberry Pi 5** — VideoCore VII through Mesa's V3DV, installed with Raspberry Pi OS desktop (Lite:
  `sudo apt install libvulkan1 mesa-vulkan-drivers`). It is slower than the Pi's CPU, but Auto still prefers it because
  the CPU is busy stacking; choose **CPU** under AI compute if you prefer.
- **Orange Pi 5 / 5 Pro / 5 Plus and other RK3588 boards** — Mali-G610 through Mesa's PanVK: the Panthor kernel driver
  (Linux 6.10 or later) and Mesa 25.0 or later (mainline-kernel images such as Armbian's). Vendor images with the Mali
  blob have no Vulkan; the GPU is then greyed out and the NPU and CPU still work.
- **Windows** — the graphics driver only; D3D12 is part of Windows.
- **macOS** — nothing; Metal is part of macOS.

### NPU

- **Rockchip RK3588 / RK3588S / RK3576** (Orange Pi 5 family, Radxa Rock 5, …) — runs in FP16:
  - the `rknpu` kernel driver: included in Orange Pi's official images (vendor kernel); on mainline kernels install its
    DKMS module;
  - `librknnrt.so` **2.3.2 or newer** in `/usr/lib`: Orange Pi images ship it; otherwise copy
    `rknpu2/runtime/Linux/librknn_api/aarch64/librknnrt.so` from
    [airockchip/rknn-toolkit2](https://github.com/airockchip/rknn-toolkit2) and run `sudo ldconfig`. It is not shipped
    with Night Amplifier because it must match the kernel driver.
  - Check: `ldconfig -p | grep rknnrt`.
- **Intel NPU** (Core Ultra "AI Boost"; not N100/N150) — through OpenVINO, in FP16:
  - Linux: kernel 6.6 or later (`intel_vpu`), the [Intel NPU driver](https://github.com/intel/linux-npu-driver) and the
    [OpenVINO runtime](https://docs.openvino.ai/) 2025 or later. `libopenvino_c.so` is looked for in
    `/usr/lib/x86_64-linux-gnu`, `/opt/intel/openvino*/runtime/lib/intel64`, `$INTEL_OPENVINO_DIR`, or the path in
    `NIGHT_AMPLIFIER_OPENVINO_LIB`. Check: `ls /dev/accel/`.
  - Windows: the Intel NPU driver (Windows Update or Intel) and the OpenVINO runtime; set `INTEL_OPENVINO_DIR` or put
    `runtime\bin\intel64\Release` on `PATH`.
- **Hailo-8 / 8L / 10H** (Raspberry Pi AI Kit, AI HAT+, AI HAT+ 2; M.2 cards) — 16-bit integer:
  - `sudo apt install hailo-all` (Hailo-8/8L) or `sudo apt install hailo-h10-all` (Hailo-10H), then reboot. Check:
    `hailortcli fw-control identify`.
  - This release ships no Hailo model yet (it needs Hailo's Dataflow Compiler), so a fitted accelerator is listed as not
    available until one does.
- **Apple Neural Engine** (Apple silicon) — nothing to install; Core ML is part of macOS. Checking that every layer runs
  on the Neural Engine needs macOS 14.4 or later.
- **AMD Ryzen AI and Qualcomm Snapdragon NPUs** — not supported; their GPUs are used instead.
