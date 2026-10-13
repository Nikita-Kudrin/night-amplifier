import {createDefaultSettings} from './settingsPanel.js'
import type {Settings} from '../composables/api.types.js'

describe('createDefaultSettings', () => {
    // It once listed a few keys by hand and missed `raw_frame_saving.guide`.
    it('carries every Settings field the server sends', () => {
        expectTypeOf(createDefaultSettings()).toEqualTypeOf<Settings>()
    })

    it('hands out fresh objects', () => {
        createDefaultSettings().raw_frame_saving.guide = true
        expect(createDefaultSettings().raw_frame_saving.guide).toBe(false)
    })
})
