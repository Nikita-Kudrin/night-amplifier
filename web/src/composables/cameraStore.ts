import {computed, readonly, ref} from 'vue'
import {listCameras} from './api.js'
import {findCamera, isSimulator} from './appContext.js'
import {onEvents, resyncOn} from './useEventStream.js'
import {CAMERA_PHASES} from '../constants'
import type {Camera, CameraPhase, CameraRole} from './api.types.js'
import type {CameraPhaseEntry, CameraStatusUpdatedEvent} from './api.events.js'
import type {EventHandlers} from './events/handlers.js'
import type {EventStream} from './useEventStream.js'

/** A cooled camera's latest status sample, as `camera_status_updated` reports it. */
export type CameraStatus = Pick<CameraStatusUpdatedEvent, 'temperature_c' | 'cooler_on' | 'dew_heater_on'> & {
    cooler_power: number | null
    target_temp_c: number | null
}

/** By camera name: a camera the map does not hold is not connected, or has said nothing yet. */
export type ByCameraName<T> = Partial<Record<string, T>>

function withoutKey<T>(map: ByCameraName<T>, key: string): ByCameraName<T> {
    if (!(key in map)) return map
    const next = {...map}
    delete next[key]
    return next
}

/**
 * The camera list, the selection, and each camera's live status and lifecycle phase.
 * `loading` says whether the first load is still running, which fetches the list itself.
 */
export function createCameraStore({loading = () => false}: {loading?: () => boolean} = {}) {
    const cameras = ref<Camera[]>([])
    let listed = false
    const selectedCameraId = ref<string | null>(null)
    // Latest live camera status keyed by camera name (cooled cameras)
    const cameraStatus = ref<ByCameraName<CameraStatus>>({})
    // Camera lifecycle phase keyed by camera name: 'idle' | 'precooling' | 'capturing' |
    // 'guiding' | 'warming_up' | 'recovering'. Seeded from the camera list and the server's
    // `camera_phases` snapshot, then kept current by `camera_phase_changed` events: built from
    // events alone, a page opened mid-session — or a phone waking up — had no phases and
    // offered "Start guide" for a loop that was running (2026-09-20).
    const cameraPhase = ref<ByCameraName<CameraPhase>>({})
    // When a warm-up is cut short at the latest (ms since epoch), keyed by camera name.
    const warmupEndsAt = ref<ByCameraName<number>>({})
    // When each camera's phase last came from an event, so a camera-list response that left
    // the server before that event cannot overwrite it.
    const phaseEventAt = new Map<string, number>()

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
    const selectedCameraRole = computed<CameraRole>(() => selectedCamera.value?.role || 'main')
    const isSimulatorCamera = computed(() => isSimulator(selectedCamera.value))

    /**
     * `discover: false` reads what the server's last enumeration found instead of
     * enumerating the USB bus again.
     */
    async function refreshCameras({discover = true}: {discover?: boolean} = {}): Promise<Camera[]> {
        try {
            const requestedAt = Date.now()
            cameras.value = await listCameras({discover})
            listed = true
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

    function selectCamera(cameraId: string | null) {
        selectedCameraId.value = cameraId
    }

    function updateCameraStatus(name: string, status: CameraStatus) {
        cameraStatus.value = {...cameraStatus.value, [name]: status}
    }

    /**
     * A camera's phase from a `camera_phase_changed` event. A camera that disconnected is
     * dropped, so nothing mistakes stale state for a live camera. `warmupRemainingS` is the
     * seconds a warm-up has left at the latest.
     */
    function updateCameraPhase(name: string, phase: CameraPhase, warmupRemainingS?: number | null) {
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
     */
    function replaceCameraPhases(entries: CameraPhaseEntry[]) {
        const now = Date.now()
        for (const name of Object.keys(cameraPhase.value)) phaseEventAt.set(name, now)
        const phases: ByCameraName<CameraPhase> = {}
        const warmups: ByCameraName<number> = {}
        for (const entry of entries) {
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
    function seedCameraPhases(list: Camera[], requestedAt: number) {
        const stale = (name: string) => (phaseEventAt.get(name) ?? -Infinity) < requestedAt
        const phases: ByCameraName<CameraPhase> = {}
        const warmups: ByCameraName<number> = {}
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
    function addDiscoveredCamera(camera: Camera) {
        if (!cameras.value.some((c) => c.id === camera.id)) {
            cameras.value = [...cameras.value, camera]
        }
    }

    let listing: Promise<void> | null = null
    let relist = false
    let relistDiscovers = false

    /**
     * The list again for an event, one request at a time: asked during one, it reads once
     * more after it, since that answer may predate the event, enumerating if any of the
     * askers wanted that. `refreshCameras` logs a failure; an event has no caller to hand it to.
     */
    function refreshQuietly({discover = true}: {discover?: boolean} = {}) {
        relist = true
        relistDiscovers ||= discover
        listing ??= (async () => {
            while (relist) {
                const enumerate = relistDiscovers
                relist = false
                relistDiscovers = false
                await refreshCameras({discover: enumerate}).catch(() => {})
            }
            listing = null
        })()
        return listing
    }

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
        },
        // Any client's connect, this page's included: the list is the only place a new
        // camera's id and role reach, so another tab once showed no guide camera at all.
        camera_connected: () => refreshQuietly(),
        camera_disconnected(event) {
            updateCameraPhase(event.name, CAMERA_PHASES.DISCONNECTED)
            refreshQuietly()
        },
        camera_discovered(event) {
            addDiscoveredCamera(event.camera)
        },
    } satisfies EventHandlers

    /**
     * Subscribes to `stream` (`useEventStream()`); the caller's scope owns it. A sync lists
     * the cameras again, since a connect or disconnect may have been missed meanwhile; the
     * first only when nothing has listed them yet or is listing them. Without enumerating:
     * a sync follows every reconnect and lag of every page, and each enumeration opens
     * every idle QHY/ZWO device for seconds, mid-capture too.
     */
    function connectEvents(stream: EventStream) {
        onEvents(stream, handlers)
        resyncOn(stream, () => refreshQuietly({discover: false}), {skipFirst: () => listed || loading()})
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
        connectEvents,
    }
}
