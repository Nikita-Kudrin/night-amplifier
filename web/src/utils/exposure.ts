/** Microseconds in one of each exposure unit the controls offer. */
export const EXPOSURE_UNITS = {us: 1, ms: 1000, s: 1000000} as const

export type ExposureUnit = keyof typeof EXPOSURE_UNITS

/**
 * `us` microseconds as the controls show them: in the largest unit it reaches a whole
 * one of, so 2 s reads "2 s" and 500 ms "500 ms".
 */
export function splitExposure(us: number): {value: number; unit: ExposureUnit} {
    if (us >= EXPOSURE_UNITS.s) return {value: us / EXPOSURE_UNITS.s, unit: 's'}
    if (us >= EXPOSURE_UNITS.ms) return {value: us / EXPOSURE_UNITS.ms, unit: 'ms'}
    return {value: us, unit: 'us'}
}

/**
 * An exposure typed in `unit`, in whole microseconds: the server's field is an integer,
 * and 4.1 s is 4099999.9999999995 µs in floating point, which it refused. `null` for a
 * field holding no exposure (emptied, negative, under 1 µs): `''` read as 0 and was sent.
 */
export function toMicros(value: number | string | null, unit: string): number | null {
    if (value === '' || value === null) return null
    const units: Partial<Record<string, number>> = EXPOSURE_UNITS
    const us = Math.round(Number(value) * (units[unit] ?? 1))
    return Number.isFinite(us) && us >= 1 ? us : null
}
