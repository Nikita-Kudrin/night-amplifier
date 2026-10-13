import {SERVER_DEFAULT_SETTINGS, SERVER_SETTING_RANGES} from '../composables/api.settings.js'
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

/** Each control's range, by the setting the server clamps it as. */
const BOUND = [
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
        expect(limits.min).toBeTypeOf('number')
        expect(limits.max).toBeTypeOf('number')
        expect(limits.default).toBe(serverDefault(path))
    })

    it('binds every range the server clamps to a control, or says why not', () => {
        // The guide camera's block reuses the imaging camera's controls; the cooler's
        // setpoint is clamped by the camera's own range, inside the server's.
        const unbound = ['target_temp_c', 'guide_camera.target_temp_c', 'guide_camera.dew_heater_power']
        const bound = BOUND.map(([path]) => path)
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
        expect(serverDefault('no.such.key')).toBeUndefined()
        expect(() => serverLimits('gain')).toThrow('the server clamps no setting named gain')
        expect(serverLimits('rejection_sigma', {step: 0.1, max: 4})).toEqual({min: 0.5, max: 4, default: 2.5, step: 0.1})
    })
})
