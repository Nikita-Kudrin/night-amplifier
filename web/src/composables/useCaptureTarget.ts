import {computed} from 'vue'
import type {Ref} from 'vue'
import {useCameraPhase, useEventStreamContext, useGuideCamera, useSelectedCameraId} from './appContext'
import {CAMERA_PHASES, CAPTURE_STATES, isCaptureRunning} from '../constants'
import type {CameraRole} from './api.types'

/**
 * Whether the camera Start/Stop addresses is running, and what its buttons say. `role`
 * is the selection: the imaging camera's capture session, or the guide camera's loop.
 *
 * The guide loop is read from the camera's phase rather than `captureState`: `Guiding`
 * is a phase of its own because a guide loop is not a capture session. It has no
 * Stopping state either — its stop is one request that returns once the loop has handed
 * its handle back — so `busy` (that request in flight) is the only signal there is.
 * Without a provided event stream the imaging camera reads as idle.
 */
export function useCaptureTarget({role, busy}: {role: Readonly<Ref<CameraRole>>; busy: Readonly<Ref<boolean>>}) {
    const eventStream = useEventStreamContext()
    const cameraPhase = useCameraPhase()
    const selectedCamera = useSelectedCameraId()
    const guideCamera = useGuideCamera()

    const isGuide = computed(() => role.value === 'guide')

    const captureState = computed(() => eventStream?.captureState.value ?? CAPTURE_STATES.IDLE)
    const mainCapturing = computed(() => {
        const state = captureState.value
        return isCaptureRunning(state) || state === CAPTURE_STATES.STARTING
    })
    const mainStopping = computed(() => captureState.value === CAPTURE_STATES.STOPPING)

    const guideRunning = computed(() => {
        const name = guideCamera.value?.name
        return name ? cameraPhase.value[name] === CAMERA_PHASES.GUIDING : false
    })

    const isCapturing = computed(() => (isGuide.value ? guideRunning.value : mainCapturing.value))
    const isStopping = computed(() => (isGuide.value ? busy.value && guideRunning.value : mainStopping.value))

    const canStart = computed(() => {
        if (isCapturing.value || isStopping.value) return false
        return isGuide.value ? Boolean(guideCamera.value) : Boolean(selectedCamera.value)
    })

    // Named, so it is never ambiguous which camera the button acts on.
    const startLabel = computed(() => (isGuide.value ? 'Start guide' : 'Start'))
    const stopLabel = computed(() => (isGuide.value ? 'Stop guide' : 'Stop'))

    return {isGuide, mainCapturing, isCapturing, isStopping, canStart, startLabel, stopLabel}
}
