import {defineComponent, h, nextTick, ref} from 'vue'
import {mountWithDefaults} from '../test/mount'
import type {ProvideOverrides} from '../test/mount'
import {camera, settings} from '../test/fixtures'
import {hardwareBlock, useCameraHardwareSettings} from './useCameraHardwareSettings'
import type {HardwareEdit} from './useCameraHardwareSettings'
import type {HardwarePatch} from './api'
import type {CameraStatus} from './cameraStore'
import type {CameraRole} from './api.types'

vi.mock('./api', async (importOriginal) =>
    (await import('../test/mockApi')).mockApi(await importOriginal()))

import {ApiError, updateSettings} from './api'

const CAMERAS = [
    camera({id: 'main1', name: 'Imaging Cam', info: {has_cooler: true}}),
    camera({id: 'guide1', name: 'Guide Cam', info: {has_cooler: false}}),
]

const IMAGING_STATUS: CameraStatus = {
    cooler_on: true,
    temperature_c: -9.8,
    dew_heater_on: false,
    cooler_power: null,
    target_temp_c: null,
}

type Hardware = ReturnType<typeof useCameraHardwareSettings>

function mountHardware(provide: {role?: CameraRole, selectedCamera?: string, overrides?: ProvideOverrides} = {}) {
    let hardware!: Hardware
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
            cameraStatus: {'Imaging Cam': IMAGING_STATUS},
            ...provide.overrides,
        },
    })
    return {wrapper, hardware, role, selectedCamera}
}

describe('hardwareBlock', () => {
    it('is the top level for the imaging camera and guide_camera for the guide', () => {
        const all = settings({gain: 1, guide_camera: {...settings().guide_camera, gain: 2}})
        expect(hardwareBlock(all, 'main')).toBe(all)
        expect(hardwareBlock(all, 'guide')).toBe(all.guide_camera)
        expect(hardwareBlock(null, 'guide')).toBeNull()
    })
})

describe('useCameraHardwareSettings', () => {
    beforeEach(() => vi.clearAllMocks())

    it('follows the selected role\'s block', async () => {
        const {hardware, role} = mountHardware()
        expect(hardware.cameraSettings.value?.target_temp_c).toBe(-10)

        role.value = 'guide'
        await nextTick()

        expect(hardware.cameraSettings.value?.target_temp_c).toBe(5)
    })

    it('reads the live status of the selected camera by name', async () => {
        const {hardware, selectedCamera} = mountHardware()
        expect(hardware.liveStatus.value).toEqual(IMAGING_STATUS)

        selectedCamera.value = 'guide1'
        await nextTick()

        expect(hardware.liveStatus.value).toBeUndefined()
    })

    // The store adopts the POST's answer: one request per edit, no refresh behind it.
    it('names the camera in every save, through the settings store', async () => {
        const refreshSettings = vi.fn(async () => null)
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
        vi.mocked(updateSettings).mockRejectedValueOnce(new ApiError('Cooler busy', 409))
        const {hardware} = mountHardware()

        await hardware.apply({cooler_enabled: true})

        expect(hardware.error.value).toBe('Cooler busy')
    })

    // Binning without `camera_role` rebinned the imaging camera while the guide was selected.
    it('cannot send a hardware edit that does not name its camera', () => {
        expectTypeOf<Parameters<Hardware['apply']>[0]>().toEqualTypeOf<HardwareEdit>()
        expectTypeOf<HardwareEdit>().not.toHaveProperty('camera_role')
        expectTypeOf({camera_role: 'guide', bin: 2} as const).toExtend<HardwarePatch>()
        // @ts-expect-error a hardware patch names its camera
        expectTypeOf({bin: 2} as const).toExtend<HardwarePatch>()
        // @ts-expect-error stacking is no hardware setting
        expectTypeOf({stacking: true} as const).toExtend<HardwareEdit>()
    })
})
