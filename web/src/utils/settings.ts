import {defaultSettings} from '../constants'
import type {DeepReadonly} from 'vue'
import type {EyepieceSettings, Settings, UpdateSettingsRequest} from '../composables/api.types'

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

/** A setting a settings section edits: one the panel's mirror holds and the server takes. */
export type SettingKey = keyof Settings & keyof UpdateSettingsRequest

/** The fields of the `eyepiece` group a section changed. */
export type EyepieceFields = Partial<EyepieceSettings>

/**
 * What a section hands over for each setting: its value as the mirror holds it, except the
 * `eyepiece` group. The server replaces that group whole and two sections edit it, so each
 * sends only the fields it changed, which the panel lays over its own copy.
 */
export type SectionValues = {
    [K in SettingKey]-?: K extends 'eyepiece' ? EyepieceFields : Settings[K]
}

/** A settings section's `apply(key, value)`: a key with its own value, never another key's. */
export type SettingApply = {[K in SettingKey]-?: [key: K, value: SectionValues[K]]}[SettingKey]

/** Persists a setting a section edited: what a section that must await the save takes as a prop. */
export type SaveSetting = (...args: SettingApply) => Promise<void>

/**
 * `apply`'s arguments for `key`, with a `copy` of its value in `values` (a section's
 * mirror). For a key typed as any of a section's keys, which `SettingApply` cannot pair
 * with a value: the value is read for the key, so the two cannot come apart.
 */
export function applyArgs<T extends Partial<Settings>, K extends keyof T & SettingKey>(
    values: T,
    key: K,
    copy: (value: T[K]) => T[K] = copySetting,
): SettingApply {
    return [key, copy(values[key])] as SettingApply
}

/**
 * A deep copy of a settings value. JSON, since settings are JSON off the wire and arrive
 * as reactive proxies, which `structuredClone` refuses.
 */
export function copySetting<T>(value: T): T {
    return value !== null && typeof value === 'object' ? JSON.parse(JSON.stringify(value)) : value
}

/**
 * The settings panel's editable copy of the server's settings, the defaults before the
 * first answer: no edit can reach the server's object or the shared defaults.
 */
export function settingsMirror(server: DeepReadonly<Settings> | null | undefined): Settings {
    // A deep copy, so nothing of it is read-only any more.
    return server ? copySetting(server as Settings) : defaultSettings()
}
