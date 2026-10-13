import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {h, ref} from 'vue'
import DewHeaterControl from './DewHeaterControl.vue'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
        updateSettings: vi.fn(),
    }))

import {updateSettings} from '../composables/api.js'

function provides(role = 'main') {
    const hardware = {dew_heater_enabled: true, dew_heater_power: 10}
    return {
        settings: ref({...hardware, guide_camera: {...hardware}}),
        refreshSettings: vi.fn().mockResolvedValue(undefined),
        cameras: ref([{id: 'cam1', name: 'Dew Cam', info: {has_dew_heater: true}}]),
        selectedCamera: ref('cam1'),
        selectedCameraRole: ref(role),
    }
}

function dragPower(wrapper, value) {
    const slider = wrapper.findComponent({name: 'BaseSlider'})
    slider.vm.$emit('update:modelValue', value)
    slider.vm.$emit('change', value)
}

describe('DewHeaterControl', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.useFakeTimers()
        updateSettings.mockResolvedValue({})
    })

    afterEach(() => vi.useRealTimers())

    it('sends the last power once, 300 ms after the drag ends, for the edited camera', async () => {
        const wrapper = mount(DewHeaterControl, {global: {provide: provides('guide')}})
        await flushPromises()

        dragPower(wrapper, 30)
        dragPower(wrapper, 45)
        expect(updateSettings).not.toHaveBeenCalled()
        await vi.advanceTimersByTimeAsync(300)

        expect(updateSettings).toHaveBeenCalledTimes(1)
        expect(updateSettings).toHaveBeenCalledWith({camera_role: 'guide', dew_heater_power: 45})
    })

    it('sends a pending power when the Settings tab is torn down', async () => {
        const wrapper = mount({render: () => h(DewHeaterControl)}, {global: {provide: provides()}})
        await flushPromises()

        dragPower(wrapper, 60)
        wrapper.unmount()
        await flushPromises()

        expect(updateSettings).toHaveBeenCalledTimes(1)
        expect(updateSettings).toHaveBeenCalledWith({camera_role: 'main', dew_heater_power: 60})
    })

    // The camera list sits on another tab, but a fault can move the selection inside the
    // debounce: the pending power is the outgoing camera's, and only its.
    it('sends a pending power to its own camera when the selection changes', async () => {
        const provide = provides('guide')
        const wrapper = mount(DewHeaterControl, {global: {provide}})
        await flushPromises()

        dragPower(wrapper, 45)
        provide.selectedCameraRole.value = 'main'
        await flushPromises()
        expect(updateSettings).toHaveBeenCalledWith({camera_role: 'guide', dew_heater_power: 45})

        dragPower(wrapper, 20)
        await vi.advanceTimersByTimeAsync(300)
        expect(updateSettings).toHaveBeenCalledTimes(2)
        expect(updateSettings).toHaveBeenLastCalledWith({camera_role: 'main', dew_heater_power: 20})
    })
})
