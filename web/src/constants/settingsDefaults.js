/** The settings a UI falls back on before (or without) the server's answer. */
import {SERVER_DEFAULT_SETTINGS} from '../composables/api.settings.js'

function deepFreeze(value) {
    if (value === null || typeof value !== 'object') return value
    Object.values(value).forEach(deepFreeze)
    return Object.freeze(value)
}

/**
 * `CaptureSettings::default()` as `/api/settings` sends it, generated into `api.settings.js`.
 * The hand copy this replaced had drifted from the server twice (a key it never sent, two
 * defaults missing). Frozen: take `defaultSettings()` for a copy to edit.
 * @type {Readonly<import('../composables/api.types.js').Settings>}
 */
export const DEFAULT_SETTINGS = deepFreeze(structuredClone(SERVER_DEFAULT_SETTINGS))

/**
 * A fresh copy of the defaults. `DEFAULT_SETTINGS` has nested objects, so spreading it
 * hands out references into the shared constant: bind one to a v-model and the first
 * click would edit the defaults every later fallback reads.
 */
export function defaultSettings() {
    return structuredClone(DEFAULT_SETTINGS)
}
