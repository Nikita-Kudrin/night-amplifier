import {reactive, readonly} from 'vue'
import {DEFAULT_SETTINGS} from '../constants'
import {settingsMirror} from './settings'

describe('settingsMirror', () => {
    it('is the defaults before the server answered', () => {
        expect(settingsMirror(null)).toEqual(DEFAULT_SETTINGS)
    })

    it('takes every value the server sent, and the default for one it left out or nulled', () => {
        const mirror = settingsMirror({bin: 2, auto_crop: null, rejection_sigma: undefined})

        expect(mirror.bin).toBe(2)
        expect(mirror.auto_crop).toBe(DEFAULT_SETTINGS.auto_crop)
        expect(mirror.rejection_sigma).toBe(DEFAULT_SETTINGS.rejection_sigma)
    })

    it('merges a group field by field', () => {
        const mirror = settingsMirror({eyepiece: {intensity: 0.5}, denoise: {ai: true}})

        expect(mirror.eyepiece).toEqual({...DEFAULT_SETTINGS.eyepiece, intensity: 0.5})
        expect(mirror.denoise).toEqual({...DEFAULT_SETTINGS.denoise, ai: true})
    })

    it('keeps groups the defaults do not know', () => {
        const profiles = {'Guide Cam': {focal_length_mm: 120}}
        expect(settingsMirror({camera_telescope_profiles: profiles}).camera_telescope_profiles).toEqual(profiles)
    })

    it('shares nothing with the server settings or the defaults', () => {
        const server = readonly(reactive({eyepiece: {intensity: 0.5}, raw_frame_saving: {stacking: true}}))
        const mirror = settingsMirror(server)

        mirror.eyepiece.intensity = 0.9
        mirror.raw_frame_saving.stacking = false
        mirror.denoise.enabled = false

        expect(server.eyepiece.intensity).toBe(0.5)
        expect(server.raw_frame_saving.stacking).toBe(true)
        expect(DEFAULT_SETTINGS.denoise.enabled).toBe(true)
    })
})
