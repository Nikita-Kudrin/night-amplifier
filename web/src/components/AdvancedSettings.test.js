import {flushPromises, mount} from '@vue/test-utils'
import {computed, nextTick, ref} from 'vue'
import AdvancedSettings from './AdvancedSettings.vue'
import SettingsPanel from './SettingsPanel.vue'
import {settingsMirror} from '../utils/settings.js'
import {findToggleByLabel} from '../test/settingsPanel.js'
import {mountWithDefaults} from '../test/mount.js'
import {settings} from '../test/fixtures.js'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {updateSettings} from '../composables/api.js'
import {contextProvides} from '../composables/appContext.js'

function mountSection(server = {}, simulatorEnabled = ref(false)) {
    const wrapper = mount(AdvancedSettings, {
        props: {settings: settingsMirror(server), save: vi.fn(async () => {}), guard: vi.fn((op) => op())},
        global: {provide: contextProvides({simulatorEnabled}), stubs: {BaseProLock: true}},
    })
    return {wrapper, simulatorEnabled}
}

const binButton = (wrapper, label) => wrapper.findAll('.btn-option').find((b) => b.text() === label)

describe('AdvancedSettings', () => {
    beforeEach(() => vi.clearAllMocks())

    it('sends a binning choice for the selected camera', async () => {
        const {wrapper} = mountSection({bin: 1})

        await binButton(wrapper, '2x2').trigger('click')

        expect(updateSettings).toHaveBeenCalledExactlyOnceWith({camera_role: 'main', bin: 2})
        expect(wrapper.emitted('apply')).toBeUndefined()
    })

    // `bin` is per camera: the server writes it to the camera `camera_role` names, and the
    // guide keeps its own in `guide_camera.bin`. Shown and posted as the top-level one, a
    // click with the guide selected rebinned (and reset the stack of) the imaging camera.
    it('shows and edits the selected guide camera\'s binning, naming it', async () => {
        const applySettings = vi.fn(async () => settings())
        const wrapper = mountWithDefaults(SettingsPanel, {
            provide: {
                settings: settings({bin: 1, guide_camera: {...settings().guide_camera, bin: 2}}),
                selectedCameraRole: computed(() => 'guide'),
                applySettings,
            },
        })

        expect(binButton(wrapper, '2x2').classes()).toContain('active')

        await binButton(wrapper, '4x4').trigger('click')
        await flushPromises()
        expect(applySettings).toHaveBeenCalledExactlyOnceWith({camera_role: 'guide', bin: 4})
        expect(binButton(wrapper, '4x4').classes()).toContain('active')
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
