/**
 * Expands RGB8 pixels into an RGBA8 buffer (opaque), for Canvas 2D's `ImageData`.
 * Writes into `rgba8` rather than allocating: at 1440² a fresh buffer was ~8 MB a frame.
 * @param {Uint8Array} rgb8 - Interleaved 8-bit RGB
 * @param {Uint8ClampedArray|Uint8Array} rgba8 - Destination, 4 bytes for each pixel of `rgb8`
 * @returns {Uint8ClampedArray|Uint8Array} `rgba8`
 */
export function rgb8ToRgba8(rgb8, rgba8) {
    const pixelCount = Math.min(rgb8.length / 3, rgba8.length / 4)

    for (let i = 0; i < pixelCount; i++) {
        const src = i * 3
        const dst = i * 4

        rgba8[dst] = rgb8[src]
        rgba8[dst + 1] = rgb8[src + 1]
        rgba8[dst + 2] = rgb8[src + 2]
        rgba8[dst + 3] = 255
    }

    return rgba8
}
