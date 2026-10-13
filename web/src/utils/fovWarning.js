import {formatFov} from './format.js'

/**
 * Whether the scope's field of view lies outside the active ASTAP database's range, and
 * which databases would cover it: the installed ones to switch to, the others to
 * download. ASTAP mostly fails to solve outside its database's range.
 *
 * @param {Object} options
 * @param {number|null} options.fovDeg - The field's height, in degrees
 * @param {string|null} options.activeId - The database the solver uses
 * @param {Object[]} options.installed - `AstapStatus.installed_databases`
 * @param {Object[]} options.available - Every database, with `installed`
 * @returns {null|{database_id: string, current_fov: string, message: string, suggestion: string|null}}
 */
export function fovWarning({fovDeg, activeId, installed = [], available = []}) {
    if (!activeId || !fovDeg) return null
    const active = installed.find((db) => db.id === activeId)
    if (!active) return null
    const covers = (db) => fovDeg >= db.min_fov_deg && fovDeg <= db.max_fov_deg
    if (covers(active)) return null

    const fov = formatFov(fovDeg)
    const direction = fovDeg < active.min_fov_deg ? 'too narrow' : 'too wide'
    const message = `FOV ${fov} is ${direction} for the ${active.id} database ` +
        `(${active.min_fov_deg}°–${active.max_fov_deg}°). Plate solving may fail with this setup.`

    const matching = available.filter((db) => db.id !== active.id && covers(db))
    const switchTo = matching.filter((db) => db.installed).map((db) => db.id)
    const download = matching.filter((db) => !db.installed).map((db) => db.id)
    const parts = []
    if (switchTo.length) parts.push(`Switch to ${switchTo.join(', ')}`)
    if (download.length) parts.push(`download ${download.join(', ')}`)

    return {database_id: active.id, current_fov: fov, message, suggestion: parts.length ? parts.join(' or ') : null}
}
