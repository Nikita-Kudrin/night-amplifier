import {mount} from '@vue/test-utils'
import {nextTick, ref} from 'vue'
import AdvancedSettings from './AdvancedSettings.vue'
import {settingsMirror} from '../utils/settings.js'
import {findToggleByLabel} from '../test/settingsPanel.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

function mountSection(server = {}, simulatorEnabled = ref(false)) {
    const wrapper = mount(AdvancedSettings, {
        props: {settings: settingsMirror(server), save: vi.fn(async () => {}), guard: vi.fn((op) => op())},
        global: {provide: {simulatorEnabled}, stubs: {BaseProLock: true}},
    })
    return {wrapper, simulatorEnabled}
}

describe('AdvancedSettings', () => {
    it('sends a binning choice', async () => {
        const {wrapper} = mountSection({bin: 1})

        await wrapper.findAll('.btn-group button, button').find((b) => b.text().includes('2x2')).trigger('click')

        expect(wrapper.emitted('apply')).toEqual([['bin', 2]])
    })

    it('switches the simulator in the app and on the server', async () => {
        const {wrapper, simulatorEnabled} = mountSection()

        await findToggleByLabel(wrapper, 'Simulated Camera').setValue(true)

        expect(simulatorEnabled.value).toBe(true)
        expect(wrapper.emitted('apply')).toEqual([['use_simulated_camera', true]])
    })

    // It used to wait 300 ms; every settings slider now sends on release.
    it('sends the preload count on release, at once', async () => {
        vi.useFakeTimers()
        try {
            const {wrapper} = mountSection({}, ref(true))
            const preload = wrapper.findAllComponents({name: 'BaseSlider'}).find((s) => s.props('label') === 'Preload Count')

            preload.vm.$emit('update:modelValue', 8)
            preload.vm.$emit('change', 8)
            await nextTick()

            expect(wrapper.emitted('apply')).toEqual([['simulated_preload_images', 8]])
        } finally {
            vi.useRealTimers()
        }
    })

    it('offers "Now" only while the focus image is on', async () => {
        const {wrapper} = mountSection({show_focus_image: false})
        expect(() => findToggleByLabel(wrapper, 'Now')).toThrow()

        await findToggleByLabel(wrapper, 'Show focus image').setValue(true)

        expect(wrapper.emitted('apply')).toEqual([['show_focus_image', true]])
        await findToggleByLabel(wrapper, 'Now').setValue(true)
        expect(wrapper.emitted('apply')[1]).toEqual(['force_focus_image_now', true])
    })
})
