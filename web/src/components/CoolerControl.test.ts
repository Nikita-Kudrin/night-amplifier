import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import type {VueWrapper} from '@vue/test-utils'
import {ref, nextTick} from 'vue'
import CoolerControl from './CoolerControl.vue'
import {COOLER_TEMP_LIMITS} from '../constants'

vi.mock('../composables/api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal(), {
        updateSettings: vi.fn(),
    }))

import {updateSettings} from '../composables/api.js'
import {contextProvides} from '../composables/appContext.js'
import {camera, settings} from '../test/fixtures.js'
import type {ByCameraName, CameraStatus} from '../composables/cameraStore.js'
import type {Camera, CameraCaptureProfile, CameraInfo, CameraRole, Settings} from '../composables/api.types.js'

/** The guide camera's block: the defaults, with `fields` over them. */
function guideCamera(fields: Partial<CameraCaptureProfile>): CameraCaptureProfile {
    return {...settings().guide_camera, ...fields}
}

/** A connected camera, named `cam1`, with `info` over the fixture's. */
function cam(name: string, info: Partial<CameraInfo>): Camera {
    return camera({id: 'cam1', name, info})
}

/** A cooler's status sample; `dew_heater_on` is the only field these tests do not set. */
function status(fields: Omit<CameraStatus, 'dew_heater_on'>): CameraStatus {
    return {dew_heater_on: false, ...fields}
}

describe('CoolerControl', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(updateSettings).mockResolvedValue(settings())
    })

    afterEach(() => vi.useRealTimers())

    interface Overrides {
        /** Over the default settings. */
        settings?: Partial<Settings>
        cameras?: Camera[]
        selectedCamera?: string | null
        cameraStatus?: ByCameraName<CameraStatus>
        role?: CameraRole
    }

    function createMockProvides(overrides: Overrides = {}) {
        return {
            settings: ref(settings(overrides.settings ?? {cooler_enabled: false, target_temp_c: null})),
            refreshSettings: vi.fn(async () => null),
            cameras: ref(overrides.cameras ?? []),
            selectedCamera: ref(overrides.selectedCamera ?? null),
            cameraStatus: ref(overrides.cameraStatus ?? {}),
            selectedCameraRole: ref(overrides.role ?? 'main'),
        }
    }

    function mountControl(provides: Overrides = {}) {
        return mount(CoolerControl, {
            global: {
                provide: contextProvides(createMockProvides(provides)),
            },
        })
    }

    it('renders nothing when the selected camera has no cooler', async () => {
        const wrapper = mountControl({
            cameras: [
                cam('No-Cool Cam', {has_cooler: false, max_width: 1920, max_height: 1080}),
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        expect(wrapper.find('.cooler-control').exists()).toBe(false)
    })

    it('shows the cooler section when the camera supports cooling', async () => {
        const wrapper = mountControl({
            cameras: [
                cam('Cool Cam', {has_cooler: true, min_temp_c: -30, max_temp_c: 20, max_width: 1920, max_height: 1080}),
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        expect(wrapper.find('.cooler-control').exists()).toBe(true)
        expect(wrapper.text()).toContain('Camera cooler')
        // The camera list sits on another tab, so the heading names the camera.
        expect(wrapper.find('.section-title').text()).toContain('Cool Cam')
        expect(wrapper.text()).toContain('Sensor')
    })

    it('shows the target slider only when cooler is enabled', async () => {
        const wrapper = mountControl({
            cameras: [
                cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080}),
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
                cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080}),
            ],
            selectedCamera: 'cam1',
            cameraStatus: {
                'Cool Cam': status({
                    temperature_c: -10.1,
                    cooler_power: 50,
                    cooler_on: true,
                    target_temp_c: -10.0,
                }),
            },
        })
        await flushPromises()

        expect(wrapper.text()).toContain('Stable')
    })

    it('renders Cooling badge when sensor temperature is above target', async () => {
        const wrapper = mountControl({
            cameras: [
                cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080}),
            ],
            selectedCamera: 'cam1',
            cameraStatus: {
                'Cool Cam': status({
                    temperature_c: 5.0,
                    cooler_power: 75,
                    cooler_on: true,
                    target_temp_c: -10.0,
                }),
            },
        })
        await flushPromises()

        expect(wrapper.text()).toContain('Cooling')
    })

    // The server's default setpoint is null; the slider shows the default, which must be
    // the one the camera is told to cool to.
    it('sends the shown setpoint with Cool when the camera has none stored', async () => {
        const wrapper = mountControl({
            cameras: [
                cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080}),
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        const toggle = wrapper.find('input[type="checkbox"]')
        await toggle.setValue(true)
        await nextTick()

        expect(updateSettings).toHaveBeenCalledWith({
            camera_role: 'main',
            cooler_enabled: true,
            target_temp_c: COOLER_TEMP_LIMITS.default,
        })
    })

    it('sends Cool alone when the camera has a setpoint stored', async () => {
        const wrapper = mountControl({
            cameras: [cam('Cool Cam', {has_cooler: true})],
            selectedCamera: 'cam1',
            settings: {cooler_enabled: false, target_temp_c: -20},
        })
        await flushPromises()

        await wrapper.find('input[type="checkbox"]').setValue(true)

        expect(updateSettings).toHaveBeenCalledExactlyOnceWith({camera_role: 'main', cooler_enabled: true})
    })

    it('renders the Fast toggle when the camera has a cooler', async () => {
        const wrapper = mountControl({
            cameras: [
                cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080}),
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        expect(wrapper.find('.toggles-row').exists()).toBe(true)
        expect(wrapper.text()).toContain('Fast')
    })

    it('shows the warning icon only when Fast mode is on', async () => {
        const provides: Overrides = {
            cameras: [
                cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080}),
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
                cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080}),
            ],
            selectedCamera: 'cam1',
        })
        await flushPromises()

        // Toggles are rendered in DOM order: cooler_enabled first, then Fast.
        const toggles = wrapper.findAll('input[type="checkbox"]')
        const fastToggle = toggles[toggles.length - 1]
        await fastToggle.setValue(true)
        await nextTick()

        expect(updateSettings).toHaveBeenCalledWith({camera_role: 'main', cooler_fast_mode: true})
    })
    describe('target temperature', () => {
        const cooled = {
            cameras: [cam('Cool Cam', {has_cooler: true, max_width: 1920, max_height: 1080})],
            selectedCamera: 'cam1',
            settings: {cooler_enabled: true, target_temp_c: -10},
        }

        function dragTarget(wrapper: VueWrapper, value: number) {
            const slider = wrapper.findComponent({name: 'BaseSlider'})
            slider.vm.$emit('update:modelValue', value)
            slider.vm.$emit('change', value)
        }

        // A slider release sends at once, like every other settings slider.
        it('sends the released value at once, naming the camera', async () => {
            vi.useFakeTimers()
            const hardware = {cooler_enabled: true, target_temp_c: -10}
            const wrapper = mountControl({
                ...cooled,
                settings: {...hardware, guide_camera: guideCamera(hardware)},
                role: 'guide',
            })
            await flushPromises()

            dragTarget(wrapper, -15)
            await flushPromises()

            expect(updateSettings).toHaveBeenCalledExactlyOnceWith({camera_role: 'guide', target_temp_c: -15})
        })

        // The guide's setpoint is the server's default, null: the previous camera's -25 °C
        // stayed on the slider.
        it('shows the default, not the last camera\'s setpoint, for a camera with none', async () => {
            const provide = createMockProvides({
                ...cooled,
                settings: {
                    cooler_enabled: true,
                    target_temp_c: -25,
                    guide_camera: guideCamera({cooler_enabled: true, target_temp_c: null}),
                },
            })
            const wrapper = mount(CoolerControl, {global: {provide: contextProvides(provide)}})
            const target = () => wrapper.findComponent({name: 'BaseSlider'}).props('modelValue')
            await flushPromises()
            expect(target()).toBe(-25)

            provide.selectedCameraRole.value = 'guide'
            await flushPromises()

            expect(target()).toBe(COOLER_TEMP_LIMITS.default)
        })
    })
})
