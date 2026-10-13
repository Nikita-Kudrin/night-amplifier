const positive = (value: number | null | undefined): value is number => value != null && value > 0

/**
 * The angle one sensor dimension spans on the sky, in degrees: `2·atan(size / 2f)`, the
 * formula Pro's solver plans its FOV hint with, so the UI's warning about a database's
 * range judges the number the solver will use. `null` while any input is missing.
 *
 * `pixels` is the sensor's size along the dimension, `pixelSizeUm` its pixel pitch in µm,
 * and `barlow` the Barlow (> 1) or reducer (< 1) factor.
 */
export function fieldOfViewDeg(
    pixels: number | null | undefined,
    pixelSizeUm: number | null | undefined,
    focalLengthMm: number | null | undefined,
    barlow: number | null = 1
): number | null {
    if (!(positive(pixels) && positive(pixelSizeUm) && positive(focalLengthMm))) return null
    const sizeMm = (pixels * pixelSizeUm) / 1000
    const effectiveFocalMm = focalLengthMm * (barlow || 1)
    return (2 * Math.atan(sizeMm / (2 * effectiveFocalMm)) * 180) / Math.PI
}
