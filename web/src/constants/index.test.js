import {DEFAULT_CAPABILITIES, DEFAULT_SETTINGS, defaultSettings} from './index.js'
import {wireTypes} from '../test/apiTypes.js'

const types = wireTypes()

/** The typedef a settings group is sent as, by the `Settings` property naming it. */
const GROUPS = {
    raw_frame_saving: 'RawFrameSaving',
    sensor_correction: 'SensorCorrectionSettings',
    denoise: 'DenoiseSettings',
    eyepiece: 'EyepieceSettings',
    telescope: 'TelescopeSettings',
}

describe('DEFAULT_SETTINGS', () => {
    // The fallback once mirrored `simulated_camera`, a key the server never sent.
    it('names only keys the server sends', () => {
        const unknown = Object.keys(DEFAULT_SETTINGS).filter((key) => !types.Settings.all.includes(key))
        expect(unknown).toEqual([])
    })

    it.each(Object.entries(GROUPS))('names only %s keys the server sends', (group, typedef) => {
        const unknown = Object.keys(DEFAULT_SETTINGS[group]).filter((key) => !types[typedef].all.includes(key))
        expect(unknown).toEqual([])
    })

    it('hands out copies a v-model cannot write through', () => {
        const copy = defaultSettings()
        copy.eyepiece.binoview = !DEFAULT_SETTINGS.eyepiece.binoview
        expect(defaultSettings().eyepiece.binoview).toBe(DEFAULT_SETTINGS.eyepiece.binoview)
    })
})

describe('DEFAULT_CAPABILITIES', () => {
    it('has the shape the server answers, with every Pro feature off', () => {
        expect(Object.keys(DEFAULT_CAPABILITIES).sort()).toEqual([...types.Capabilities.all].sort())
        expect(Object.keys(DEFAULT_CAPABILITIES.deep_sky).sort()).toEqual([...types.DeepSkyCapabilities.all].sort())
        const flags = JSON.stringify(DEFAULT_CAPABILITIES).match(/true/g)
        expect(flags).toBeNull()
    })
})
