import {computed, inject, ref} from 'vue'
import {useError} from './useError.js'
import {useApplySettings, useSelectedCamera} from './appContext.js'

/**
 * The settings block of the camera in `role`: the imaging camera's hardware settings sit at
 * the top level, the guide camera's in `guide_camera`.
 */
export function hardwareBlock(settings, role) {
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
    const settings = inject('settings', ref(null))
    const applySettings = useApplySettings()
    const cameraStatus = inject('cameraStatus', ref({}))
    const settingsRole = inject('selectedCameraRole', computed(() => 'main'))

    const {error, clearError, withErrorHandling} = useError()
    const currentCamera = useSelectedCamera()

    const cameraSettings = computed(() => hardwareBlock(settings.value, settingsRole.value))

    const liveStatus = computed(() => {
        const name = currentCamera.value?.name
        return name ? cameraStatus.value?.[name] : null
    })

    async function apply(payload, role = settingsRole.value) {
        await withErrorHandling(async () => {
            await applySettings({camera_role: role, ...payload})
        })
    }

    return {settingsRole, cameraSettings, currentCamera, liveStatus, error, clearError, apply}
}
