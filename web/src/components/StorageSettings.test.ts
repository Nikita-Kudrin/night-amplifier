import {ref} from 'vue'
import {mount} from '@vue/test-utils'
import StorageSettings from './StorageSettings.vue'
import ReconnectSettings from './ReconnectSettings.vue'
import {contextProvides} from '../composables/appContext'
import {capabilities, settings} from '../test/fixtures'
import type {SettingsOverrides} from '../test/fixtures'
import {findToggleByLabel} from '../test/settingsPanel'

describe('StorageSettings', () => {
    function mountSection(
        server: SettingsOverrides = {},
        props: {hasGuideCamera?: boolean} = {},
        debugLogging = true,
    ) {
        return mount(StorageSettings, {
            props: {settings: settings(server), ...props},
            global: {provide: contextProvides({capabilities: ref(capabilities({debug_logging: debugLogging}))})},
        })
    }

    it('offers the guide camera\'s switch only with one attached', () => {
        expect(() => findToggleByLabel(mountSection(), 'Guide')).toThrow()
        expect(findToggleByLabel(mountSection({}, {hasGuideCamera: true}), 'Guide')).toBeTruthy()
    })

    // The server saves guide frames only in debug mode, so the switch is offered only then.
    it('offers the guide camera\'s switch only in debug mode', () => {
        expect(() => findToggleByLabel(mountSection({}, {hasGuideCamera: true}, false), 'Guide')).toThrow()
        expect(findToggleByLabel(mountSection({}, {hasGuideCamera: true}, true), 'Guide')).toBeTruthy()
    })

    // The server ignores a guide switch outside debug mode rather than clearing it; the
    // group is sent whole, so editing a shown switch must not clear the hidden one.
    it('keeps a hidden guide switch when another raw-frame switch is sent', async () => {
        const wrapper = mountSection(
            {raw_frame_saving: {live_view: false, wanderer: false, stacking: false, guide: true}},
            {hasGuideCamera: true},
            false,
        )

        await findToggleByLabel(wrapper, 'Stacking').setValue(true)

        expect(wrapper.emitted('apply')).toEqual([
            ['raw_frame_saving', {live_view: false, wanderer: false, stacking: true, guide: true}],
        ])
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
        const wrapper = mount(ReconnectSettings, {props: {settings: settings({auto_reconnect: false})}})
        expect(() => findToggleByLabel(wrapper, 'Resume the capture')).toThrow()

        await findToggleByLabel(wrapper, 'Reconnect automatically').setValue(true)
        await findToggleByLabel(wrapper, 'Resume the capture').setValue(false)

        expect(wrapper.emitted('apply')).toEqual([['auto_reconnect', true], ['auto_resume_capture', false]])
    })
})
