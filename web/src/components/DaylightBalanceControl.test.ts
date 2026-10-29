import {describe, it, expect, vi, beforeEach} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import DaylightBalanceControl from './DaylightBalanceControl.vue'

vi.mock('../composables/api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal(), {
        updateSettings: vi.fn(),
        calibrateDaylightBalance: vi.fn(),
    }))

import {ApiError, calibrateDaylightBalance, updateSettings} from '../composables/api'
import {contextProvides} from '../composables/appContext'
import {camera, settings} from '../test/fixtures'
import type {CameraRole, DaylightBalance, SensorType} from '../composables/api.types'

const MOON: DaylightBalance = {red: 1.06, green: 1, blue: 2.07}

function provides(role: CameraRole = 'main', sensorType: SensorType = 'Color', guide: DaylightBalance = MOON) {
    const base = settings()
    return {
        settings: ref(settings({guide_camera: {...base.guide_camera, daylight_balance: guide}})),
        refreshSettings: vi.fn(async () => null),
        cameras: ref([camera({id: 'cam1', name: 'Ares-C PRO', info: {sensor_type: sensorType}})]),
        selectedCamera: ref<string | null>('cam1'),
        selectedCameraRole: ref(role),
    }
}

async function mounted(provide = provides()) {
    const wrapper = mount(DaylightBalanceControl, {global: {provide: contextProvides(provide)}})
    await flushPromises()
    return wrapper
}

describe('DaylightBalanceControl', () => {
    beforeEach(() => {
        vi.clearAllMocks()
        vi.mocked(updateSettings).mockResolvedValue(settings())
        vi.mocked(calibrateDaylightBalance).mockResolvedValue(MOON)
    })

    it('is offered only for a colour camera', async () => {
        expect((await mounted(provides('main', 'Color'))).find('.daylight-balance-control').exists()).toBe(true)
        expect((await mounted(provides('main', 'Mono'))).find('.daylight-balance-control').exists()).toBe(false)
    })

    it('shows the selected camera\'s gains, or that it was never calibrated', async () => {
        const provide = provides('main')
        const wrapper = await mounted(provide)
        expect(wrapper.find('.balance-gains').text()).toBe('Not calibrated')
        expect(wrapper.find<HTMLButtonElement>('.reset-button').element.disabled).toBe(true)

        provide.selectedCameraRole.value = 'guide'
        await flushPromises()
        expect(wrapper.find('.balance-gains').text()).toBe('R ×1.06 · G ×1.00 · B ×2.07')
        expect(wrapper.find<HTMLButtonElement>('.reset-button').element.disabled).toBe(false)
    })

    it('calibrates the camera selected when it was pressed', async () => {
        const wrapper = await mounted(provides('guide'))
        await wrapper.find('.calibrate-button').trigger('click')
        await flushPromises()
        expect(calibrateDaylightBalance).toHaveBeenCalledExactlyOnceWith('guide')
    })

    it('says why a calibration was refused', async () => {
        vi.mocked(calibrateDaylightBalance).mockRejectedValue(
            new ApiError('Calibrate on a Planetary view of the Moon or the Sun', 409))
        const wrapper = await mounted()
        await wrapper.find('.calibrate-button').trigger('click')
        await flushPromises()
        expect(wrapper.text()).toContain('Calibrate on a Planetary view of the Moon or the Sun')
        expect(wrapper.find<HTMLButtonElement>('.calibrate-button').element.disabled).toBe(false)
    })

    it('resets to the camera\'s raw colour through a hardware edit for that camera', async () => {
        const wrapper = await mounted(provides('guide'))
        await wrapper.find('.reset-button').trigger('click')
        await flushPromises()
        expect(updateSettings).toHaveBeenCalledExactlyOnceWith({
            camera_role: 'guide',
            daylight_balance: {red: 1, green: 1, blue: 1},
        })
    })
})
