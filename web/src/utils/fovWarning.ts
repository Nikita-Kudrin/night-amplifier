import type {DatabaseType, InstalledDatabaseInfo} from '../composables/api.types'
import {formatFov} from './format'

type FovRange = Pick<DatabaseType, 'min_fov_deg' | 'max_fov_deg'>

export interface FovWarning {
    database_id: string
    current_fov: string
    message: string
    suggestion: string | null
}

/**
 * Whether the scope's field of view lies outside the active ASTAP database's range, and
 * which databases would cover it: the installed ones to switch to, the others to
 * download. ASTAP mostly fails to solve outside its database's range.
 *
 * `fovDeg` is the field's height, `activeId` the database the solver uses, `installed`
 * `AstapStatus.installed_databases` and `available` every database.
 */
export function fovWarning({fovDeg, activeId, installed = [], available = []}: {
    fovDeg: number | null
    activeId: string | null
    installed?: readonly InstalledDatabaseInfo[]
    available?: readonly DatabaseType[]
}): FovWarning | null {
    if (!activeId || !fovDeg) return null
    const active = installed.find((db) => db.id === activeId)
    if (!active) return null
    const covers = (db: FovRange) => fovDeg >= db.min_fov_deg && fovDeg <= db.max_fov_deg
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
