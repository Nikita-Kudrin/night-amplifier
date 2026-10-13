import {defineComponent, h, nextTick, ref} from 'vue'
import {mountWithDefaults} from '../test/mount.js'
import {settings} from '../test/fixtures.js'
import {hardwareBlock, useCameraHardwareSettings} from './useCameraHardwareSettings.js'

vi.mock('./api.js', async (importOriginal) =>
    (await import('../test/mockApi.js')).mockApi(await importOriginal()))

import {ApiError, updateSettings} from './api.js'

const CAMERAS = [
    {id: 'main1', name: 'Imaging Cam', info: {has_cooler: true}},
    {id: 'guide1', name: 'Guide Cam', info: {has_cooler: false}},
]

function mountHardware(provide = {}) {
    let hardware
    const Host = defineComponent({
        setup() {
            hardware = useCameraHardwareSettings()
            return () => h('div')
        },
    })
    const role = ref(provide.role ?? 'main')
    const selectedCamera = ref(provide.selectedCamera ?? 'main1')
    const wrapper = mountWithDefaults(Host, {
        provide: {
            settings: settings({target_temp_c: -10, guide_camera: {...settings().guide_camera, target_temp_c: 5}}),
            cameras: CAMERAS,
            selectedCamera,
            selectedCameraRole: role,
            cameraStatus: {'Imaging Cam': {cooler_on: true, temperature_c: -9.8}},
            ...provide.overrides,
        },
    })
    return {wrapper, hardware, role, selectedCamera}
}

describe('hardwareBlock', () => {
    it('is the top level for the imaging camera and guide_camera for the guide', () => {
        const all = {gain: 1, guide_camera: {gain: 2}}
        expect(hardwareBlock(all, 'main')).toBe(all)
        expect(hardwareBlock(all, 'guide')).toBe(all.guide_camera)
        expect(hardwareBlock(null, 'guide')).toBeNull()
    })
})

describe('useCameraHardwareSettings', () => {
    beforeEach(() => vi.clearAllMocks())

    it('follows the selected role\'s block', async () => {
        const {hardware, role} = mountHardware()
        expect(hardware.cameraSettings.value.target_temp_c).toBe(-10)

        role.value = 'guide'
        await nextTick()

        expect(hardware.cameraSettings.value.target_temp_c).toBe(5)
    })

    it('reads the live status of the selected camera by name', async () => {
        const {hardware, selectedCamera} = mountHardware()
        expect(hardware.liveStatus.value).toEqual({cooler_on: true, temperature_c: -9.8})

        selectedCamera.value = 'guide1'
        await nextTick()

        expect(hardware.liveStatus.value).toBeUndefined()
    })

    // The store adopts the POST's answer: one request per edit, no refresh behind it.
    it('names the camera in every save, through the settings store', async () => {
        const refreshSettings = vi.fn(async () => {})
        const applySettings = vi.fn(async () => settings())
        const {hardware} = mountHardware({role: 'guide', overrides: {refreshSettings, applySettings}})

        await hardware.apply({cooler_enabled: true})

        expect(applySettings).toHaveBeenCalledExactlyOnceWith({camera_role: 'guide', cooler_enabled: true})
        expect(refreshSettings).not.toHaveBeenCalled()
    })

    it('posts the patch itself when mounted without the store', async () => {
        const {hardware} = mountHardware({role: 'guide'})
        await hardware.apply({cooler_enabled: true})
        expect(updateSettings).toHaveBeenCalledWith({camera_role: 'guide', cooler_enabled: true})
    })

    it('surfaces a refused save', async () => {
        updateSettings.mockRejectedValueOnce(new ApiError('Cooler busy', 409))
        const {hardware} = mountHardware()

        await hardware.apply({cooler_enabled: true})

        expect(hardware.error.value).toBe('Cooler busy')
    })
})
