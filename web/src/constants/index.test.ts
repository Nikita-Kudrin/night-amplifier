import * as constants from './index.js'
import {
    CAMERA_PHASES,
    CAPTURE_STATES,
    captureStateDisplay,
    DEFAULT_CAPABILITIES,
    DEFAULT_SETTINGS,
    defaultSettings,
    EYEPIECE_STREAM_RESOLUTION_OPTIONS,
    HELP_TEXTS,
} from './index.js'
import type {
    BACKGROUND_ALGORITHM_OPTIONS,
    CAPTURE_STATE_DISPLAY,
    PREVIEW_RESOLUTION_OPTIONS,
    RAW_FRAME_SAVING_MODES,
    REJECTION_METHOD_OPTIONS,
    STACKING_TYPES,
    STREAMING_RESOLUTION_OPTIONS,
    STRETCH_AGGRESSIVENESS_OPTIONS,
    WEIGHTING_PRESET_OPTIONS,
} from './index.js'
import {HELP_SECTIONS} from './help/index.js'
import type {CaptureState} from '../composables/api.events.js'
import type {
    BackgroundExtractionAlgorithm,
    CameraPhase,
    Capabilities,
    EyepieceStreamResolution,
    RawFrameSaving,
    RejectionMethod,
    Resolution,
    StackingType,
    StretchAggressiveness,
    WeightingPreset,
} from '../composables/api.types.js'

// Every domain module behind the barrel; `export *` silently drops a name two of them share.
// `serverSettings` and `tables` are the modules' own helpers, not constants.
const modules = import.meta.glob<Record<string, unknown>>(
    [
        './*.{js,ts}',
        '!./*.test.{js,ts}',
        '!./index.{js,ts}',
        '!./serverSettings.{js,ts}',
        '!./tables.{js,ts}',
        '!./cameras.{js,ts}',
    ],
    {eager: true},
)

/** The values an option table offers. */
type Offered<T extends readonly {value: unknown}[]> = T[number]['value']

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
        expectTypeOf<keyof typeof CAPTURE_STATE_DISPLAY>().toEqualTypeOf<CaptureState>()
        expect(captureStateDisplay('Teleporting')).toEqual({tone: 'idle', label: 'Idle'})
    })
})

// Type-level: `npm run typecheck` fails a value the server would refuse, or one it sends
// that a table leaves out.
describe('wire enums', () => {
    it('names every CameraPhase, CaptureState and StackingType the server sends, and no other', () => {
        expectTypeOf<(typeof CAMERA_PHASES)[keyof typeof CAMERA_PHASES]>().toEqualTypeOf<CameraPhase>()
        expectTypeOf<(typeof CAPTURE_STATES)[keyof typeof CAPTURE_STATES]>().toEqualTypeOf<CaptureState>()
        expectTypeOf<(typeof STACKING_TYPES)[keyof typeof STACKING_TYPES]>().toEqualTypeOf<StackingType>()
        expect(CAMERA_PHASES.WARMING_UP).toBe('warming_up')
        expect(CAPTURE_STATES.RECOVERING).toBe('Recovering')
    })
})

describe('option tables', () => {
    it('offers every value the server takes, and no other', () => {
        expectTypeOf<Offered<typeof STRETCH_AGGRESSIVENESS_OPTIONS>>().toEqualTypeOf<StretchAggressiveness>()
        expectTypeOf<Offered<typeof BACKGROUND_ALGORITHM_OPTIONS>>().toEqualTypeOf<BackgroundExtractionAlgorithm>()
        expectTypeOf<Offered<typeof WEIGHTING_PRESET_OPTIONS>>().toEqualTypeOf<WeightingPreset>()
        expectTypeOf<Offered<typeof PREVIEW_RESOLUTION_OPTIONS>>().toEqualTypeOf<Resolution>()
        expectTypeOf<Offered<typeof STREAMING_RESOLUTION_OPTIONS>>().toEqualTypeOf<Resolution>()
        expectTypeOf<Offered<typeof EYEPIECE_STREAM_RESOLUTION_OPTIONS>>().toEqualTypeOf<EyepieceStreamResolution>()
        expectTypeOf<(typeof RAW_FRAME_SAVING_MODES)[number]['key']>().toEqualTypeOf<keyof RawFrameSaving>()
    })

    // Min-Max needs per-pixel extremes the live stack does not keep; the server substitutes.
    it('offers every RejectionMethod but Min-Max', () => {
        expectTypeOf<Offered<typeof REJECTION_METHOD_OPTIONS>>().toEqualTypeOf<Exclude<RejectionMethod, 'MinMax'>>()
    })

    // The type above sees only the declared values: hd1080 must be gone at runtime too.
    it('leaves 1080p out of the lossless eyepiece stream', () => {
        expect(EYEPIECE_STREAM_RESOLUTION_OPTIONS.map((option) => option.value)).toEqual(['native', 'uhd2160', 'qhd1440'])
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
