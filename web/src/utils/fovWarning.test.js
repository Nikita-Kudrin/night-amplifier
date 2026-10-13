import {fovWarning} from './fovWarning.js'

const D80 = {id: 'D80', min_fov_deg: 0.15, max_fov_deg: 6, installed: true}
const D05 = {id: 'D05', min_fov_deg: 0.6, max_fov_deg: 10, installed: true}
const W08 = {id: 'W08', min_fov_deg: 20, max_fov_deg: 180, installed: false}
const G05 = {id: 'G05', min_fov_deg: 6, max_fov_deg: 30, installed: false}

const warn = (fovDeg, available = [D80, D05, W08, G05]) =>
    fovWarning({fovDeg, activeId: 'D80', installed: [D80, D05], available})

describe('fovWarning', () => {
    it('says nothing inside the active database\'s range, or without a field or database', () => {
        expect(warn(1.5)).toBeNull()
        expect(warn(null)).toBeNull()
        expect(fovWarning({fovDeg: 1, activeId: null})).toBeNull()
        expect(fovWarning({fovDeg: 1, activeId: 'D80', installed: []})).toBeNull()
    })

    it('names a field too narrow for the database', () => {
        const warning = warn(0.1)

        expect(warning.database_id).toBe('D80')
        expect(warning.message).toContain('is too narrow for the D80 database (0.15°–6°)')
        expect(warning.suggestion).toBeNull()
    })

    it('suggests switching to an installed database and downloading another', () => {
        const warning = warn(8)

        expect(warning.message).toContain('too wide')
        expect(warning.suggestion).toBe('Switch to D05 or download G05')
    })

    it('suggests a download alone when nothing installed covers it', () => {
        expect(warn(25).suggestion).toBe('download W08, G05')
    })
})
