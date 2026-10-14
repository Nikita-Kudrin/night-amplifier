import {reactive, readonly} from 'vue'
import {DEFAULT_SETTINGS} from '../constants'
import {settings} from '../test/fixtures'
import {settingsMirror} from './settings'

describe('settingsMirror', () => {
    it('is the defaults before the server answered', () => {
        expect(settingsMirror(null)).toEqual(DEFAULT_SETTINGS)
    })

    it('is the server settings as they are', () => {
        const server = settings({bin: 2, eyepiece: {intensity: 0.5}, camera_telescope_profiles: {'Guide Cam': {
            focal_length_mm: 120, pixel_size_x_um: null, pixel_size_y_um: null,
            sensor_width_px: null, sensor_height_px: null, barlow_coeff: null,
        }}})

        expect(settingsMirror(server)).toEqual(server)
    })

    it('shares nothing with the server settings or the defaults', () => {
        const server = readonly(reactive(settings({eyepiece: {intensity: 0.5}, raw_frame_saving: {stacking: true}})))
        const mirror = settingsMirror(server)
        const before = settingsMirror(null)

        mirror.eyepiece.intensity = 0.9
        mirror.raw_frame_saving.stacking = false
        before.denoise.enabled = false

        expect(server.eyepiece.intensity).toBe(0.5)
        expect(server.raw_frame_saving.stacking).toBe(true)
        expect(DEFAULT_SETTINGS.denoise.enabled).toBe(true)
    })
})
