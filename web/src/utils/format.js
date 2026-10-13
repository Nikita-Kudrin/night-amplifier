/**
 * Display strings for numbers the UI shows in more than one place. Every formatter answers
 * `'—'` for a missing or non-finite value rather than printing `NaN`.
 */

export const MISSING = '—'

const isNumber = (value) => typeof value === 'number' && Number.isFinite(value)

/** A fraction as a percentage: `0.42` → `"42%"`. Callers holding 0–100 divide first. */
export function formatPercent(fraction, digits = 0) {
    return isNumber(fraction) ? `${(fraction * 100).toFixed(digits)}%` : MISSING
}

/** A rejection threshold: `2.5` → `"2.5σ"`. */
export function formatSigma(sigma) {
    return isNumber(sigma) ? `${sigma.toFixed(1)}σ` : MISSING
}

/** A sensor temperature: `-9.96` → `"-10.0°C"`. */
export function formatTemperature(celsius) {
    return isNumber(celsius) ? `${celsius.toFixed(1)}°C` : MISSING
}

/**
 * An angle on the sky: arcminutes below a degree, degrees from there. `degreeDigits` sets
 * the precision of the degree form (FOVs show 2, distances 1).
 */
export function formatAngle(degrees, degreeDigits = 1) {
    if (!isNumber(degrees)) return MISSING
    if (Math.abs(degrees) < 1) return `${(degrees * 60).toFixed(1)}'`
    return `${degrees.toFixed(degreeDigits)}°`
}

/** A field of view: `formatAngle` at FOV precision. */
export function formatFov(degrees) {
    return formatAngle(degrees, 2)
}

const KIB = 1024
const MIB = KIB * 1024
const GIB = MIB * 1024

/** A download size in binary units: `"512 KB"`, `"12.3 MB"`, `"1.30 GB"`. */
export function formatBytes(bytes) {
    if (!isNumber(bytes)) return MISSING
    if (bytes < KIB) return `${Math.round(bytes)} B`
    if (bytes < MIB) return `${Math.round(bytes / KIB)} KB`
    if (bytes < GIB) return `${(bytes / MIB).toFixed(1)} MB`
    return `${(bytes / GIB).toFixed(2)} GB`
}

/** A short duration: one decimal below 10 ms, whole milliseconds above. */
export function formatMs(ms) {
    if (!isNumber(ms)) return ''
    return ms < 10 ? `${ms.toFixed(1)} ms` : `${Math.round(ms)} ms`
}

/** An image size, `"3008x3008"`; a camera listed without being opened reports 0x0. */
export function formatResolution(width, height) {
    return width && height ? `${width}x${height}` : MISSING
}
