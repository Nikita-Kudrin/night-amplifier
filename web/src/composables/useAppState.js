import {readonly, ref} from 'vue'
import {getCapabilities} from './api.js'
import {onEvents, resyncOn} from './useEventStream.js'
import {createSettingsStore} from './settingsStore.js'
import {createCameraStore} from './cameraStore.js'
import {DEFAULT_CAPABILITIES} from '../constants'

/** How long a failed first load waits before it tries again, while `/ws/events` is up. */
export const RETRY_INTERVAL_MS = 2000

/**
 * The app's server state: settings (`settingsStore.js`), cameras (`cameraStore.js`) and
 * capabilities, and the first load that fetches them.
 *
 * The page's stores share one convention: `createX()` builds a store, a module default
 * serves `useX()`, and `__resetForTests()` drops that default so each test starts afresh.
 * Their event handlers subscribe through `connectEvents(stream)`, once, from `App.vue`.
 */
export function createAppState() {
    const loading = ref(true)
    const globalError = ref(null)
    const capabilities = ref(DEFAULT_CAPABILITIES)
    const settingsStore = createSettingsStore()
    const {connectEvents: connectCameraEvents, ...cameras} = createCameraStore({loading: () => loading.value})

    let events = null
    let loadFailed = false
    let retryTimeoutId = null

    async function refreshCapabilities() {
        try {
            capabilities.value = await getCapabilities()
            return capabilities.value
        } catch (e) {
            console.error('Failed to load capabilities:', e)
            throw e
        }
    }

    function stopRetry() {
        clearTimeout(retryTimeoutId)
        retryTimeoutId = null
    }

    // A server that is down has the event socket down too, and the sync its reconnect
    // delivers retries the load (`connectEvents`). Only a server that keeps the socket up
    // while refusing the load is polled.
    function scheduleRetry() {
        stopRetry()
        if (events && !events.connected.value) return
        retryTimeoutId = setTimeout(initializeState, RETRY_INTERVAL_MS)
    }

    async function initializeState() {
        loading.value = true
        globalError.value = null
        stopRetry()

        try {
            await Promise.all([settingsStore.refresh(), cameras.refreshCameras(), refreshCapabilities()])
            loadFailed = false
        } catch (e) {
            loadFailed = true
            globalError.value = e.message
            scheduleRetry()
        } finally {
            loading.value = false
        }
    }

    /** Subscribes the stores to `stream` (`useEventStream()`); the caller's scope owns them. */
    function connectEvents(stream) {
        events = stream
        onEvents(stream, settingsStore.handlers)
        // No snapshot carries the settings, so a sync reads them again. The first only
        // retries a failed load: the first load is reading them already. Subscribed before
        // the cameras, so a retried load is already `loading` when they ask.
        resyncOn(stream, () => (loadFailed ? initializeState() : settingsStore.refresh().catch(() => {})), {
            skipFirst: () => !loadFailed,
        })
        connectCameraEvents(stream)
    }

    function setGlobalError(message) {
        globalError.value = message
    }

    function clearGlobalError() {
        globalError.value = null
    }

    function dispose() {
        stopRetry()
    }


    return {
        // State (readonly to prevent direct mutation)
        settings: settingsStore.settings,
        simulatorEnabled: settingsStore.simulatorEnabled,
        loading: readonly(loading),
        globalError: readonly(globalError),
        capabilities: readonly(capabilities),
        ...cameras,

        // Actions
        refreshSettings: settingsStore.refresh,
        applySettings: settingsStore.apply,
        setSimulatorEnabled: settingsStore.setSimulatorEnabled,
        refreshCapabilities,
        initializeState,
        connectEvents,
        setGlobalError,
        clearGlobalError,
        dispose,
    }
}

let appState = null

/** The page's app state; see `createAppState`. */
export function useAppState() {
    appState ??= createAppState()
    return appState
}

/** For tests: the next `useAppState()` builds a fresh state. */
export function __resetForTests() {
    appState?.dispose()
    appState = null
}
