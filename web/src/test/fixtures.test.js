import * as fixtures from './fixtures.js'
import {wireEnum, wireTypes} from './apiTypes.js'

const types = wireTypes()

/** Each builder against the typedef it stands in for. */
const BUILDERS = [
    ['settings', 'Settings'],
    ['capabilities', 'Capabilities'],
    ['proCapabilities', 'Capabilities'],
    ['camera', 'Camera'],
    ['cameraInfo', 'CameraInfo'],
    ['astapStatus', 'AstapStatus'],
    ['catalogStatus', 'CatalogStatus'],
    ['databaseType', 'DatabaseType'],
    ['pushToStatus', 'PushToStatus'],
    ['catalogEntry', 'CatalogEntry'],
    ['stackingTypeInfo', 'StackingTypeInfo'],
]

describe('fixtures', () => {
    it.each(BUILDERS)('%s() names only %s fields', (builder, typedef) => {
        const unknown = Object.keys(fixtures[builder]()).filter((key) => !types[typedef].all.includes(key))
        expect(unknown).toEqual([])
    })

    it.each(BUILDERS)('%s() carries every required %s field', (builder, typedef) => {
        const built = fixtures[builder]()
        expect(types[typedef].required.filter((key) => !(key in built))).toEqual([])
    })

    it('describes installed databases as the server does', () => {
        for (const entry of fixtures.astapStatus().installed_databases) {
            expect(Object.keys(entry).sort()).toEqual([...types.InstalledDatabaseInfo.all].sort())
        }
    })

    it('lists the stacking types as the server does, one per type', () => {
        expect(fixtures.stackingTypes().map(({id}) => id)).toEqual(wireEnum('StackingType'))
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
