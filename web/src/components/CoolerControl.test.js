import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref, nextTick} from 'vue'
import CoolerControl from './CoolerControl.vue'

vi.mock('../composables/api.js', () => ({
    updateSettings: vi.fn(),
}))

import {updateSettings} from '../composables/api.js'

describe('CoolerControl', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        updateSettings.mockResolvedValue({})
    })

    function createMockProvides(overrides = {}) {
        return {
            settings: ref(
                overrides.settings ?? {
                    cooler_enabled: false,
                    target_temp_c: null,
                }
            ),
            refreshSettings: vi.fn().mockResolvedValue(undefined),
            cameras: ref(overrides.cameras ?? []),
            selectedCamera: ref(overrides.selectedCamera ?? null),
            cameraStatus: ref(overrides.cameraStatus ?? {}),
        }
    }

    function mountControl(provides = {}) {
        return mount(CoolerControl, {
            global: {
                provide: createMockProvides(provides),
            },
        })
    }

    it('renders nothing when the selected camera has no cooler', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'No-Cool Cam',
                    info: {has_cooler: false, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        expect(wrapper.find('.cooler-control').exists()).toBe(false)
    })

    it('shows the cooler section when the camera supports cooling', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {
                        has_cooler: true,
                        min_temp_c: -30,
                        max_temp_c: 20,
                        max_width: 1920,
                        max_height: 1080,
                    },
                },
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        expect(wrapper.find('.cooler-control').exists()).toBe(true)
        expect(wrapper.text()).toContain('Cooler')
        expect(wrapper.text()).toContain('Sensor')
    })

    it('shows the target slider only when cooler is enabled', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {has_cooler: true, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
            settings: {cooler_enabled: true, target_temp_c: -10},
        })
        await flushPromises()

        expect(wrapper.find('.slider-control').exists()).toBe(true)
    })

    it('renders Stable badge when sensor is within tolerance of target', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {has_cooler: true, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
            cameraStatus: {
                'Cool Cam': {
                    temperature_c: -10.1,
                    cooler_power: 50,
                    cooler_on: true,
                    target_temp_c: -10.0,
                },
            },
        })
        await flushPromises()

        expect(wrapper.text()).toContain('Stable')
    })

    it('renders Cooling badge when sensor temperature is above target', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {has_cooler: true, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
            cameraStatus: {
                'Cool Cam': {
                    temperature_c: 5.0,
                    cooler_power: 75,
                    cooler_on: true,
                    target_temp_c: -10.0,
                },
            },
        })
        await flushPromises()

        expect(wrapper.text()).toContain('Cooling')
    })

    it('calls updateSettings when the cooler is toggled on', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {has_cooler: true, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        const toggle = wrapper.find('input[type="checkbox"]')
        await toggle.setValue(true)
        await nextTick()

        expect(updateSettings).toHaveBeenCalledWith({cooler_enabled: true})
    })

    it('renders the Fast toggle when the camera has a cooler', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {has_cooler: true, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        expect(wrapper.find('.fast-mode-row').exists()).toBe(true)
        expect(wrapper.text()).toContain('Fast')
    })

    it('shows the warning icon only when Fast mode is on', async () => {
        const provides = {
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {has_cooler: true, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
            settings: {
                cooler_enabled: false,
                target_temp_c: null,
                cooler_fast_mode: false,
            },
        }

        const off = mountControl(provides)
        await flushPromises()
        expect(off.find('.fast-warning-icon').exists()).toBe(false)

        const on = mountControl({
            ...provides,
            settings: {...provides.settings, cooler_fast_mode: true},
        })
        await flushPromises()
        expect(on.find('.fast-warning-icon').exists()).toBe(true)
    })

    it('calls updateSettings with cooler_fast_mode when Fast is toggled', async () => {
        const wrapper = mountControl({
            cameras: [
                {
                    id: 'cam1',
                    name: 'Cool Cam',
                    info: {has_cooler: true, max_width: 1920, max_height: 1080},
                },
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        // Toggles are rendered in DOM order: cooler_enabled first, then Fast.
        const toggles = wrapper.findAll('input[type="checkbox"]')
        const fastToggle = toggles[toggles.length - 1]
        await fastToggle.setValue(true)
        await nextTick()

        expect(updateSettings).toHaveBeenCalledWith({cooler_fast_mode: true})
    })
})
