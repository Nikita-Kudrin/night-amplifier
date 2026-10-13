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
 * the precision of the degree form (FOVs show 2, distances 1). The unit is picked after
 * rounding, so 0.99999° reads `1.00°`, not `60.0'`.
 */
export function formatAngle(degrees, degreeDigits = 1) {
    if (!isNumber(degrees)) return MISSING
    const arcminutes = (degrees * 60).toFixed(1)
    if (Math.abs(Number(arcminutes)) < 60) return `${arcminutes}'`
    return `${degrees.toFixed(degreeDigits)}°`
}

/** A field of view: `formatAngle` at FOV precision. */
export function formatFov(degrees) {
    return formatAngle(degrees, 2)
}

const KIB = 1024
const BYTE_UNITS = [['B', 0], ['KB', 0], ['MB', 1], ['GB', 2]]

/**
 * A download size in binary units: `"512 KB"`, `"12.3 MB"`, `"1.30 GB"`. Each unit is tried
 * at its display precision, so 1048575 B reads `1.0 MB`, not `1024 KB`.
 */
export function formatBytes(bytes) {
    if (!isNumber(bytes)) return MISSING
    let value = bytes
    for (const [index, [unit, digits]] of BYTE_UNITS.entries()) {
        const shown = value.toFixed(digits)
        if (Number(shown) < KIB || index === BYTE_UNITS.length - 1) return `${shown} ${unit}`
        value /= KIB
    }
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
