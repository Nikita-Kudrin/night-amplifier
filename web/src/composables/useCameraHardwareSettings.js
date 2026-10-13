import {computed, inject, ref, watch} from 'vue'
import {updateSettings} from './api.js'
import {useDebouncedCall} from './useDebouncedCall.js'
import {useError} from './useError.js'
import {useSelectedCamera} from './appContext.js'
import {SETTINGS_DEBOUNCE_MS} from '../constants'

/**
 * The settings block of the camera in `role`: the imaging camera's hardware settings sit at
 * the top level, the guide camera's in `guide_camera`.
 */
export function hardwareBlock(settings, role) {
    if (!settings) return null
    return role === 'guide' ? settings.guide_camera : settings
}

/**
 * The selected camera's hardware settings (cooler, dew heater), its live status and a
 * save that names the camera.
 *
 * These controls save through the API themselves rather than emitting `apply` like a
 * settings section: every request must carry `camera_role`, so that the edit lands on
 * the camera it was made for. A debounced edit is bound to the camera selected when it
 * was made, and is sent at once when the selection moves, so it is never coalesced with
 * the next camera's.
 */
export function useCameraHardwareSettings() {
    const settings = inject('settings', ref(null))
    const refreshSettings = inject('refreshSettings', async () => {})
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
            await updateSettings({camera_role: role, ...payload})
            await refreshSettings()
        })
    }

    const {schedule, flush} = useDebouncedCall(apply, SETTINGS_DEBOUNCE_MS)
    watch(settingsRole, flush)

    function applyDebounced(payload) {
        schedule(payload, settingsRole.value)
    }

    return {settingsRole, cameraSettings, currentCamera, liveStatus, error, clearError, apply, applyDebounced}
}
