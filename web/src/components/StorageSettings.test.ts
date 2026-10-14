import {mount} from '@vue/test-utils'
import StorageSettings from './StorageSettings.vue'
import ReconnectSettings from './ReconnectSettings.vue'
import {settingsMirror} from '../utils/settings'
import {findToggleByLabel} from '../test/settingsPanel'

describe('StorageSettings', () => {
    function mountSection(server: Parameters<typeof settingsMirror>[0] = {}, props: {hasGuideCamera?: boolean} = {}) {
        return mount(StorageSettings, {props: {settings: settingsMirror(server), ...props}})
    }

    it('offers the guide camera\'s switch only with one attached', () => {
        expect(() => findToggleByLabel(mountSection(), 'Guide')).toThrow()
        expect(findToggleByLabel(mountSection({}, {hasGuideCamera: true}), 'Guide')).toBeTruthy()
    })

    it('offers the stacked image only while Stacking runs without Wanderer', () => {
        expect(findToggleByLabel(mountSection({stacking: true, wanderer_mode: false}), 'Save Stacked Image')).toBeTruthy()
        expect(() => findToggleByLabel(mountSection({stacking: true, wanderer_mode: true}), 'Save Stacked Image')).toThrow()
        expect(() => findToggleByLabel(mountSection({stacking: false}), 'Save Stacked Image')).toThrow()
    })

    it('sends a raw-frame switch with the rest of its group', async () => {
        const wrapper = mountSection({raw_frame_saving: {live_view: true, wanderer: false, stacking: false, guide: false}})

        await findToggleByLabel(wrapper, 'Stacking').setValue(true)

        expect(wrapper.emitted('apply')).toEqual([
            ['raw_frame_saving', {live_view: true, wanderer: false, stacking: true, guide: false}],
        ])
    })
})

describe('ReconnectSettings', () => {
    it('offers resuming only while reconnecting is on, and sends each switch', async () => {
        const wrapper = mount(ReconnectSettings, {props: {settings: settingsMirror({auto_reconnect: false})}})
        expect(() => findToggleByLabel(wrapper, 'Resume the capture')).toThrow()

        await findToggleByLabel(wrapper, 'Reconnect automatically').setValue(true)
        await findToggleByLabel(wrapper, 'Resume the capture').setValue(false)

        expect(wrapper.emitted('apply')).toEqual([['auto_reconnect', true], ['auto_resume_capture', false]])
    })
})
