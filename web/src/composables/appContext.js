import {computed, inject, provide, unref} from 'vue'
import {DEFAULT_CAPABILITIES} from '../constants'

/**
 * What `App.vue` provides to the components below it, and the accessors that read it.
 *
 * Server state goes out read-only: a component edits it through the API and the answer
 * comes back through `useAppState`. The camera selection and the simulator switch are UI
 * state the panels set themselves, so those two stay writable. The keys are plain strings
 * because tests provide them by name.
 */
export function provideAppContext(appState, eventStream) {
    provide('settings', appState.settings)
    provide('cameras', appState.cameras)
    provide('selectedCamera', appState.selection)
    provide('eventStream', eventStream)
    provide('simulatorEnabled', appState.simulatorEnabled)
    provide('refreshSettings', appState.refreshSettings)
    provide('refreshCameras', appState.refreshCameras)
    provide('capabilities', appState.capabilities)
    provide('cameraStatus', appState.cameraStatus)
    provide('cameraPhase', appState.cameraPhase)
    provide('warmupEndsAt', appState.warmupEndsAt)
    provide('mainCamera', appState.mainCamera)
    provide('guideCamera', appState.guideCamera)
    provide('hasGuideCamera', appState.hasGuideCamera)
    provide('selectedCameraRole', appState.selectedCameraRole)
}

/**
 * The server's capabilities, plus one flag per Pro feature a control locks on. Without a
 * provided answer every flag is off, so Pro controls show locked rather than missing.
 */
export function useCapabilities() {
    const provided = inject('capabilities', null)
    const capabilities = computed(() => unref(provided) ?? DEFAULT_CAPABILITIES)
    const flag = (read) => computed(() => read(capabilities.value) ?? false)
    return {
        capabilities,
        advancedRejection: flag((c) => c.deep_sky?.advanced_rejection),
        rbfBackground: flag((c) => c.deep_sky?.rbf_background),
        saturationBoost: flag((c) => c.deep_sky?.saturation_boost),
        denoise: flag((c) => c.deep_sky?.denoise),
        aiDenoise: flag((c) => c.deep_sky?.ai_denoise),
        advancedPlanetary: flag((c) => c.planetary?.advanced_stacking),
        astapSolver: flag((c) => c.push_to?.astap_solver),
        cometPro: flag((c) => c.comet?.pro_stacking),
        debugLogging: flag((c) => c.debug_logging),
    }
}

/** The camera list's entry for the selected camera, or `null`. */
export function useSelectedCamera() {
    const cameras = inject('cameras', null)
    const selectedId = inject('selectedCamera', null)
    return computed(() => findCamera(unref(cameras), unref(selectedId)))
}

/** The entry in `cameras` with this id, or `null`. */
export function findCamera(cameras, id) {
    return (cameras ?? []).find((camera) => camera.id === id) ?? null
}

/** Whether a camera entry is the directory-backed simulator. */
export function isSimulator(camera) {
    return camera?.provider === 'Simulator'
}
