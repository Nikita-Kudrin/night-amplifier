import {computed, readonly, ref} from 'vue'
import {listCameras} from './api.js'
import {findCamera, isSimulator} from './appContext.js'
import {CAMERA_PHASES} from '../constants'

function withoutKey(map, key) {
    if (!(key in map)) return map
    const next = {...map}
    delete next[key]
    return next
}

/** Whether the snapshot names exactly the cameras the list holds as connected. */
function sameCameraNames(snapshot, list) {
    const named = new Set((snapshot ?? []).map((camera) => camera.name))
    const connected = list.filter((camera) => camera.connected)
    return named.size === connected.length && connected.every((camera) => named.has(camera.name))
}

/**
 * The camera list, the selection, and each camera's live status and lifecycle phase.
 *
 * @param {Object} [options]
 * @param {() => boolean} [options.loading] - Whether the first load is still running, which
 *   fetches the list itself.
 */
export function createCameraStore({loading = () => false} = {}) {
    const cameras = ref([])
    const selectedCameraId = ref(null)
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

    const selectedCamera = computed(() => findCamera(cameras.value, selectedCameraId.value))
    const connectedCameras = computed(() => cameras.value.filter((c) => c.connected))
    const availableCameras = computed(() => cameras.value.filter((c) => !c.connected))
    const mainCamera = computed(() => cameras.value.find((c) => c.connected && c.role === 'main') || null)
    const guideCamera = computed(() => cameras.value.find((c) => c.connected && c.role === 'guide') || null)
    const hasGuideCamera = computed(() => guideCamera.value !== null)
    /**
     * Which camera the settings panel is editing. The selection means "the camera being
     * configured", not "the capture target" — a capture always runs on the imaging camera,
     * which the backend resolves for itself.
     */
    const selectedCameraRole = computed(() => selectedCamera.value?.role || 'main')
    const isSimulatorCamera = computed(() => isSimulator(selectedCamera.value))

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

    function selectCamera(cameraId) {
        selectedCameraId.value = cameraId
    }

    function updateCameraStatus(name, status) {
        cameraStatus.value = {...cameraStatus.value, [name]: status}
    }

    /**
     * A camera's phase from a `camera_phase_changed` event. A camera that disconnected is
     * dropped, so nothing mistakes stale state for a live camera.
     * @param {number} [warmupRemainingS] - seconds a warm-up has left at the latest
     */
    function updateCameraPhase(name, phase, warmupRemainingS) {
        const now = Date.now()
        phaseEventAt.set(name, now)
        if (phase !== CAMERA_PHASES.WARMING_UP) {
            warmupEndsAt.value = withoutKey(warmupEndsAt.value, name)
        } else if (warmupRemainingS != null) {
            warmupEndsAt.value = {...warmupEndsAt.value, [name]: now + warmupRemainingS * 1000}
        }
        if (phase === CAMERA_PHASES.DISCONNECTED) {
            cameraPhase.value = withoutKey(cameraPhase.value, name)
        } else {
            cameraPhase.value = {...cameraPhase.value, [name]: phase}
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
            if (entry.phase === CAMERA_PHASES.DISCONNECTED) continue
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
            if (camera.phase === CAMERA_PHASES.DISCONNECTED) continue
            phases[camera.name] = camera.phase
            if (camera.warmup_remaining_s != null) {
                warmups[camera.name] = requestedAt + camera.warmup_remaining_s * 1000
            }
        }
        cameraPhase.value = phases
        warmupEndsAt.value = warmups
    }

    /** An asynchronously discovered camera, added once by id. */
    function addDiscoveredCamera(camera) {
        if (!cameras.value.some((c) => c.id === camera.id)) {
            cameras.value = [...cameras.value, camera]
        }
    }

    // Each logs its own failure; an event has no caller to hand one to.
    const refreshQuietly = () => refreshCameras().catch(() => {})

    const handlers = {
        camera_status_updated(event) {
            updateCameraStatus(event.name, {
                temperature_c: event.temperature_c,
                cooler_power: event.cooler_power ?? null,
                cooler_on: event.cooler_on,
                dew_heater_on: event.dew_heater_on,
                target_temp_c: event.target_temp_c ?? null,
            })
        },
        camera_phase_changed(event) {
            updateCameraPhase(event.name, event.phase, event.warmup_remaining_s)
        },
        camera_phases(event) {
            replaceCameraPhases(event.cameras)
            // Sent on (re)connect: a camera connected or disconnected while this page was
            // away has no other way to reach the list. Not during the first load, which is
            // fetching the list already.
            if (!loading() && !sameCameraNames(event.cameras, cameras.value)) refreshQuietly()
        },
        camera_disconnected(event) {
            updateCameraPhase(event.name, CAMERA_PHASES.DISCONNECTED)
            refreshQuietly()
        },
        camera_discovered(event) {
            addDiscoveredCamera(event.camera)
        },
    }

    return {
        cameras: readonly(cameras),
        cameraStatus: readonly(cameraStatus),
        cameraPhase: readonly(cameraPhase),
        warmupEndsAt: readonly(warmupEndsAt),
        selectedCameraId: readonly(selectedCameraId),
        // The selected camera's id, writable: the camera list selects by setting it.
        selection: selectedCameraId,
        selectedCamera,
        selectedCameraRole,
        connectedCameras,
        availableCameras,
        mainCamera,
        guideCamera,
        hasGuideCamera,
        isSimulatorCamera,
        refreshCameras,
        selectCamera,
        updateCameraStatus,
        updateCameraPhase,
        replaceCameraPhases,
        addDiscoveredCamera,
        handlers,
    }
}
