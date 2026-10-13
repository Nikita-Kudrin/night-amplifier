import * as constants from './index.js'
import {
    BACKGROUND_ALGORITHM_OPTIONS,
    CAMERA_PHASES,
    CAPTURE_STATE_DISPLAY,
    CAPTURE_STATES,
    captureStateDisplay,
    DEFAULT_CAPABILITIES,
    DEFAULT_SETTINGS,
    defaultSettings,
    EYEPIECE_STREAM_RESOLUTION_OPTIONS,
    HELP_TEXTS,
    PREVIEW_RESOLUTION_OPTIONS,
    REJECTION_METHOD_OPTIONS,
    STACKING_TYPES,
    STREAMING_RESOLUTION_OPTIONS,
    STRETCH_AGGRESSIVENESS_OPTIONS,
    WEIGHTING_PRESET_OPTIONS,
} from './index.js'
import {HELP_SECTIONS} from './help/index.js'
import {wireEnum, wireTypes} from '../test/apiTypes.js'

// Every domain module behind the barrel; `export *` silently drops a name two of them share.
const modules = import.meta.glob(['./*.js', '!./*.test.js', '!./index.js', '!./serverSettings.js', '!./cameras.js'], {eager: true})

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

describe('captureStateDisplay', () => {
    it('reads a capture paused for a reconnect as still capturing', () => {
        expect(captureStateDisplay(CAPTURE_STATES.RECOVERING)).toEqual(captureStateDisplay(CAPTURE_STATES.CAPTURING))
    })

    it('has an entry for every capture state, and shows an unknown one as Idle', () => {
        for (const state of Object.values(CAPTURE_STATES)) expect(CAPTURE_STATE_DISPLAY[state], state).toBeTruthy()
        expect(captureStateDisplay('Teleporting')).toEqual({tone: 'idle', label: 'Idle'})
    })
})

describe('wire enums', () => {
    it.each([
        ['CameraPhase', CAMERA_PHASES],
        ['StackingType', STACKING_TYPES],
    ])('names every %s the server sends, and no other', (typedef, table) => {
        expect(Object.values(table).sort()).toEqual(wireEnum(typedef).sort())
    })
})

describe('option tables', () => {
    const values = (options) => options.map((option) => option.value).sort()

    it.each([
        ['StretchAggressiveness', STRETCH_AGGRESSIVENESS_OPTIONS],
        ['BackgroundExtractionAlgorithm', BACKGROUND_ALGORITHM_OPTIONS],
        ['WeightingPreset', WEIGHTING_PRESET_OPTIONS],
        ['Resolution', PREVIEW_RESOLUTION_OPTIONS],
        ['Resolution', STREAMING_RESOLUTION_OPTIONS],
        ['EyepieceStreamResolution', EYEPIECE_STREAM_RESOLUTION_OPTIONS],
    ])('offers every %s the server takes, and no other', (typedef, options) => {
        expect(values(options)).toEqual(wireEnum(typedef).sort())
    })

    // Min-Max needs per-pixel extremes the live stack does not keep; the server substitutes.
    it('offers every RejectionMethod but Min-Max', () => {
        expect(values(REJECTION_METHOD_OPTIONS)).toEqual(wireEnum('RejectionMethod').filter((m) => m !== 'MinMax').sort())
    })
})

describe('the constants barrel', () => {
    it('re-exports every domain module, none shadowing another', () => {
        const seen = new Map()
        for (const [file, module] of Object.entries(modules)) {
            for (const [name, value] of Object.entries(module)) {
                expect(seen.get(name), `${name} in ${file} and ${seen.get(name)}`).toBeUndefined()
                seen.set(name, file)
                expect(constants[name], name).toBe(value)
            }
        }
        expect(seen.size).toBeGreaterThan(40)
    })
})

describe('HELP_TEXTS', () => {
    it('has each key in one panel’s file only', () => {
        const keys = Object.values(HELP_SECTIONS).flatMap((section) => Object.keys(section))
        expect(keys.length).toBe(new Set(keys).size)
        expect(Object.keys(HELP_TEXTS)).toHaveLength(keys.length)
    })

    it('has text for every key', () => {
        for (const [key, text] of Object.entries(HELP_TEXTS)) {
            expect(typeof text === 'string' && text.trim().length > 0, key).toBe(true)
        }
    })
})
