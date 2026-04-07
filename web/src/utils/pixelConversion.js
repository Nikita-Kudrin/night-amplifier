/**
 * Maps a flat 3-channel RGB8 array (Uint8Array) to a 4-channel RGBA8 array
 * for Canvas 2D fallback rendering.
 * @param {Uint8Array} rgb8 - The raw 8-bit RGB data
 * @param {number} width - Image width
 * @param {number} height - Image height
 * @returns {Uint8ClampedArray} RGBA8 data
 */
export function rgb8ToRgba8(rgb8, width, height) {
    const pixelCount = width * height;
    const rgba8 = new Uint8ClampedArray(pixelCount * 4);

    for (let i = 0; i < pixelCount; i++) {
        const src = i * 3;
        const dst = i * 4;

        rgba8[dst] = rgb8[src]; // Red
        rgba8[dst + 1] = rgb8[src + 1]; // Green
        rgba8[dst + 2] = rgb8[src + 2]; // Blue
        rgba8[dst + 3] = 255; // Alpha
    }

    return rgba8;
}
