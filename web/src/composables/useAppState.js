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
    debug_logging: false,
})
// Latest live camera status keyed by camera name (cooled cameras)
const cameraStatus = ref({})
// Camera lifecycle phase keyed by camera name: 'idle' | 'precooling' | 'capturing' |
// 'guiding' | 'warming_up' | 'recovering'. Seeded from the camera list and the server's
// `camera_phases` snapshot, then kept current by `camera_phase_changed` events: built from
// events alone, a page opened mid-session — or a phone waking up — had no phases and
// offered "Start guide" for a loop that was running (2026-09-20).
const cameraPhase = ref({})
// When a warm-up is cut short at the latest (ms since epoch), keyed by camera name.
const warmupEndsAt = ref({})
// When each camera's phase last came from an event, so a camera-list response that left
// the server before that event cannot overwrite it.
const phaseEventAt = new Map()

// Retry state
let retryTimeoutId = null
const RETRY_INTERVAL_MS = 2000

// Computed getters
const selectedCamera = computed(
    () => cameras.value.find((c) => c.id === selectedCameraId.value) || null
)

const connectedCameras = computed(() => cameras.value.filter((c) => c.connected))

const availableCameras = computed(() => cameras.value.filter((c) => !c.connected))

const mainCamera = computed(() => cameras.value.find((c) => c.connected && c.role === 'main') || null)

const guideCamera = computed(
    () => cameras.value.find((c) => c.connected && c.role === 'guide') || null
)

const hasGuideCamera = computed(() => guideCamera.value !== null)

/**
 * Which camera the settings panel is editing.
 *
 * `selectedCameraId` means "the camera being configured", not "the capture target" —
 * a capture always runs on the imaging camera, which the backend resolves for itself.
 */
const selectedCameraRole = computed(() => selectedCamera.value?.role || 'main')

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
        const requestedAt = Date.now()
        cameras.value = await listCameras()
        seedCameraPhases(cameras.value, requestedAt)
        // Drop a selection whose camera has gone, so the settings panel never edits a
        // camera that is no longer there.
        if (selectedCameraId.value && !cameras.value.some((c) => c.id === selectedCameraId.value && c.connected)) {
            selectedCameraId.value = null
        }
        // Auto-select the imaging camera when nothing is selected; a guide camera is a
        // deliberate choice, never the default one.
        if (!selectedCameraId.value) {
            const preferred =
                cameras.value.find((c) => c.connected && c.role === 'main') ||
                cameras.value.find((c) => c.connected)
            if (preferred) {
                selectedCameraId.value = preferred.id
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
 * @param {string} name
 * @param {string} phase
 * @param {number} [warmupRemainingS] - seconds a warm-up has left at the latest
 */
function updateCameraPhase(name, phase, warmupRemainingS) {
    const now = Date.now()
    phaseEventAt.set(name, now)
    if (phase !== 'warming_up') {
        warmupEndsAt.value = withoutKey(warmupEndsAt.value, name)
    } else if (warmupRemainingS != null) {
        warmupEndsAt.value = {...warmupEndsAt.value, [name]: now + warmupRemainingS * 1000}
    }
    if (phase === 'disconnected') {
        cameraPhase.value = withoutKey(cameraPhase.value, name)
    } else {
        cameraPhase.value = {
            ...cameraPhase.value,
            [name]: phase,
        }
    }
}

/**
 * Replace every phase with the server's `camera_phases` snapshot, sent when the event
 * socket connects and after it fell behind. Newer than anything held, so it wins outright.
 * @param {Array<{name: string, phase: string, warmup_remaining_s?: number}>} entries
 */
function replaceCameraPhases(entries) {
    const now = Date.now()
    for (const name of Object.keys(cameraPhase.value)) phaseEventAt.set(name, now)
    const phases = {}
    const warmups = {}
    for (const entry of entries ?? []) {
        phaseEventAt.set(entry.name, now)
        if (entry.phase === 'disconnected') continue
        phases[entry.name] = entry.phase
        if (entry.warmup_remaining_s != null) warmups[entry.name] = now + entry.warmup_remaining_s * 1000
    }
    cameraPhase.value = phases
    warmupEndsAt.value = warmups
}

/**
 * Take phases from a camera-list response, except for cameras an event has updated since
 * the request went out: the event is newer.
 */
function seedCameraPhases(list, requestedAt) {
    const stale = (name) => (phaseEventAt.get(name) ?? -Infinity) < requestedAt
    const phases = {}
    const warmups = {}
    for (const [name, phase] of Object.entries(cameraPhase.value)) {
        if (!stale(name)) phases[name] = phase
    }
    for (const [name, endsAt] of Object.entries(warmupEndsAt.value)) {
        if (!stale(name)) warmups[name] = endsAt
    }
    for (const camera of list) {
        if (!camera.connected || !camera.phase || !stale(camera.name)) continue
        if (camera.phase === 'disconnected') continue
        phases[camera.name] = camera.phase
        if (camera.warmup_remaining_s != null) {
            warmups[camera.name] = requestedAt + camera.warmup_remaining_s * 1000
        }
    }
    cameraPhase.value = phases
    warmupEndsAt.value = warmups
}

function withoutKey(map, key) {
    if (!(key in map)) return map
    const next = {...map}
    delete next[key]
    return next
}

/**
 * Merge an asynchronously discovered camera into the list.
 * Avoids duplicates by checking the camera id.
 */
function addDiscoveredCamera(camera) {
    const exists = cameras.value.some((c) => c.id === camera.id)
    if (!exists) {
        cameras.value = [...cameras.value, camera]
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
        warmupEndsAt: readonly(warmupEndsAt),

        // Computed
        selectedCamera,
        selectedCameraRole,
        connectedCameras,
        availableCameras,
        mainCamera,
        guideCamera,
        hasGuideCamera,
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
        replaceCameraPhases,
        addDiscoveredCamera,

        // Direct refs for provide/inject compatibility (temporary)
        _settingsRef: settings,
        _camerasRef: cameras,
        _selectedCameraIdRef: selectedCameraId,
        _cameraStatusRef: cameraStatus,
        _cameraPhaseRef: cameraPhase,
        _warmupEndsAtRef: warmupEndsAt,
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
