import * as fixtures from './fixtures.js'
import {STACKING_TYPE_VALUES} from '../composables/api.types.js'

// Which fields each builder names and carries is its return type's to say (`fixtures.ts`).
describe('fixtures', () => {
    it('lists the stacking types as the server does, one per type', () => {
        expect(fixtures.stackingTypes().map(({id}) => id)).toEqual(STACKING_TYPE_VALUES)
    })

    it('keeps a camera and its info naming the same body', () => {
        const built = fixtures.camera({id: 'qhy_1', name: 'QHY5III462C', info: {has_cooler: true}})
        expect(built.info).toMatchObject({id: 'qhy_1', name: 'QHY5III462C', has_cooler: true})
    })

    it('hands out fresh objects', () => {
        fixtures.capabilities().deep_sky.denoise = true
        expect(fixtures.capabilities().deep_sky.denoise).toBe(false)
    })
})
