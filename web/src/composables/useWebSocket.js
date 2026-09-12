import {ref, onUnmounted, shallowRef, computed, toValue, watch} from 'vue'
import {WS_RECONNECT} from '../constants'
import {decodeFrame} from '../utils/frameDecoder.js'
import {getPushToStatus} from './api.js'

/**
 * WebSocket connection manager composable
 * @param {string} path - WebSocket path (e.g., '/ws/events')
 * @param {object} options - Connection options
 * @returns {object} WebSocket state and methods
 */
export function useWebSocket(path, options = {}) {
    const {
        autoConnect = true,
        reconnect = true,
        reconnectInterval = WS_RECONNECT.interval,
        maxReconnectInterval = WS_RECONNECT.maxInterval,
        onMessage = null,
        onOpen = null,
        onClose = null,
        onError = null,
    } = options

    const connected = ref(false)
    const error = ref(null)
    const reconnectAttempts = ref(0)

    let ws = null
    let reconnectTimer = null
    let pingTimer = null

    /**
     * Get the WebSocket URL
     */
    function getUrl() {
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
        const host = window.location.host
        // Resolved per connect, not captured once: the live view swaps between the
        // imaging and guide sources on one socket by changing this.
        return `${protocol}//${host}${toValue(path)}`
    }

    /**
     * Connect to WebSocket
     */
    function connect() {
        if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
            return
        }

        error.value = null

        try {
            // Held locally as well as in `ws` so every handler below can tell whether it
            // still speaks for the current connection. Without that, the socket closed by
            // `disconnect()` — or by the live view swapping camera sources — fires
            // `onclose` after its replacement is already open, and schedules a reconnect
            // that opens a second socket nobody asked for. Harmless while retries were
            // capped at ten; not once they are unbounded.
            const socket = new WebSocket(getUrl())
            ws = socket

            socket.onopen = () => {
                if (ws !== socket) return
                connected.value = true
                reconnectAttempts.value = 0
                startPing()
                onOpen?.()
            }

            socket.onclose = (event) => {
                if (ws !== socket) return
                connected.value = false
                stopPing()
                onClose?.(event)

                if (reconnect) {
                    scheduleReconnect()
                }
            }

            socket.onerror = (event) => {
                if (ws !== socket) return
                error.value = 'WebSocket error'
                onError?.(event)
            }

            socket.onmessage = (event) => {
                if (ws !== socket) return
                if (event.data === 'pong') {
                    return // Ignore ping responses
                }
                onMessage?.(event)
            }
        } catch (e) {
            error.value = e.message
            if (reconnect) {
                scheduleReconnect()
            }
        }
    }

    /**
     * Disconnect from WebSocket
     */
    function disconnect() {
        clearTimeout(reconnectTimer)
        stopPing()

        if (ws) {
            const socket = ws
            // Cleared before closing, so the socket's own `onclose` sees itself
            // superseded and does not schedule a reconnect against a deliberate close.
            ws = null
            socket.close()
        }

        connected.value = false
    }

    /**
     * Send a message
     * @param {string|ArrayBuffer|Blob} data - Data to send
     * @returns {boolean} Whether the socket was open and the data went out.
     *   Callers that have to know a message arrived — the viewport report, whose
     *   loss silently pins a stream to the server's smallest tier — must check
     *   this rather than assume a call is a delivery.
     */
    function send(data) {
        if (!ws || ws.readyState !== WebSocket.OPEN) {
            return false
        }
        ws.send(data)
        return true
    }

    /**
     * Delay before retry `attempt` (1-based): doubles from `reconnectInterval` up to a
     * `maxReconnectInterval` ceiling.
     */
    function backoffDelay(attempt) {
        return Math.min(reconnectInterval * 2 ** (attempt - 1), maxReconnectInterval)
    }

    /**
     * Schedule a reconnection attempt. Never gives up.
     *
     * The attempt count used to be capped, which meant a client that could not reach the
     * server for thirty seconds stayed dead until someone reloaded the page. On the
     * kiosk display that runs `/eyepiece_quality` there is no keyboard to reload with, so
     * a restarted server left a black screen at the telescope for the rest of the night.
     * The backoff ceiling, not a cap, is what keeps a server that is down until morning
     * from being polled every second.
     */
    function scheduleReconnect() {
        clearTimeout(reconnectTimer)
        reconnectAttempts.value++
        reconnectTimer = setTimeout(connect, backoffDelay(reconnectAttempts.value))
    }

    /**
     * Start ping interval to keep connection alive
     */
    function startPing() {
        stopPing()
        pingTimer = setInterval(() => {
            send('ping')
        }, WS_RECONNECT.pingInterval)
    }

    /**
     * Stop ping interval
     */
    function stopPing() {
        clearInterval(pingTimer)
    }

    // Auto-connect if enabled
    if (autoConnect) {
        connect()
    }

    // Cleanup on unmount
    onUnmounted(() => {
        disconnect()
    })

    return {
        connected,
        error,
        reconnectAttempts,
        connect,
        disconnect,
        send,
    }
}

/**
 * WebSocket composable for server events
 * @returns {object} Event stream state and methods
 */
export function useEventStream() {
    const lastEvent = ref(null)
    const captureState = ref('Idle')
    const frameCount = ref(0)
    const stackedCount = ref(0)
    const rejectedCount = ref(0)
    const droppedCount = ref(0)
    // The denominator the count needs: 40 drops is a ruined evening at 30 s subs and a
    // rounding error at 100 ms.
    const deliveredCount = ref(0)
    const lastError = ref(null)
    const diskWriterWarning = ref(null)
    const unresponsiveWarning = ref(null)
    // Which camera the warning is about — `{name, role}`, role null when the event had
    // none — so its recovery can clear it.
    const unresponsiveCamera = ref(null)
    const resumeNotice = ref(null)
    // Why the server switched Focus/Finder mode off on its own; stays until dismissed.
    const focusModeNotice = ref(null)

    // Push-To state
    const pushDirection = ref(null)
    const currentTarget = ref(null)
    const plateSolving = ref({inProgress: false, targetName: null, lastResult: null, stage: null})

    // Why Push-To is idle, when it is idle for a reason the user can act on
    // (no target, ASTAP not installed, waiting out a retry backoff). The server
    // sends this only on a transition, so it is safe to render directly.
    const pushToBlocked = ref(null)

    const solvingMessage = computed(() => {
        const {inProgress, lastResult, targetName, stage} = plateSolving.value
        const targetSuffix = targetName ? `: ${targetName}` : ''

        if (inProgress) {
            // The step counter, not the strategy behind it. A label like "last solve
            // (this session) + last known pointing" is a diagnostic that overran the
            // status bar and pushed the one thing the user is looking for -- the object
            // name -- off the end. It stays in `stage.label` and in the server log.
            // The counter alone still separates a slow solve from a hung one.
            if (stage && stage.total > 1) {
                return `Searching (step ${stage.attempt}/${stage.total})${targetSuffix}`
            }
            return targetName ? `Searching${targetSuffix}` : 'Updating position...'
        }

        // A live blocker outranks the last verdict, because it is *newer*. The server
        // sends one only on a transition, so its presence means the situation has
        // moved on since that verdict -- the scope is being pushed, the view has not
        // settled, a retry is being held off. Ordering these the other way round is
        // what left "Found: M31" on screen for the whole time the user was pushing
        // away from M31, and hid the retry countdown behind "Failed to find M31"
        // forever, since nothing ever cleared `lastResult`.
        if (pushToBlocked.value) {
            return pushToBlocked.value
        }

        if (lastResult === 'success') {
            return targetName ? `Found${targetSuffix}` : 'Position updated'
        }

        if (lastResult === 'failed') {
            return targetName ? `Failed to find${targetSuffix}` : 'Update failed'
        }

        if (lastResult === 'cancelled') {
            return 'Solving cancelled'
        }

        return null
    })

    // ASTAP installation state
    const astapInstallProgress = ref(null)

    // Catalog installation state
    const catalogInstallProgress = ref(null)

    // Why the most recent frame was kept out of the stack. Held rather than
    // cleared per frame, so the reason a rejection burst is happening stays
    // readable while good frames come and go between the bad ones.
    const lastRejectionReason = ref(null)

    function handleFrameEvent(data) {
        frameCount.value = data.frame_number
        stackedCount.value = data.stacked_count
        rejectedCount.value = data.rejected_count
        if (data.rejection_reason) {
            lastRejectionReason.value = data.rejection_reason
        }
    }

    const eventHandlers = {
        state_changed(data) {
            // A capture resuming after a camera reconnect carries its session on; only a
            // fresh start zeroes the counters.
            const resuming = captureState.value === 'Recovering'
            captureState.value = data.state
            if (data.state === 'Starting' && !resuming) {
                frameCount.value = 0
                stackedCount.value = 0
                rejectedCount.value = 0
                droppedCount.value = 0
                deliveredCount.value = 0
                lastRejectionReason.value = null
            }
        },
        frame_captured: handleFrameEvent,
        frame_rejected: handleFrameEvent,
        error(data) {
            lastError.value = data.message
        },
        camera_persistently_unresponsive(data) {
            // The server sends `name`, not `camera_name` — see the shape pinned
            // by src/server/tests/events.rs.
            unresponsiveWarning.value = `${data.name} has stopped responding.`
            unresponsiveCamera.value = {name: data.name, role: null}
        },
        camera_reconnecting(data) {
            unresponsiveWarning.value =
                `Reconnecting to ${data.name} — attempt ${data.attempt} of ${data.of}.`
            unresponsiveCamera.value = {name: data.name, role: data.role ?? null}
        },
        camera_phase_changed(data) {
            // A camera that came back needs no warning, and a guide camera or an idle
            // one gets no `capture_resumed` to clear it. Matched on role too where both
            // sides carry one: two bodies of one model share a name.
            const healthy = data.phase !== 'recovering' && data.phase !== 'disconnected'
            const warned = unresponsiveCamera.value
            const sameCamera = warned?.name === data.name
                && (!warned.role || !data.role || warned.role === data.role)
            if (healthy && sameCamera) {
                unresponsiveWarning.value = null
                unresponsiveCamera.value = null
            }
        },
        camera_reconnect_failed(data) {
            unresponsiveWarning.value =
                `Could not bring ${data.name} back after ${data.attempts} attempts: ${data.reason}.`
        },
        capture_resumed(data) {
            unresponsiveWarning.value = null
            lastError.value = null
            resumeNotice.value =
                `${data.name} is back. Capture resumed with ${data.stacked_count} frames still stacked.`
        },
        focus_mode_left() {
            focusModeNotice.value =
                'Focus/Finder mode turned off: stacking needs the sensor corrections it holds off.'
        },
        settings_updated() { /* components should refresh */
        },
        disk_writer_warning(data) {
            diskWriterWarning.value = data.queue_depth
        },
        disk_writer_warning_cleared() {
            diskWriterWarning.value = null
        },
        frame_dropped(data) {
            droppedCount.value = data.dropped_count
            deliveredCount.value = data.delivered_count ?? 0
        },
        plate_solving_started(data) {
            plateSolving.value = {
                inProgress: true, targetName: data.target_name, lastResult: null, stage: null,
            }
            // Solving has resumed, so whatever was holding it up no longer is.
            pushToBlocked.value = null
        },
        plate_solving_progress(data) {
            // A cold solve works down several strategies and the last one can run for
            // a minute. Without the stage the indicator is indistinguishable from a hang.
            plateSolving.value = {
                ...plateSolving.value,
                inProgress: true,
                stage: {label: data.stage, attempt: data.attempt, total: data.total},
            }
        },
        position_solved() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: 'success', stage: null,
            }
        },
        position_solve_failed() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: 'failed', stage: null,
            }
        },
        plate_solving_cancelled() {
            // Distinct from a failure: the user asked for this, and the last known
            // position is still good. Showing it as "Failed to find M31" made an
            // ordinary settings save look like a solver problem.
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: 'cancelled', stage: null,
            }
        },
        plate_solving_restarted() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName,
                lastResult: null, stage: null,
            }
            pushToBlocked.value = null
        },
        push_to_blocked(data) {
            pushToBlocked.value = data.reason ?? null
        },
        push_direction_updated(data) {
            pushDirection.value = {
                angleDeg: data.angle_deg,
                distanceDeg: data.distance_deg,
                directionHint: data.direction_hint,
                isClose: data.is_close,
                fovDeg: data.fov_deg || 0,
            }
        },
        target_changed(data) {
            currentTarget.value = {
                name: data.name,
                designation: data.designation,
                ra_degrees: data.ra_degrees,
                dec_degrees: data.dec_degrees,
            }
        },
        target_cleared() {
            currentTarget.value = null
            pushDirection.value = null
            clearPlateSolving()
        },
        astap_install_starting(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'starting',
                percent: null, bytesDownloaded: 0, totalBytes: null,
                overallPercent: null, error: null,
            }
        },
        astap_install_progress(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'downloading',
                percent: data.percent, bytesDownloaded: data.bytes_downloaded,
                totalBytes: data.total_bytes, stageName: data.stage,
                overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_extracting(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'extracting',
                percent: data.progress, stageName: data.stage,
                overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_completed(data) {
            astapInstallProgress.value = {
                component: data.component, stage: 'completed',
                stageName: data.stage, overallPercent: data.overall_percent, error: null,
            }
        },
        astap_install_failed(data) {
            astapInstallProgress.value = {component: data.component, stage: 'failed', error: data.error}
        },
        catalog_install_starting() {
            catalogInstallProgress.value = {
                stage: 'starting', fileName: '',
                percent: null, bytesDownloaded: 0, totalBytes: null, error: null,
            }
        },
        catalog_install_progress(data) {
            catalogInstallProgress.value = {
                stage: 'downloading', fileName: data.file_name,
                percent: data.percent, bytesDownloaded: data.bytes_downloaded,
                totalBytes: data.total_bytes, error: null,
            }
        },
        catalog_file_completed(data) {
            catalogInstallProgress.value = {
                ...catalogInstallProgress.value,
                fileName: data.file_name, stage: 'file_completed',
            }
        },
        catalog_install_completed(data) {
            catalogInstallProgress.value = {stage: 'completed', object_count: data.object_count, error: null}
        },
        catalog_install_failed(data) {
            catalogInstallProgress.value = {stage: 'failed', error: data.error}
        },
    }

    const {connected, error, connect, disconnect} = useWebSocket('/ws/events', {
        onOpen: async () => {
            try {
                // Fetch the initial state upon connection
                const status = await getPushToStatus()
                currentTarget.value = status.current_target || null
                if (status.direction) {
                    pushDirection.value = {
                        angleDeg: status.direction.angle_deg,
                        distanceDeg: status.direction.distance_deg,
                        directionHint: status.direction.direction_hint,
                        isClose: status.direction.is_close,
                        fovDeg: status.direction.fov_deg || 0,
                    }
                } else {
                    pushDirection.value = null
                }
            } catch {
                // Ignore
            }
        },
        onMessage: (event) => {
            try {
                const data = JSON.parse(event.data)
                lastEvent.value = data
                eventHandlers[data.type]?.(data)
            } catch (e) {
                console.error('Failed to parse event:', e)
            }
        },
    })

    function clearError() {
        lastError.value = null
    }

    function clearDiskWriterWarning() {
        diskWriterWarning.value = null
    }

    function clearPushDirection() {
        pushDirection.value = null
    }

    function clearPlateSolving() {
        plateSolving.value = {inProgress: false, targetName: null, lastResult: null, stage: null}
        pushToBlocked.value = null
    }

    function clearAstapInstallProgress() {
        astapInstallProgress.value = null
    }

    function clearCatalogInstallProgress() {
        catalogInstallProgress.value = null
    }

    function clearUnresponsiveWarning() {
        unresponsiveWarning.value = null
    }

    function clearResumeNotice() {
        resumeNotice.value = null
    }

    function clearFocusModeNotice() {
        focusModeNotice.value = null
    }

    return {
        connected,
        error,
        lastEvent,
        captureState,
        frameCount,
        stackedCount,
        rejectedCount,
        lastRejectionReason,
        droppedCount,
        deliveredCount,
        lastError,
        diskWriterWarning,
        unresponsiveWarning,
        resumeNotice,
        focusModeNotice,
        pushDirection,
        currentTarget,
        plateSolving,
        pushToBlocked,
        solvingMessage,
        astapInstallProgress,
        catalogInstallProgress,
        clearError,
        clearDiskWriterWarning,
        clearUnresponsiveWarning,
        clearResumeNotice,
        clearFocusModeNotice,
        clearPushDirection,
        clearPlateSolving,
        clearAstapInstallProgress,
        clearCatalogInstallProgress,
        connect,
        disconnect,
    }
}

/**
 * WebSocket composable for high-quality RGB16 image streaming: receives RGB16+LZ4
 * frames, exposing `rgb16Data` (raw 16-bit RGB for WebGL) and `dimensions`.
 *
 * @param {object} options - Stream options
 * @param {string} options.endpoint - WebSocket endpoint (default: '/ws/stream')
 * @param {number|null} options.width - Initial viewport width, until the caller reports a real one
 * @param {number|null} options.height - Initial viewport height, until the caller reports a real one
 * @returns {object} Image stream state and methods
 */
export function useImageStream(options = {}) {
    // May be a plain string, a ref or a getter — the live view passes a getter so the
    // Guide camera toggle can move this socket to the other source.
    const endpointSource = options.endpoint ?? '/ws/stream'
    const resolveEndpoint = () => toValue(endpointSource) || '/ws/stream'

    // Selects the decoder, not whether resolution can be negotiated: both the
    // JPEG and the lossless endpoints size their output from the client's report.
    const isDynamicJpeg = !resolveEndpoint().startsWith('/ws/eyepiece_quality')

    // Use shallowRef for large binary data to avoid deep reactivity overhead
    const frameData = shallowRef(null)
    const isJpeg = ref(isDynamicJpeg)
    const dimensions = ref({width: 0, height: 0})
    const frameNumber = ref(0)
    const fps = ref(0)
    const decodeError = ref(null)

    let framesSinceLastFPS = 0
    let fpsTimer = null

    /**
     * The viewport this client wants, kept across reconnects.
     *
     * The server registers a fresh tier for every connection, so a socket that
     * reconnects without re-reporting is served at the server's default — the
     * *smallest* tier. This has to survive the socket, not live on it.
     */
    let viewport = normalizeViewport(options.width, options.height)
    /** What the current socket has actually been told, so a repeat is not re-sent. */
    let sentViewport = null

    function normalizeViewport(w, h) {
        if (!(w > 0 && h > 0)) return null
        return {width: Math.round(w), height: Math.round(h)}
    }

    function sameViewport(a, b) {
        return a !== null && b !== null && a.width === b.width && a.height === b.height
    }

    /**
     * Report the current viewport to the server, unless this socket already has it.
     *
     * Only records the send when the socket actually took it: a report made
     * before the socket opened is a no-op, and treating it as delivered is what
     * would leave the stream stuck at the default tier.
     */
    function pushViewport() {
        if (viewport === null || sameViewport(sentViewport, viewport)) return
        if (!send(JSON.stringify(viewport))) return
        sentViewport = viewport
    }

    /**
     * Clear frame data to reset the live view
     * Called when starting a new capture session
     */
    function clearFrameData() {
        frameData.value = null
        dimensions.value = {width: 0, height: 0}
        frameNumber.value = 0
        fps.value = 0
        framesSinceLastFPS = 0
        decodeError.value = null
    }

    function startFpsTimer() {
        if (fpsTimer) return
        fpsTimer = setInterval(() => {
            fps.value = Math.round(framesSinceLastFPS / 3)
            framesSinceLastFPS = 0
        }, 3000)
    }

    function stopFpsTimer() {
        if (fpsTimer) {
            clearInterval(fpsTimer)
            fpsTimer = null
        }
    }

    const {connected, error, connect, disconnect, send} = useWebSocket(resolveEndpoint, {
        onOpen: () => {
            startFpsTimer()
            // A new socket knows nothing about the viewport, whichever stream
            // family it belongs to.
            sentViewport = null
            pushViewport()
        },
        onClose: () => {
            stopFpsTimer()
            fps.value = 0
            clearFrameData()
            // The next socket has to be told again, even though nothing about
            // the viewport changed.
            sentViewport = null
        },
        onMessage: async (event) => {
            let buffer

            // Convert Blob to ArrayBuffer if needed
            if (event.data instanceof Blob) {
                buffer = await event.data.arrayBuffer()
            } else if (event.data instanceof ArrayBuffer) {
                buffer = event.data
            } else {
                return // Ignore non-binary messages
            }

            // Decode frame (dispatches by magic number)
            const decoded = decodeFrame(buffer)

            if (decoded) {
                frameData.value = decoded.frameData
                isJpeg.value = decoded.isJpeg || isDynamicJpeg
                dimensions.value = {width: decoded.width, height: decoded.height}
                frameNumber.value++
                framesSinceLastFPS++
                decodeError.value = null
            } else {
                decodeError.value = 'Failed to decode frame'
            }
        },
    })

    // Moving to the other source means a new socket: the server picks the stream from
    // the URL at upgrade time. The old frame is dropped rather than left on screen,
    // since it belongs to the camera we just stopped watching.
    watch(
        () => resolveEndpoint(),
        () => {
            disconnect()
            clearFrameData()
            connect()
        }
    )

    onUnmounted(() => {
        stopFpsTimer()
    })

    /**
     * Report the viewport this client will actually display, so the server resamples
     * to it instead of shipping a larger frame for the GPU to minify — matters most on
     * the lossless endpoint, where the browser's four-tap bilinear filter would
     * otherwise discard most of the averaging a server-side box downsample delivers.
     *
     * Callers may report on every layout change: remembered and de-duplicated here, so
     * a repeated size costs nothing and a reconnect replays the last one unprompted.
     */
    function sendResolution(w, h) {
        const next = normalizeViewport(w, h)
        if (next === null) return
        viewport = next
        pushViewport()
    }

    return {
        connected,
        error,
        decodeError,
        frameData,
        isJpeg,
        isDynamicJpeg,
        dimensions,
        frameNumber,
        fps,
        connect,
        disconnect,
        clearFrameData,
        sendResolution,
    }
}
