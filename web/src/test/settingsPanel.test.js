import {createDefaultSettings} from './settingsPanel.js'
import {wireTypes} from './apiTypes.js'

const types = wireTypes()

describe('createDefaultSettings', () => {
    // It once listed a few keys by hand and missed `raw_frame_saving.guide`.
    it('carries every Settings field the server sends', () => {
        const built = createDefaultSettings()
        expect(types.Settings.required.filter((key) => !(key in built))).toEqual([])
        expect(Object.keys(built.raw_frame_saving).sort()).toEqual([...types.RawFrameSaving.all].sort())
    })

    it('hands out fresh objects', () => {
        createDefaultSettings().raw_frame_saving.guide = true
        expect(createDefaultSettings().raw_frame_saving.guide).toBe(false)
    })
})
