import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import DewHeaterControl from './DewHeaterControl.vue'

vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal(), {
        updateSettings: vi.fn(),
    }))

import {updateSettings} from '../composables/api'
import {contextProvides} from '../composables/appContext'
import {camera, settings} from '../test/fixtures'
import type {CameraRole} from '../composables/api.types'
import type {VueWrapper} from '@vue/test-utils'

function provides(role: CameraRole = 'main') {
    const hardware = {dew_heater_enabled: true, dew_heater_power: 10}
    const base = settings(hardware)
    return {
        settings: ref(settings({...hardware, guide_camera: {...base.guide_camera, ...hardware}})),
        refreshSettings: vi.fn(async () => null),
        cameras: ref([camera({id: 'cam1', name: 'Dew Cam', info: {has_dew_heater: true}})]),
        selectedCamera: ref<string | null>('cam1'),
        selectedCameraRole: ref(role),
    }
}

function dragPower(wrapper: VueWrapper, value: number) {
    const slider = wrapper.findComponent({name: 'BaseSlider'})
    slider.vm.$emit('update:modelValue', value)
    slider.vm.$emit('change', value)
}

describe('DewHeaterControl', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(updateSettings).mockResolvedValue(settings())
    })

    // A slider release sends at once, like every other settings slider.
    it('sends the released power at once, for the edited camera', async () => {
        const wrapper = mount(DewHeaterControl, {global: {provide: contextProvides(provides('guide'))}})
        await flushPromises()

        dragPower(wrapper, 45)
        await flushPromises()

        expect(updateSettings).toHaveBeenCalledExactlyOnceWith({camera_role: 'guide', dew_heater_power: 45})
    })

    it('shows the selected camera\'s power', async () => {
        const provide = provides('main')
        provide.settings.value.guide_camera.dew_heater_power = 70
        const wrapper = mount(DewHeaterControl, {global: {provide: contextProvides(provide)}})
        await flushPromises()

        provide.selectedCameraRole.value = 'guide'
        await flushPromises()

        expect(wrapper.findComponent({name: 'BaseSlider'}).props('modelValue')).toBe(70)
    })
})
