/**
 * The angle one sensor dimension spans on the sky, in degrees: `2·atan(size / 2f)`, the
 * formula Pro's solver plans its FOV hint with, so the UI's warning about a database's
 * range judges the number the solver will use. `null` while any input is missing.
 *
 * @param {number} pixels - Sensor size along the dimension, in pixels
 * @param {number} pixelSizeUm - Pixel pitch along it, in µm
 * @param {number} focalLengthMm - Telescope focal length
 * @param {number} [barlow] - Barlow (> 1) or reducer (< 1) factor
 */
export function fieldOfViewDeg(pixels, pixelSizeUm, focalLengthMm, barlow = 1) {
    if (!(pixels > 0 && pixelSizeUm > 0 && focalLengthMm > 0)) return null
    const sizeMm = (pixels * pixelSizeUm) / 1000
    const effectiveFocalMm = focalLengthMm * (barlow || 1)
    return (2 * Math.atan(sizeMm / (2 * effectiveFocalMm)) * 180) / Math.PI
}
