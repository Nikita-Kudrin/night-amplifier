import {assert} from 'vitest'
import type {DatabaseType, InstalledDatabaseInfo} from '../composables/api.types.js'
import {databaseType} from '../test/fixtures.js'
import {fovWarning} from './fovWarning.js'

const D80 = databaseType({id: 'D80', min_fov_deg: 0.15, max_fov_deg: 6, installed: true})
const D05 = databaseType({id: 'D05', min_fov_deg: 0.6, max_fov_deg: 10, installed: true})
const W08 = databaseType({id: 'W08', min_fov_deg: 20, max_fov_deg: 180, installed: false})
const G05 = databaseType({id: 'G05', min_fov_deg: 6, max_fov_deg: 30, installed: false})

/** How `AstapStatus.installed_databases` lists a database. */
const installed = ({id, min_fov_deg, max_fov_deg}: DatabaseType): InstalledDatabaseInfo => ({
    id,
    database_path: `/opt/astap/${id.toLowerCase()}`,
    min_fov_deg,
    max_fov_deg,
})

const warn = (fovDeg: number | null, available = [D80, D05, W08, G05]) =>
    fovWarning({fovDeg, activeId: 'D80', installed: [D80, D05].map(installed), available})

describe('fovWarning', () => {
    it('says nothing inside the active database\'s range, or without a field or database', () => {
        expect(warn(1.5)).toBeNull()
        expect(warn(null)).toBeNull()
        expect(fovWarning({fovDeg: 1, activeId: null})).toBeNull()
        expect(fovWarning({fovDeg: 1, activeId: 'D80', installed: []})).toBeNull()
    })

    it('names a field too narrow for the database', () => {
        const warning = warn(0.1)

        assert(warning)
        expect(warning.database_id).toBe('D80')
        expect(warning.message).toContain('is too narrow for the D80 database (0.15°–6°)')
        expect(warning.suggestion).toBeNull()
    })

    it('suggests switching to an installed database and downloading another', () => {
        const warning = warn(8)

        assert(warning)
        expect(warning.message).toContain('too wide')
        expect(warning.suggestion).toBe('Switch to D05 or download G05')
    })

    it('suggests a download alone when nothing installed covers it', () => {
        expect(warn(25)?.suggestion).toBe('download W08, G05')
    })
})
