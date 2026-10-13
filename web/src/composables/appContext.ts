import {computed, inject, provide, readonly, ref} from 'vue'
import type {ComputedRef, InjectionKey, Ref} from 'vue'
import {updateSettings} from './api.js'
import {DEFAULT_CAPABILITIES} from '../constants'
import type {Camera} from './api.types.js'
import type {AppState} from './useAppState.js'
import type {EventStream} from './useEventStream.js'

/**
 * What `App.vue` provides to the components below it, and the accessors that read it.
 *
 * Server state goes out read-only: a component edits settings through `applySettings`
 * (`useApplySettings`), whose answer the settings store adopts. The camera selection and
 * the simulator switch are UI state the panels set themselves, so those two stay writable.
 */

/**
 * Registered (`Symbol.for`), so a module loaded twice — by HMR, or a test's `vi.resetModules()`
 * before it imports a component again — still names the same keys.
 */
const key = (name: string) => Symbol.for(`night-amplifier:${name}`)

export const SETTINGS: InjectionKey<AppState['settings']> = key('settings')
export const CAMERAS: InjectionKey<AppState['cameras']> = key('cameras')
export const SELECTED_CAMERA: InjectionKey<Ref<string | null>> = key('selectedCamera')
export const EVENT_STREAM: InjectionKey<EventStream> = key('eventStream')
export const SIMULATOR_ENABLED: InjectionKey<Ref<boolean>> = key('simulatorEnabled')
export const REFRESH_SETTINGS: InjectionKey<AppState['refreshSettings']> = key('refreshSettings')
export const APPLY_SETTINGS: InjectionKey<AppState['applySettings']> = key('applySettings')
export const REFRESH_CAMERAS: InjectionKey<AppState['refreshCameras']> = key('refreshCameras')
export const CAPABILITIES: InjectionKey<AppState['capabilities']> = key('capabilities')
export const CAMERA_STATUS: InjectionKey<AppState['cameraStatus']> = key('cameraStatus')
export const CAMERA_PHASE: InjectionKey<AppState['cameraPhase']> = key('cameraPhase')
export const WARMUP_ENDS_AT: InjectionKey<AppState['warmupEndsAt']> = key('warmupEndsAt')
export const MAIN_CAMERA: InjectionKey<Readonly<Ref<AppState['mainCamera']['value']>>> = key('mainCamera')
export const GUIDE_CAMERA: InjectionKey<Readonly<Ref<AppState['guideCamera']['value']>>> = key('guideCamera')
export const HAS_GUIDE_CAMERA: InjectionKey<Readonly<Ref<boolean>>> = key('hasGuideCamera')
export const SELECTED_CAMERA_ROLE: InjectionKey<Readonly<Ref<AppState['selectedCameraRole']['value']>>> =
    key('selectedCameraRole')

/** Every key, by the name it is provided under: `provideAppContext` and the tests (`test/mount.ts`) provide by it. */
export const APP_CONTEXT = {
    settings: SETTINGS,
    cameras: CAMERAS,
    selectedCamera: SELECTED_CAMERA,
    eventStream: EVENT_STREAM,
    simulatorEnabled: SIMULATOR_ENABLED,
    refreshSettings: REFRESH_SETTINGS,
    applySettings: APPLY_SETTINGS,
    refreshCameras: REFRESH_CAMERAS,
    capabilities: CAPABILITIES,
    cameraStatus: CAMERA_STATUS,
    cameraPhase: CAMERA_PHASE,
    warmupEndsAt: WARMUP_ENDS_AT,
    mainCamera: MAIN_CAMERA,
    guideCamera: GUIDE_CAMERA,
    hasGuideCamera: HAS_GUIDE_CAMERA,
    selectedCameraRole: SELECTED_CAMERA_ROLE,
} as const

/** What each key provides, by its name. */
export type AppContext = {
    [N in keyof typeof APP_CONTEXT]: (typeof APP_CONTEXT)[N] extends InjectionKey<infer T> ? T : never
}

/** `context`'s values under their keys: what `provide`, or a test mount's `global.provide`, takes. */
export function contextProvides(context: Partial<AppContext>): Record<symbol, unknown> {
    const names = Object.keys(context) as (keyof AppContext)[]
    return Object.fromEntries(names.map((name) => [APP_CONTEXT[name], context[name]]))
}

export function provideAppContext(appState: AppState, eventStream: EventStream) {
    const context: AppContext = {
        settings: appState.settings,
        cameras: appState.cameras,
        selectedCamera: appState.selection,
        eventStream,
        simulatorEnabled: appState.simulatorEnabled,
        refreshSettings: appState.refreshSettings,
        applySettings: appState.applySettings,
        refreshCameras: appState.refreshCameras,
        capabilities: appState.capabilities,
        cameraStatus: appState.cameraStatus,
        cameraPhase: appState.cameraPhase,
        warmupEndsAt: appState.warmupEndsAt,
        mainCamera: appState.mainCamera,
        guideCamera: appState.guideCamera,
        hasGuideCamera: appState.hasGuideCamera,
        selectedCameraRole: appState.selectedCameraRole,
    }
    const provides = contextProvides(context)
    for (const key of Object.getOwnPropertySymbols(provides)) provide(key, provides[key])
}

// One accessor per key, each with the one default a component mounted without it (a test,
// mostly) reads. A default ref is made per call, so two such components share nothing.

export const useSettings = () => inject(SETTINGS, () => readonly(ref(null)), true)
export const useCameras = () => inject(CAMERAS, () => readonly(ref([])), true)
/** The selected camera's id, writable: the camera list selects by setting it. */
export const useSelectedCameraId = () => inject(SELECTED_CAMERA, () => ref(null), true)
/** `null` without a provided stream: there is no stand-in for the server's socket. */
export const useEventStreamContext = () => inject(EVENT_STREAM, null)
export const useSimulatorEnabled = () => inject(SIMULATOR_ENABLED, () => ref(false), true)
export const useRefreshSettings = () => inject(REFRESH_SETTINGS, async () => null)
export const useRefreshCameras = () => inject(REFRESH_CAMERAS, async () => [])
export const useCameraStatus = () => inject<AppContext['cameraStatus']>(CAMERA_STATUS, () => readonly(ref({})), true)
export const useCameraPhase = () => inject<AppContext['cameraPhase']>(CAMERA_PHASE, () => readonly(ref({})), true)
export const useWarmupEndsAt = () => inject<AppContext['warmupEndsAt']>(WARMUP_ENDS_AT, () => readonly(ref({})), true)
export const useMainCamera = () => inject(MAIN_CAMERA, () => computed(() => null), true)
export const useGuideCamera = () => inject(GUIDE_CAMERA, () => computed(() => null), true)
export const useHasGuideCamera = () => inject(HAS_GUIDE_CAMERA, () => computed(() => false), true)
export const useSelectedCameraRole = () => inject(SELECTED_CAMERA_ROLE, () => computed(() => 'main' as const), true)

/**
 * The server's capabilities, plus one flag per Pro feature a control locks on. Without a
 * provided answer every flag is off, so Pro controls show locked rather than missing.
 */
export function useCapabilities() {
    const capabilities = inject(CAPABILITIES, () => readonly(ref(DEFAULT_CAPABILITIES)), true)
    const flag = (read: (c: typeof capabilities.value) => boolean): ComputedRef<boolean> =>
        computed(() => read(capabilities.value))
    return {
        capabilities,
        advancedRejection: flag((c) => c.deep_sky.advanced_rejection),
        rbfBackground: flag((c) => c.deep_sky.rbf_background),
        saturationBoost: flag((c) => c.deep_sky.saturation_boost),
        denoise: flag((c) => c.deep_sky.denoise),
        aiDenoise: flag((c) => c.deep_sky.ai_denoise),
        advancedPlanetary: flag((c) => c.planetary.advanced_stacking),
        astapSolver: flag((c) => c.push_to.astap_solver),
        cometPro: flag((c) => c.comet.pro_stacking),
        debugLogging: flag((c) => c.debug_logging),
    }
}

/**
 * Saves a settings patch and resolves to the server's answer. Below `App.vue` the settings
 * store adopts that answer; a component mounted alone only posts it.
 */
export function useApplySettings() {
    return inject(APPLY_SETTINGS, updateSettings)
}

/** The camera list's entry for the selected camera, or `null`. */
export function useSelectedCamera() {
    const cameras = useCameras()
    const selectedId = useSelectedCameraId()
    return computed(() => findCamera(cameras.value, selectedId.value))
}

/** The entry in `cameras` with this id, or `null`. */
export function findCamera<C extends Pick<Camera, 'id'>>(cameras: readonly C[] | null | undefined, id: string | null) {
    return (cameras ?? []).find((camera) => camera.id === id) ?? null
}

/** Whether a camera entry is the directory-backed simulator. */
export function isSimulator(camera: Pick<Camera, 'provider'> | null | undefined) {
    return camera?.provider === 'Simulator'
}
