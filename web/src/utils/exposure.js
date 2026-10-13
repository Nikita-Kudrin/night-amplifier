/** Microseconds in one of each exposure unit the controls offer. */
export const EXPOSURE_UNITS = Object.freeze({us: 1, ms: 1000, s: 1000000})

/**
 * `us` microseconds as the controls show them: in the largest unit it reaches a whole
 * one of, so 2 s reads "2 s" and 500 ms "500 ms".
 * @returns {{value: number, unit: 'us'|'ms'|'s'}}
 */
export function splitExposure(us) {
    if (us >= EXPOSURE_UNITS.s) return {value: us / EXPOSURE_UNITS.s, unit: 's'}
    if (us >= EXPOSURE_UNITS.ms) return {value: us / EXPOSURE_UNITS.ms, unit: 'ms'}
    return {value: us, unit: 'us'}
}

/**
 * An exposure typed in `unit`, in whole microseconds: the server's field is an integer,
 * and 4.1 s is 4099999.9999999995 µs in floating point, which it refused.
 */
export function toMicros(value, unit) {
    return Math.round(value * (EXPOSURE_UNITS[unit] ?? 1))
}
