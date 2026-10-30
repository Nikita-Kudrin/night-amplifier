import {describe, it, expect, vi} from 'vitest'
import {mount, flushPromises} from '@vue/test-utils'
import {ref} from 'vue'
import DaylightBalanceStatus from './DaylightBalanceStatus.vue'
import {contextProvides} from '../composables/appContext'
import {camera, settings} from '../test/fixtures'
import type {CameraRole, DaylightBalance, DaylightBalanceSource, SensorType} from '../composables/api.types'

const MOON: DaylightBalance = {red: 1.17, green: 1, blue: 1.89}

function provides(role: CameraRole = 'main', sensorType: SensorType = 'Color', guideSource: DaylightBalanceSource = 'moon') {
    const base = settings()
    return {
        settings: ref(settings({
            guide_camera: {...base.guide_camera, daylight_balance: MOON, daylight_balance_source: guideSource},
        })),
        refreshSettings: vi.fn(async () => null),
        cameras: ref([camera({id: 'cam1', name: 'Ares-C PRO', info: {sensor_type: sensorType}})]),
        selectedCamera: ref<string | null>('cam1'),
        selectedCameraRole: ref(role),
    }
}

async function mounted(provide = provides()) {
    const wrapper = mount(DaylightBalanceStatus, {global: {provide: contextProvides(provide)}})
    await flushPromises()
    return wrapper
}

describe('DaylightBalanceStatus', () => {
    it('is shown only for a colour camera', async () => {
        expect((await mounted(provides('main', 'Color'))).find('.daylight-balance-status').exists()).toBe(true)
        expect((await mounted(provides('main', 'Mono'))).find('.daylight-balance-status').exists()).toBe(false)
    })

    it('says whether the selected camera\'s balance was read off the Moon, and follows the selection', async () => {
        const provide = provides('main')
        const wrapper = await mounted(provide)
        expect(wrapper.find('.balance-source').text()).toBe('Typical balance until Planetary shows the Moon')
        expect(wrapper.find('.balance-gains').exists()).toBe(false)

        provide.selectedCameraRole.value = 'guide'
        await flushPromises()
        expect(wrapper.find('.balance-source').text()).toBe('Read off the Moon')
        expect(wrapper.find('.balance-gains').text()).toBe('R ×1.17 · G ×1.00 · B ×1.89')
    })

    it('offers nothing to press: the server measures by itself', async () => {
        const wrapper = await mounted(provides('guide'))
        expect(wrapper.find('button').exists()).toBe(false)
    })
})
