/**
 * Expands RGB8 pixels into an RGBA8 buffer (opaque), for Canvas 2D's `ImageData`.
 * Writes into `rgba8`, 4 bytes for each pixel of `rgb8`, and returns it, rather than
 * allocating: at 1440² a fresh buffer was ~8 MB a frame.
 */
export function rgb8ToRgba8<T extends Uint8ClampedArray | Uint8Array>(rgb8: Uint8Array, rgba8: T): T {
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

/**
 * The size `width`x`height` fits inside `maxEdge` a side at, aspect kept and never enlarged:
 * the long side is the edge exactly, the short one rounded down, as the server's
 * `output_dimensions` does.
 */
export function fitWithinEdge(width: number, height: number, maxEdge: number): {width: number; height: number} {
    if (width <= maxEdge && height <= maxEdge) return {width, height}
    if (width >= height) return {width: maxEdge, height: Math.max(1, Math.floor((maxEdge * height) / width))}
    return {width: Math.max(1, Math.floor((maxEdge * width) / height)), height: maxEdge}
}

/**
 * Box-averages interleaved RGB8 by the smallest whole factor that brings both sides to
 * `maxEdge` or under. A whole factor keeps every output pixel the mean of the same number
 * of samples; the cost is a frame up to half a factor smaller than `maxEdge` allows.
 */
export function downscaleRgb8(rgb8: Uint8Array, width: number, height: number, maxEdge: number) {
    const factor = Math.ceil(Math.max(width, height) / maxEdge)
    if (factor <= 1) return {data: rgb8, width, height}
    const outWidth = Math.floor(width / factor)
    const outHeight = Math.floor(height / factor)
    const out = new Uint8Array(outWidth * outHeight * 3)
    const sums = new Uint32Array(outWidth * 3)
    const samples = factor * factor
    for (let oy = 0; oy < outHeight; oy++) {
        sums.fill(0)
        for (let y = oy * factor; y < (oy + 1) * factor; y++) {
            const row = y * width * 3
            for (let ox = 0; ox < outWidth; ox++) {
                for (let x = ox * factor; x < (ox + 1) * factor; x++) {
                    const src = row + x * 3
                    sums[ox * 3] += rgb8[src]
                    sums[ox * 3 + 1] += rgb8[src + 1]
                    sums[ox * 3 + 2] += rgb8[src + 2]
                }
            }
        }
        const dst = oy * outWidth * 3
        for (let i = 0; i < outWidth * 3; i++) out[dst + i] = Math.round(sums[i] / samples)
    }
    return {data: out, width: outWidth, height: outHeight}
}
