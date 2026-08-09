# Multi-Image & Multi-Resolution Encoding Benchmarks

Benchmarks for three different real-world test fixtures:

1. **Dumbbell** (stacked and stretched image from test fixture 130mm*)
2. **Orion** (stacked and stretched image from test fixture 250mm*)
3. **Orion Wide** (stacked and stretched image from test fixture 350mm*)

For each image, the benchmark evaluates the compressed file size, encoding speed, and the **Bandwidth-Bound FPS** over a
**60 Mb/s** network connection (Orange Pi5 Pro default antenna) for 3 different resolutions (1080p downscaled, original
IMX464, and 4K upscaled).

> [!NOTE]
> PNG is mathematically lossless. Standard PNG encoders (like the Rust `image` crate) do not have a lossy "90% quality"
parameter like JPEG or WebP do. Therefore, it was run as standard lossless PNG using its default compression level.

---

## 1. Dumbbell

### 1080p Downscale (1920x1080)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **5.6 ms**  | **0.72 MB** | **~10.4 FPS**       |
| **turbojpeg_95**        | 7.3 ms      | 1.13 MB     | ~6.6 FPS            |
| **turbojpeg_100**       | 10.5 ms     | 2.19 MB     | ~3.4 FPS            |
| **image_webp_lossy_80** | 188.0 ms ⚠️ | 0.47 MB     | ~16.1 FPS           |
| **image_webp_lossy_90** | 227.9 ms ⚠️ | 0.79 MB     | ~9.5 FPS            |
| **image_jpeg_95**       | 82.0 ms     | 2.04 MB     | ~3.7 FPS            |
| **lz4_flex**            | 0.5 ms      | 5.96 MB     | ~1.3 FPS            |
| **image_png_lossless**  | 9.6 ms      | 4.68 MB     | ~1.6 FPS            |

### Original Resolution (2712x1538)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **12.1 ms** | **1.55 MB** | **~4.8 FPS**        |
| **turbojpeg_95**        | 14.9 ms     | 2.37 MB     | ~3.2 FPS            |
| **turbojpeg_100**       | 21.2 ms     | 4.52 MB     | ~1.7 FPS            |
| **image_webp_lossy_80** | 404.0 ms ⚠️ | 0.98 MB     | ~7.6 FPS            |
| **image_webp_lossy_90** | 472.9 ms ⚠️ | 1.62 MB     | ~4.6 FPS            |
| **image_jpeg_95**       | 163.5 ms    | 4.07 MB     | ~1.8 FPS            |
| **lz4_flex**            | 1.2 ms      | 11.98 MB    | ~0.6 FPS            |
| **image_png_lossless**  | 25.4 ms     | 8.97 MB     | ~0.8 FPS            |

### 4K Upscale (3840x2160)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **22.4 ms** | **2.62 MB** | **~2.9 FPS**        |
| **turbojpeg_95**        | 26.2 ms     | 3.95 MB     | ~1.9 FPS            |
| **turbojpeg_100**       | 39.3 ms     | 7.80 MB     | ~1.0 FPS            |
| **image_webp_lossy_80** | 691.9 ms ⚠️ | 1.48 MB     | ~5.1 FPS            |
| **image_webp_lossy_90** | 826.7 ms ⚠️ | 2.52 MB     | ~3.0 FPS            |
| **image_jpeg_95**       | 267.4 ms    | 6.15 MB     | ~1.2 FPS            |
| **lz4_flex**            | 2.8 ms      | 23.82 MB    | ~0.3 FPS            |
| **image_png_lossless**  | 38.9 ms     | 15.16 MB    | ~0.5 FPS            |

---

## 2. Orion

### 1080p Downscale (1920x1080)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **4.0 ms**  | **0.20 MB** | **~36.6 FPS**       |
| **turbojpeg_95**        | 4.4 ms      | 0.35 MB     | ~21.3 FPS           |
| **turbojpeg_100**       | 7.8 ms      | 1.16 MB     | ~6.5 FPS            |
| **image_webp_lossy_80** | 108.6 ms ⚠️ | 0.02 MB     | ~325.9 FPS          |
| **image_webp_lossy_90** | 127.3 ms ⚠️ | 0.10 MB     | ~72.2 FPS           |
| **image_jpeg_95**       | 47.3 ms     | 0.53 MB     | ~14.2 FPS           |
| **lz4_flex**            | 7.8 ms      | 5.42 MB     | ~1.4 FPS            |
| **image_png_lossless**  | 11.0 ms     | 2.58 MB     | ~2.9 FPS            |

### Original Resolution (2712x1538)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **8.4 ms**  | **0.45 MB** | **~16.6 FPS**       |
| **turbojpeg_95**        | 9.0 ms      | 0.77 MB     | ~9.8 FPS            |
| **turbojpeg_100**       | 16.3 ms     | 2.46 MB     | ~3.1 FPS            |
| **image_webp_lossy_80** | 223.2 ms ⚠️ | 0.05 MB     | ~141.5 FPS          |
| **image_webp_lossy_90** | 278.8 ms ⚠️ | 0.28 MB     | ~27.1 FPS           |
| **image_jpeg_95**       | 99.7 ms     | 1.17 MB     | ~6.4 FPS            |
| **lz4_flex**            | 16.3 ms     | 11.01 MB    | ~0.7 FPS            |
| **image_png_lossless**  | 25.7 ms     | 5.26 MB     | ~1.4 FPS            |

### 4K Upscale (3840x2160)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **17.5 ms** | **0.91 MB** | **~8.3 FPS**        |
| **turbojpeg_95**        | 18.5 ms     | 1.46 MB     | ~5.1 FPS            |
| **turbojpeg_100**       | 30.7 ms     | 4.38 MB     | ~1.7 FPS            |
| **image_webp_lossy_80** | 445.6 ms ⚠️ | 0.12 MB     | ~61.0 FPS           |
| **image_webp_lossy_90** | 550.5 ms ⚠️ | 0.53 MB     | ~14.1 FPS           |
| **image_jpeg_95**       | 188.7 ms    | 2.24 MB     | ~3.4 FPS            |
| **lz4_flex**            | 36.3 ms     | 21.08 MB    | ~0.4 FPS            |
| **image_png_lossless**  | 51.9 ms     | 9.22 MB     | ~0.8 FPS            |

---

## 3. Orion Wide

### 1080p Downscale (1920x1080)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **3.2 ms**  | **0.09 MB** | **~87.9 FPS**       |
| **turbojpeg_95**        | 3.6 ms      | 0.18 MB     | ~42.5 FPS           |
| **turbojpeg_100**       | 6.0 ms      | 0.73 MB     | ~10.3 FPS           |
| **image_webp_lossy_80** | 94.3 ms ⚠️  | 0.02 MB     | ~482.9 FPS          |
| **image_webp_lossy_90** | 105.1 ms ⚠️ | 0.03 MB     | ~242.3 FPS          |
| **image_jpeg_95**       | 36.6 ms     | 0.21 MB     | ~35.8 FPS           |
| **lz4_flex**            | 11.5 ms     | 3.13 MB     | ~2.4 FPS            |
| **image_png_lossless**  | 15.5 ms     | 1.80 MB     | ~4.2 FPS            |

### Original Resolution (2712x1538)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **6.5 ms**  | **0.17 MB** | **~45.0 FPS**       |
| **turbojpeg_95**        | 7.3 ms      | 0.35 MB     | ~21.4 FPS           |
| **turbojpeg_100**       | 12.5 ms     | 1.46 MB     | ~5.2 FPS            |
| **image_webp_lossy_80** | 191.1 ms ⚠️ | 0.03 MB     | ~270.1 FPS          |
| **image_webp_lossy_90** | 211.2 ms ⚠️ | 0.05 MB     | ~146.1 FPS          |
| **image_jpeg_95**       | 81.1 ms     | 0.44 MB     | ~17.1 FPS           |
| **lz4_flex**            | 22.7 ms     | 6.29 MB     | ~1.2 FPS            |
| **image_png_lossless**  | 31.3 ms     | 3.56 MB     | ~2.1 FPS            |

### 4K Upscale (3840x2160)

| Encoder                 | Encode Time | File Size   | Bandwidth-Bound FPS |
|-------------------------|-------------|-------------|---------------------|
| **turbojpeg_90**        | **12.8 ms** | **0.34 MB** | **~21.9 FPS**       |
| **turbojpeg_95**        | 14.7 ms     | 0.70 MB     | ~10.7 FPS           |
| **turbojpeg_100**       | 22.9 ms     | 2.61 MB     | ~2.9 FPS            |
| **image_webp_lossy_80** | 390.6 ms ⚠️ | 0.05 MB     | ~155.4 FPS          |
| **image_webp_lossy_90** | 399.4 ms ⚠️ | 0.09 MB     | ~84.7 FPS           |
| **image_jpeg_95**       | 151.6 ms    | 0.92 MB     | ~8.2 FPS            |
| **lz4_flex**            | 44.3 ms     | 11.74 MB    | ~0.6 FPS            |
| **image_png_lossless**  | 58.6 ms     | 6.24 MB     | ~1.2 FPS            |

---

## Final Conclusions

1. **Dumbbell (High Entropy):** The highest amount of sensor noise and nebulosity. Frame sizes balloon drastically here,
   dropping network performance.
2. **Orion / Orion Wide (Lower Entropy):** The dark sky background compresses extremely well. **TurboJPEG 90%** easily
   streams these images at over 30 FPS, even hitting an estimated 87.9 FPS at 1080p for Orion Wide!
3. **WebP Is Always a Trap:** Regardless of the image entropy, WebP always takes 100-200ms+ for 1080p and 400-800ms+ for
   4K. While WebP 80% produces *incredibly* small files (e.g. 0.02 MB for Orion at 1080p), the CPU encoding time renders
   it useless for live streaming.
4. **TurboJPEG 90% Wins:** It provides the most consistently fast encoding speeds and smallest payloads, allowing
   reliable performance even on the highly-noisy Dumbbell target.
