/**
 * Equatorial coordinates between degrees and the sexagesimal strings the UI reads and
 * writes. Values are rounded *before* being split into fields, so 59.96 s carries into the
 * next minute instead of showing as `60.0s` (the bug Pro's catalog strings once had).
 */

const TENTHS_PER_UNIT = 36000 // tenths of a second in an hour, or of an arcsecond in a degree

/** `[whole, minutes, seconds]` of a non-negative value, seconds to one decimal. */
function sexagesimal(value) {
    const tenths = Math.round(value * TENTHS_PER_UNIT)
    const whole = Math.floor(tenths / TENTHS_PER_UNIT)
    const minutes = Math.floor((tenths % TENTHS_PER_UNIT) / 600)
    const seconds = (tenths % 600) / 10
    return [whole, minutes, seconds.toFixed(1)]
}

/**
 * Parse coordinate string to degrees
 * @param {string} value - Coordinate string (decimal degrees or HMS/DMS format)
 * @param {'ra' | 'dec'} type - Coordinate type
 * @returns {number | null} Degrees or null if invalid
 */
export function parseCoordinate(value, type) {
    if (!value || !value.trim()) return null

    const trimmed = value.trim()

    // Try decimal degrees first
    const decimal = parseFloat(trimmed)
    if (!isNaN(decimal)) {
        if (type === 'ra' && (decimal < 0 || decimal > 360)) return null
        if (type === 'dec' && (decimal < -90 || decimal > 90)) return null
        return decimal
    }

    // Try HMS/DMS format (e.g., "12:30:45" or "12 30 45")
    const parts = trimmed.split(/[:\s]+/).map((p) => parseFloat(p))
    if (parts.length >= 2 && parts.length <= 3 && parts.every((p) => !isNaN(p))) {
        const [d, m, s = 0] = parts
        const sign = d < 0 ? -1 : 1
        let degrees = Math.abs(d) + m / 60 + s / 3600

        if (type === 'ra') {
            // Convert hours to degrees for RA
            degrees = degrees * 15
            if (degrees > 360) return null
        } else {
            degrees = sign * degrees
            if (degrees < -90 || degrees > 90) return null
        }

        return degrees
    }

    return null
}

/**
 * Format Right Ascension from degrees to HMS string; 24h wraps to 0h.
 * @param {number | null | undefined} degrees - RA in degrees
 * @returns {string} Formatted string
 */
export function formatRA(degrees) {
    if (degrees === undefined || degrees === null) return '--'
    const [h, m, s] = sexagesimal(degrees / 15)
    return `${h % 24}h ${m}m ${s}s`
}

/**
 * Format Declination from degrees to DMS string
 * @param {number | null | undefined} degrees - Dec in degrees
 * @returns {string} Formatted string
 */
export function formatDec(degrees) {
    if (degrees === undefined || degrees === null) return '--'
    const sign = degrees >= 0 ? '+' : '-'
    const [d, m, s] = sexagesimal(Math.abs(degrees))
    return `${sign}${d}° ${m}' ${s}"`
}
