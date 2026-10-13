/**
 * Equatorial coordinates between degrees and the sexagesimal strings the UI reads and
 * writes. Values are rounded *before* being split into fields, so 59.96 s carries into the
 * next minute instead of showing as `60.0s` (the bug Pro's catalog strings once had).
 */

const TENTHS_PER_UNIT = 36000 // tenths of a second in an hour, or of an arcsecond in a degree

/** `[whole, minutes, seconds]` of a non-negative value, seconds to one decimal. */
function sexagesimal(value: number): [number, number, string] {
    const tenths = Math.round(value * TENTHS_PER_UNIT)
    const whole = Math.floor(tenths / TENTHS_PER_UNIT)
    const minutes = Math.floor((tenths % TENTHS_PER_UNIT) / 600)
    const seconds = (tenths % 600) / 10
    return [whole, minutes, seconds.toFixed(1)]
}

const DECIMAL = /^[+-]?(?:\d+\.?\d*|\.\d+)$/
const FIELD = /^\d+(?:\.\d+)?$/
const SEXAGESIMAL_SEPARATORS = /[\s:hmsd°'′"″]+/i
/** Which coordinate a string is: right ascension or declination. */
export type CoordinateType = 'ra' | 'dec'

const RANGE: Record<CoordinateType, [min: number, max: number]> = {ra: [0, 360], dec: [-90, 90]}

/**
 * Signed whole units (hours or degrees) of a 2-3 field sexagesimal string, or null. The sign
 * is read from the text: taken from the whole field, `-00:30` lost it and came out as +0.5°.
 */
function parseSexagesimal(text: string): number | null {
    const fields = text.replace(/^[+-]/, '').split(SEXAGESIMAL_SEPARATORS)
    if (fields.at(-1) === '') fields.pop() // a trailing unit, as in `5h35m`
    if (fields.length < 2 || fields.length > 3 || !fields.every((f) => FIELD.test(f))) return null
    const [whole, minutes, seconds = 0] = fields.map(Number)
    if (minutes >= 60 || seconds >= 60) return null
    return (text.startsWith('-') ? -1 : 1) * (whole + minutes / 60 + seconds / 3600)
}

function toDegrees(text: string, type: CoordinateType): number | null {
    if (DECIMAL.test(text)) return Number(text)
    const units = parseSexagesimal(text)
    return units !== null && type === 'ra' ? units * 15 : units
}

/**
 * Parse a coordinate string to degrees: decimal degrees, or HH:MM:SS (RA) / DD:MM:SS (Dec)
 * separated by colons, spaces or unit symbols (`5h35m17s`, `−5°23′28″`). Decimal only when the
 * whole string is a number: `parseFloat` first read `05:35:17` as 5°. RA goes up to 360° / 24h.
 * `null` if it is none of these, or out of range.
 */
export function parseCoordinate(value: string | null | undefined, type: CoordinateType): number | null {
    // U+2212, the minus sign catalogs and Wikipedia print.
    const text = value?.trim().replace(/^\u2212/, '-')
    if (!text) return null

    const degrees = toDegrees(text, type)
    if (degrees === null) return null
    const [min, max] = RANGE[type]
    return degrees >= min && degrees <= max ? degrees : null
}

/** Format Right Ascension from degrees to HMS string; 24h wraps to 0h. */
export function formatRA(degrees: number | null | undefined): string {
    if (degrees === undefined || degrees === null) return '--'
    const [h, m, s] = sexagesimal(degrees / 15)
    return `${h % 24}h ${m}m ${s}s`
}

/** Format Declination from degrees to DMS string. */
export function formatDec(degrees: number | null | undefined): string {
    if (degrees === undefined || degrees === null) return '--'
    const sign = degrees >= 0 ? '+' : '-'
    const [d, m, s] = sexagesimal(Math.abs(degrees))
    return `${sign}${d}° ${m}' ${s}"`
}
