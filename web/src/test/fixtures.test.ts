import * as fixtures from './fixtures'
import {STACKING_TYPE_VALUES} from '../composables/api.types'

// Which fields each builder names and carries is its return type's to say (`fixtures.ts`).
describe('fixtures', () => {
    it('lists the stacking types as the server does, one per type', () => {
        expect(fixtures.stackingTypes().map(({id}) => id)).toEqual(STACKING_TYPE_VALUES)
    })

    it('keeps a camera and its info naming the same body', () => {
        const built = fixtures.camera({id: 'qhy_1', name: 'QHY5III462C', info: {has_cooler: true}})
        expect(built.info).toMatchObject({id: 'qhy_1', name: 'QHY5III462C', has_cooler: true})
    })

    // A test names only the fields of a group it is about; the rest are the server's defaults.
    it('lays a partial group over its default, and any other value whole', () => {
        const defaults = fixtures.settings()
        const built = fixtures.settings({bin: 2, eyepiece: {intensity: 0.5}, comet_roi: {x: 1, y: 2, width: 3, height: 4}})

        expect(built.bin).toBe(2)
        expect(built.eyepiece).toEqual({...defaults.eyepiece, intensity: 0.5})
        expect(built.comet_roi).toEqual({x: 1, y: 2, width: 3, height: 4})
        expect(built.denoise).toEqual(defaults.denoise)
    })

    it('hands out fresh objects', () => {
        fixtures.settings().eyepiece.intensity = 0.9
        expect(fixtures.settings().eyepiece.intensity).not.toBe(0.9)
        fixtures.capabilities().deep_sky.denoise = true
        expect(fixtures.capabilities().deep_sky.denoise).toBe(false)
    })
})
