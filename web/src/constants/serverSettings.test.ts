import {SERVER_DEFAULT_SETTINGS, SERVER_SETTING_RANGES} from '../composables/api.settings.js'
import type {ClampedPath} from '../composables/api.settings.js'
import type {Settings} from '../composables/api.types.js'
import type {SettingPath} from '../utils/settings.js'
import {serverDefault, serverLimits} from './serverSettings.js'
import {
    AUTO_STRETCH_INTENSITY,
    BACKGROUND_GRAIN_LIMITS,
    BLACK_FLOOR_LIMITS,
    BLACK_LEVEL_LIMITS,
    COOLER_TEMP_LIMITS,
    DEFAULT_SETTINGS,
    DENOISE_CHROMA_STRENGTH_LIMITS,
    DENOISE_LUMA_STRENGTH_LIMITS,
    DETAIL_LIMITS,
    DEW_HEATER_POWER_LIMITS,
    GAIN_LIMITS,
    HOT_PIXEL_SIGMA_LIMITS,
    REJECTION_SIGMA_LIMITS,
    SATURATION_BOOST_LIMITS,
    SIMULATED_PRELOAD_LIMITS,
} from './index.js'

/** Each control's range, by the setting the server clamps it as; every end a number. */
const BOUND: [ClampedPath, {min: number; max: number; default: unknown}][] = [
    ['rejection_sigma', REJECTION_SIGMA_LIMITS],
    ['auto_stretch_intensity', AUTO_STRETCH_INTENSITY],
    ['saturation_boost_strength', SATURATION_BOOST_LIMITS],
    ['simulated_preload_images', SIMULATED_PRELOAD_LIMITS],
    ['dew_heater_power', DEW_HEATER_POWER_LIMITS],
    ['sensor_correction.hot_pixel_sigma', HOT_PIXEL_SIGMA_LIMITS],
    ['denoise.chroma_strength', DENOISE_CHROMA_STRENGTH_LIMITS],
    ['denoise.background_grain', BACKGROUND_GRAIN_LIMITS],
    ['denoise.luma_strength', DENOISE_LUMA_STRENGTH_LIMITS],
    ['denoise.detail', DETAIL_LIMITS],
    ['eyepiece.black_floor', BLACK_FLOOR_LIMITS],
]

describe('server-owned numbers', () => {
    // A control reaching past the server's clamp would offer a value that silently becomes
    // another; a hand-kept default drifted from the server's before (B7).
    it.each(BOUND)('%s: the control stays inside the server’s range and starts at its default', (path, limits) => {
        const range = SERVER_SETTING_RANGES[path]
        expect(range.min === null || limits.min >= range.min).toBe(true)
        expect(range.max === null || limits.max <= range.max).toBe(true)
        expect(limits.default).toBe(serverDefault(path))
    })

    it('binds every range the server clamps to a control, or says why not', () => {
        // The guide camera's block reuses the imaging camera's controls; the cooler's
        // setpoint is clamped by the camera's own range, inside the server's.
        const unbound = ['target_temp_c', 'guide_camera.target_temp_c', 'guide_camera.dew_heater_power']
        const bound: string[] = BOUND.map(([path]) => path)
        expect(Object.keys(SERVER_SETTING_RANGES).filter((path) => !bound.includes(path)).sort()).toEqual(unbound.sort())
    })

    it('keeps the hand-picked ranges inside the server’s', () => {
        const temp = SERVER_SETTING_RANGES.target_temp_c
        expect(COOLER_TEMP_LIMITS.min).toBeGreaterThanOrEqual(temp.min)
        expect(COOLER_TEMP_LIMITS.max).toBeLessThanOrEqual(temp.max)
        expect(BLACK_LEVEL_LIMITS.default).toBe(SERVER_DEFAULT_SETTINGS.eyepiece.intensity)
        expect(GAIN_LIMITS.default).toBe(SERVER_DEFAULT_SETTINGS.gain)
    })

    it('takes the defaults from the server, frozen', () => {
        expect(DEFAULT_SETTINGS).toEqual(SERVER_DEFAULT_SETTINGS)
        expect(DEFAULT_SETTINGS).not.toBe(SERVER_DEFAULT_SETTINGS)
        expect(Object.isFrozen(DEFAULT_SETTINGS.eyepiece)).toBe(true)
    })

    it('reads a default by its dotted path, and refuses a range the server does not clamp', () => {
        expect(serverDefault('denoise.ai_compute')).toBe('auto')
        expectTypeOf(serverDefault('denoise.ai_compute')).toEqualTypeOf<Settings['denoise']['ai_compute']>()
        expect(serverLimits('rejection_sigma', {step: 0.1, max: 4})).toEqual({min: 0.5, max: 4, default: 2.5, step: 0.1})

        expectTypeOf(serverDefault).toBeCallableWith('guide_camera.target_temp_c')
        // @ts-expect-error: no such setting
        expectTypeOf(serverDefault).toBeCallableWith('no.such.key')
        // @ts-expect-error: the server does not clamp the gain
        expectTypeOf(serverLimits).toBeCallableWith('gain')
    })

    it('takes a path into a group of fixed keys, never into a map keyed by camera', () => {
        expectTypeOf<ClampedPath>().toExtend<SettingPath>()
        expectTypeOf<'guide_camera.dew_heater_power'>().toExtend<SettingPath>()
        expectTypeOf<'camera_profiles.x'>().not.toExtend<SettingPath>()
        expectTypeOf<'telescope.focal_length_mm.x'>().not.toExtend<SettingPath>()
    })

    it('reads an end the control narrows as a number, and one the server leaves open as null', () => {
        expectTypeOf(serverLimits('simulated_preload_images', {max: 20}).max).toEqualTypeOf<number>()
        expectTypeOf(serverLimits('simulated_preload_images').max).toEqualTypeOf<null>()
        expectTypeOf(serverLimits('rejection_sigma').default).toEqualTypeOf<number>()
    })
})
