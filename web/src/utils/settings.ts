import {defaultSettings} from '../constants'
import type {Settings} from '../composables/api.types.js'

/**
 * A setting by its dotted path in `T`: `'gain'`, `'eyepiece.intensity'`. Paths go into
 * groups of fixed keys (`eyepiece`, `denoise`, `guide_camera`), not into maps keyed by
 * camera, lists, or a value that may be missing.
 */
export type SettingPath<T = Settings> = {
    [K in keyof T & string]-?: T[K] extends readonly unknown[]
        ? K
        : T[K] extends object
            ? string extends keyof T[K] ? K : K | `${K}.${SettingPath<T[K]>}`
            : K
}[keyof T & string]

/** The type of the setting at `P`. */
export type PathValue<T, P extends string> = P extends `${infer K}.${infer Rest}`
    ? K extends keyof T ? PathValue<T[K], Rest> : never
    : P extends keyof T ? T[P] : never

/** What a server may send for `T`, with fields left out or null at any depth. */
type Lenient<T> = T extends object ? {readonly [K in keyof T]?: Lenient<T[K]> | null} : T

function isPlainObject(value: unknown): value is Record<string, unknown> {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function mergeOnto<T extends object>(target: T, source: object): T {
    const fields = target as Record<string, unknown>
    for (const [key, value] of Object.entries(source)) {
        if (value === null || value === undefined) continue
        const current = fields[key]
        fields[key] = isPlainObject(value) && isPlainObject(current) ? mergeOnto(current, value) : value
    }
    return target
}

/**
 * A deep copy of a settings value. JSON, since settings are JSON off the wire and arrive
 * as reactive proxies, which `structuredClone` refuses.
 */
export function copySetting<T>(value: T): T {
    return value !== null && typeof value === 'object' ? JSON.parse(JSON.stringify(value)) : value
}

/**
 * The settings panel's editable copy of the server's settings: every group merged onto a
 * fresh `defaultSettings()`, so a field an older server leaves out (or sends as null)
 * shows its default, and no edit can reach the server's object or the shared defaults.
 */
export function settingsMirror(server: Lenient<Settings> | null | undefined): Settings {
    const mirror = defaultSettings()
    if (!server) return mirror
    return mergeOnto(mirror, copySetting(server))
}
