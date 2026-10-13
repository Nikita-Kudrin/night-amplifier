/** The settings a UI falls back on before (or without) the server's answer. */
import type {DeepReadonly} from 'vue'
import {SERVER_DEFAULT_SETTINGS} from '../composables/api.settings.js'
import type {Settings} from '../composables/api.types.js'

function deepFreeze<T extends object>(value: T): DeepReadonly<T> {
    for (const child of Object.values(value)) {
        if (child !== null && typeof child === 'object') deepFreeze(child)
    }
    return Object.freeze(value) as DeepReadonly<T>
}

/**
 * `CaptureSettings::default()` as `/api/settings` sends it, generated into `api.settings.ts`.
 * The hand copy this replaced had drifted from the server twice (a key it never sent, two
 * defaults missing). Frozen: take `defaultSettings()` for a copy to edit.
 */
export const DEFAULT_SETTINGS = deepFreeze<Settings>(structuredClone(SERVER_DEFAULT_SETTINGS))

/**
 * A fresh copy of the defaults. `DEFAULT_SETTINGS` has nested objects, so spreading it
 * hands out references into the shared constant: bind one to a v-model and the first
 * click would edit the defaults every later fallback reads.
 */
export function defaultSettings(): Settings {
    return structuredClone(DEFAULT_SETTINGS)
}
