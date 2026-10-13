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
import {
    BACKGROUND_EXTRACTION_ALGORITHM_VALUES,
    CAMERA_PHASE_VALUES,
    EYEPIECE_STREAM_RESOLUTION_VALUES,
    REJECTION_METHOD_VALUES,
    RESOLUTION_VALUES,
    STACKING_TYPE_VALUES,
    STRETCH_AGGRESSIVENESS_VALUES,
    WEIGHTING_PRESET_VALUES,
} from '../composables/api.types.js'
import type {Capabilities} from '../composables/api.types.js'

// Every domain module behind the barrel; `export *` silently drops a name two of them share.
const modules = import.meta.glob<Record<string, unknown>>(
    ['./*.{js,ts}', '!./*.test.{js,ts}', '!./index.{js,ts}', '!./serverSettings.{js,ts}', '!./cameras.{js,ts}'],
    {eager: true},
)

// It names only wire keys by construction: `structuredClone(SERVER_DEFAULT_SETTINGS)`, which
// `api.settings.ts` holds to `Settings`. (The fallback once mirrored `simulated_camera`, a key
// the server never sent.)
describe('DEFAULT_SETTINGS', () => {
    it('hands out copies a v-model cannot write through', () => {
        const copy = defaultSettings()
        copy.eyepiece.binoview = !DEFAULT_SETTINGS.eyepiece.binoview
        expect(defaultSettings().eyepiece.binoview).toBe(DEFAULT_SETTINGS.eyepiece.binoview)
    })
})

describe('DEFAULT_CAPABILITIES', () => {
    it('has the shape the server answers, with every Pro feature off', () => {
        expectTypeOf(DEFAULT_CAPABILITIES).toExtend<Capabilities>()
        expectTypeOf<keyof Capabilities>().toEqualTypeOf<keyof typeof DEFAULT_CAPABILITIES>()
        expectTypeOf<keyof Capabilities['deep_sky']>().toEqualTypeOf<keyof typeof DEFAULT_CAPABILITIES.deep_sky>()
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
        ['CameraPhase', CAMERA_PHASES, CAMERA_PHASE_VALUES],
        ['StackingType', STACKING_TYPES, STACKING_TYPE_VALUES],
    ])('names every %s the server sends, and no other', (_, table, wire) => {
        expect(Object.values(table).sort()).toEqual([...wire].sort())
    })
})

describe('option tables', () => {
    const values = (options: readonly {value: string}[]) => options.map((option) => option.value).sort()

    it.each([
        ['StretchAggressiveness', STRETCH_AGGRESSIVENESS_OPTIONS, STRETCH_AGGRESSIVENESS_VALUES],
        ['BackgroundExtractionAlgorithm', BACKGROUND_ALGORITHM_OPTIONS, BACKGROUND_EXTRACTION_ALGORITHM_VALUES],
        ['WeightingPreset', WEIGHTING_PRESET_OPTIONS, WEIGHTING_PRESET_VALUES],
        ['Resolution', PREVIEW_RESOLUTION_OPTIONS, RESOLUTION_VALUES],
        ['Resolution', STREAMING_RESOLUTION_OPTIONS, RESOLUTION_VALUES],
        ['EyepieceStreamResolution', EYEPIECE_STREAM_RESOLUTION_OPTIONS, EYEPIECE_STREAM_RESOLUTION_VALUES],
    ])('offers every %s the server takes, and no other', (_, options, wire) => {
        expect(values(options)).toEqual([...wire].sort())
    })

    // Min-Max needs per-pixel extremes the live stack does not keep; the server substitutes.
    it('offers every RejectionMethod but Min-Max', () => {
        expect(values(REJECTION_METHOD_OPTIONS)).toEqual(REJECTION_METHOD_VALUES.filter((m) => m !== 'MinMax').sort())
    })
})

describe('the constants barrel', () => {
    it('re-exports every domain module, none shadowing another', () => {
        const exported: Record<string, unknown> = constants
        const seen = new Map()
        for (const [file, module] of Object.entries(modules)) {
            for (const [name, value] of Object.entries(module)) {
                expect(seen.get(name), `${name} in ${file} and ${seen.get(name)}`).toBeUndefined()
                seen.set(name, file)
                expect(exported[name], name).toBe(value)
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
