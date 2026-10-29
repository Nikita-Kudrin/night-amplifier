import {computed} from 'vue'
import type {DeepReadonly} from 'vue'
import {useError} from './useError'
import {
    useApplySettings,
    useCameraStatus,
    useSelectedCamera,
    useSelectedCameraRole,
    useSettings,
} from './appContext'
import type {HardwarePatch} from './api'
import type {CameraCaptureProfile, CameraRole, Settings} from './api.types'

/** A hardware edit, before `apply` names the camera it is for. */
export type HardwareEdit = Omit<HardwarePatch, 'camera_role'>

/**
 * The settings block of the camera in `role`: the imaging camera's hardware settings sit at
 * the top level, the guide camera's in `guide_camera`.
 */
export function hardwareBlock(
    settings: DeepReadonly<Settings> | null | undefined,
    role: CameraRole,
): DeepReadonly<Settings> | DeepReadonly<CameraCaptureProfile> | null {
    if (!settings) return null
    return role === 'guide' ? settings.guide_camera : settings
}

/**
 * The selected camera's hardware settings (cooler, dew heater, binning), its live status
 * and a save that names the camera.
 *
 * These controls save through the API themselves rather than emitting `apply` like a
 * settings section: every request must carry `camera_role`, so that the edit lands on
 * the camera it was made for. Each sends at once (a slider on release), bound to the
 * camera selected when it was made.
 */
export function useCameraHardwareSettings() {
    const settings = useSettings()
    const applySettings = useApplySettings()
    const cameraStatus = useCameraStatus()
    const settingsRole = useSelectedCameraRole()

    const {error, clearError, withErrorHandling} = useError()
    const currentCamera = useSelectedCamera()

    const cameraSettings = computed(() => hardwareBlock(settings.value, settingsRole.value))

    const liveStatus = computed(() => {
        const name = currentCamera.value?.name
        return name ? cameraStatus.value[name] : null
    })

    async function apply(edit: HardwareEdit, role: CameraRole = settingsRole.value) {
        const patch: HardwarePatch = {camera_role: role, ...edit}
        await withErrorHandling(async () => {
            await applySettings(patch)
        })
    }

    return {settingsRole, cameraSettings, currentCamera, liveStatus, error, clearError, withErrorHandling, apply}
}
