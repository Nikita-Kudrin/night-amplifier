import {ref, computed, readonly} from 'vue'
import {getSettings, listCameras, getCapabilities} from './api.js'

/**
 * Centralized application state management
 * Single source of truth for global state
 */

// Core state
const settings = ref(null)
const cameras = ref([])
const selectedCameraId = ref(null)
const loading = ref(true)
const globalError = ref(null)
const simulatorEnabled = ref(false)
const capabilities = ref({
    has_pro: false,
    deep_sky: {advanced_rejection: false, rbf_background: false},
    planetary: {advanced_stacking: false},
    push_to: {astap_solver: false},
})
// Latest live camera status keyed by camera name (cooled cameras)
const cameraStatus = ref({})
// Camera lifecycle phase keyed by camera name: 'idle' | 'precooling' | 'capturing' | 'warming_up' | 'disconnected'
const cameraPhase = ref({})

// Retry state
let retryTimeoutId = null
const RETRY_INTERVAL_MS = 2000

// Computed getters
const selectedCamera = computed(
    () => cameras.value.find((c) => c.id === selectedCameraId.value) || null
)

const connectedCameras = computed(() => cameras.value.filter((c) => c.connected))

const availableCameras = computed(() => cameras.value.filter((c) => !c.connected))

const isSimulatorCamera = computed(() => selectedCamera.value?.provider === 'Simulator')

/**
 * Refresh settings from server
 */
async function refreshSettings() {
    try {
        settings.value = await getSettings()
        // Sync simulatorEnabled with server setting
        if (settings.value?.use_simulated_camera !== undefined) {
            simulatorEnabled.value = settings.value.use_simulated_camera
        }
        return settings.value
    } catch (e) {
        console.error('Failed to load settings:', e)
        throw e
    }
}

/**
 * Refresh cameras list from server
 */
async function refreshCameras() {
    try {
        cameras.value = await listCameras()
        // Auto-select first connected camera if none selected
        if (!selectedCameraId.value) {
            const connected = cameras.value.find((c) => c.connected)
            if (connected) {
                selectedCameraId.value = connected.id
            }
        }
        return cameras.value
    } catch (e) {
        console.error('Failed to load cameras:', e)
        throw e
    }
}

/**
 * Refresh capabilities from server
 */
async function refreshCapabilities() {
    try {
        capabilities.value = await getCapabilities()
        return capabilities.value
    } catch (e) {
        console.error('Failed to load capabilities:', e)
        throw e
    }
}

/**
 * Select a camera by ID
 */
function selectCamera(cameraId) {
    selectedCameraId.value = cameraId
}

/**
 * Stop any pending retry
 */
function stopRetry() {
    if (retryTimeoutId) {
        clearTimeout(retryTimeoutId)
        retryTimeoutId = null
    }
}

/**
 * Schedule a retry attempt
 */
function scheduleRetry() {
    stopRetry()
    retryTimeoutId = setTimeout(() => {
        initializeState()
    }, RETRY_INTERVAL_MS)
}

/**
 * Initialize application state
 */
async function initializeState() {
    loading.value = true
    globalError.value = null
    stopRetry()

    try {
        await Promise.all([refreshSettings(), refreshCameras(), refreshCapabilities()])
    } catch (e) {
        globalError.value = e.message
        scheduleRetry()
    } finally {
        loading.value = false
    }
}

/**
 * Set global error
 */
function setGlobalError(message) {
    globalError.value = message
}

/**
 * Clear global error
 */
function clearGlobalError() {
    globalError.value = null
}

/**
 * Toggle simulator mode
 */
function setSimulatorEnabled(enabled) {
    simulatorEnabled.value = enabled
}

/**
 * Update the cached camera status map from a `camera_status_updated` event.
 */
function updateCameraStatus(name, status) {
    cameraStatus.value = {
        ...cameraStatus.value,
        [name]: status,
    }
}

/**
 * Update the cached camera phase map from a `camera_phase_changed` event.
 * When a camera transitions to 'disconnected' the entry is dropped so UI
 * components don't mistake stale state for a live camera.
 */
function updateCameraPhase(name, phase) {
    if (phase === 'disconnected') {
        const next = {...cameraPhase.value}
        delete next[name]
        cameraPhase.value = next
    } else {
        cameraPhase.value = {
            ...cameraPhase.value,
            [name]: phase,
        }
    }
}

/**
 * Composable hook for app state
 */
export function useAppState() {
    return {
        // State (readonly to prevent direct mutation)
        settings: readonly(settings),
        cameras: readonly(cameras),
        selectedCameraId: readonly(selectedCameraId),
        loading: readonly(loading),
        globalError: readonly(globalError),
        simulatorEnabled,
        capabilities: readonly(capabilities),
        cameraStatus: readonly(cameraStatus),
        cameraPhase: readonly(cameraPhase),

        // Computed
        selectedCamera,
        connectedCameras,
        availableCameras,
        isSimulatorCamera,

        // Actions
        refreshSettings,
        refreshCameras,
        refreshCapabilities,
        selectCamera,
        initializeState,
        setGlobalError,
        clearGlobalError,
        setSimulatorEnabled,
        updateCameraStatus,
        updateCameraPhase,

        // Direct refs for provide/inject compatibility (temporary)
        _settingsRef: settings,
        _camerasRef: cameras,
        _selectedCameraIdRef: selectedCameraId,
        _cameraStatusRef: cameraStatus,
        _cameraPhaseRef: cameraPhase,
    }
}

// Singleton instance for global access
let instance = null

export function getAppState() {
    if (!instance) {
        instance = useAppState()
    }
    return instance
}
