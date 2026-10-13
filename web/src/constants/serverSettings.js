import {SERVER_DEFAULT_SETTINGS, SERVER_SETTING_RANGES} from '../composables/api.settings.js'

/** The server's default for a setting, by its dotted path in `Settings`. */
export function serverDefault(path) {
    return path.split('.').reduce((value, key) => value?.[key], SERVER_DEFAULT_SETTINGS)
}

/**
 * `{min, max, default}` of a setting the server clamps, read from the generated
 * `api.settings.js`: the range it enforces and the default it starts from. `ui` adds the
 * presentation (`step`) and may narrow the range to what the control offers, never widen
 * it (`serverSettings.test.js` checks).
 */
export function serverLimits(path, ui = {}) {
    const range = SERVER_SETTING_RANGES[path]
    if (!range) throw new Error(`the server clamps no setting named ${path}`)
    return Object.freeze({min: range.min, max: range.max, default: serverDefault(path), ...ui})
}
