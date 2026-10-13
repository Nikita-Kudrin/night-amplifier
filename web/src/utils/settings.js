import {defaultSettings} from '../constants'

function isPlainObject(value) {
    return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function mergeOnto(target, source) {
    for (const [key, value] of Object.entries(source)) {
        if (value === null || value === undefined) continue
        target[key] = isPlainObject(value) && isPlainObject(target[key]) ? mergeOnto(target[key], value) : value
    }
    return target
}

/**
 * A deep copy of a settings value. JSON, since settings are JSON off the wire and arrive
 * as reactive proxies, which `structuredClone` refuses.
 */
export function copySetting(value) {
    return value !== null && typeof value === 'object' ? JSON.parse(JSON.stringify(value)) : value
}

/**
 * The settings panel's editable copy of the server's settings: every group merged onto a
 * fresh `defaultSettings()`, so a field an older server leaves out (or sends as null)
 * shows its default, and no edit can reach the server's object or the shared defaults.
 */
export function settingsMirror(server) {
    const mirror = defaultSettings()
    if (!server) return mirror
    return mergeOnto(mirror, copySetting(server))
}
