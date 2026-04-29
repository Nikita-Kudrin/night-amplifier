import {ref, onUnmounted, shallowRef, computed} from 'vue'
import {WS_RECONNECT} from '../constants'
import {decodeFrame} from '../utils/frameDecoder.js'

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
        maxReconnectAttempts = WS_RECONNECT.maxAttempts,
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
        return `${protocol}//${host}${path}`
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
            ws = new WebSocket(getUrl())

            ws.onopen = () => {
                connected.value = true
                reconnectAttempts.value = 0
                startPing()
                onOpen?.()
            }

            ws.onclose = (event) => {
                connected.value = false
                stopPing()
                onClose?.(event)

                if (reconnect && reconnectAttempts.value < maxReconnectAttempts) {
                    scheduleReconnect()
                }
            }

            ws.onerror = (event) => {
                error.value = 'WebSocket error'
                onError?.(event)
            }

            ws.onmessage = (event) => {
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
            ws.close()
            ws = null
        }

        connected.value = false
    }

    /**
     * Send a message
     * @param {string|ArrayBuffer|Blob} data - Data to send
     */
    function send(data) {
        if (ws && ws.readyState === WebSocket.OPEN) {
            ws.send(data)
        }
    }

    /**
     * Schedule a reconnection attempt
     */
    function scheduleReconnect() {
        clearTimeout(reconnectTimer)
        reconnectAttempts.value++
        reconnectTimer = setTimeout(connect, reconnectInterval)
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
    const lastError = ref(null)
    const diskWriterWarning = ref(null)

    // Push-To state
    const pushDirection = ref(null)
    const currentTarget = ref(null)
    const plateSolving = ref({inProgress: false, targetName: null, lastResult: null})

    const solvingMessage = computed(() => {
        const {inProgress, lastResult, targetName} = plateSolving.value
        const targetSuffix = targetName ? ` : ${targetName}` : ''

        if (inProgress) {
            return `Searching${targetSuffix}`
        }

        if (lastResult === 'success') {
            return `Found${targetSuffix}`
        }

        if (lastResult === 'failed') {
            return `Failed to find${targetSuffix}`
        }

        return null
    })

    // ASTAP installation state
    const astapInstallProgress = ref(null)

    // Catalog installation state
    const catalogInstallProgress = ref(null)

    function handleFrameEvent(data) {
        frameCount.value = data.frame_number
        stackedCount.value = data.stacked_count
        rejectedCount.value = data.rejected_count
    }

    const eventHandlers = {
        state_changed(data) {
            captureState.value = data.state
            if (data.state === 'Starting') {
                frameCount.value = 0
                stackedCount.value = 0
                rejectedCount.value = 0
                droppedCount.value = 0
            }
        },
        frame_captured: handleFrameEvent,
        frame_rejected: handleFrameEvent,
        error(data) {
            lastError.value = data.message
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
        },
        plate_solving_started(data) {
            plateSolving.value = {inProgress: true, targetName: data.target_name, lastResult: null}
        },
        position_solved() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName, lastResult: 'success',
            }
        },
        position_solve_failed() {
            plateSolving.value = {
                inProgress: false, targetName: plateSolving.value.targetName, lastResult: 'failed',
            }
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
        plateSolving.value = {inProgress: false, targetName: null, lastResult: null}
    }

    function clearAstapInstallProgress() {
        astapInstallProgress.value = null
    }

    function clearCatalogInstallProgress() {
        catalogInstallProgress.value = null
    }

    return {
        connected,
        error,
        lastEvent,
        captureState,
        frameCount,
        stackedCount,
        rejectedCount,
        droppedCount,
        lastError,
        diskWriterWarning,
        pushDirection,
        currentTarget,
        plateSolving,
        solvingMessage,
        astapInstallProgress,
        catalogInstallProgress,
        clearError,
        clearDiskWriterWarning,
        clearPushDirection,
        clearPlateSolving,
        clearAstapInstallProgress,
        clearCatalogInstallProgress,
        connect,
        disconnect,
    }
}

/**
 * WebSocket composable for high-quality RGB16 image streaming
 *
 * Receives RGB16+LZ4 compressed frames and provides:
 * - rgb16Data: Raw 16-bit RGB pixel data for WebGL rendering
 * - dimensions: { width, height } of the current frame
 *
 * @returns {object} Image stream state and methods
 */
export function useImageStream() {
    // Use shallowRef for large binary data to avoid deep reactivity overhead
    const frameData = shallowRef(null)
    const dimensions = ref({width: 0, height: 0})
    const frameNumber = ref(0)
    const fps = ref(0)
    const decodeError = ref(null)

    let framesSinceLastFPS = 0
    let fpsTimer = null

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
            fps.value = parseFloat((framesSinceLastFPS / 3).toFixed(1))
            framesSinceLastFPS = 0
        }, 3000)
    }

    function stopFpsTimer() {
        if (fpsTimer) {
            clearInterval(fpsTimer)
            fpsTimer = null
        }
    }

    const {connected, error, connect, disconnect} = useWebSocket('/ws/stream', {
        onOpen: () => {
            startFpsTimer()
        },
        onClose: () => {
            stopFpsTimer()
            fps.value = 0
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

            // Decode RGB8+LZ4 format
            const decoded = decodeFrame(buffer)

            if (decoded) {
                frameData.value = decoded.frameData
                dimensions.value = {width: decoded.width, height: decoded.height}
                frameNumber.value++
                framesSinceLastFPS++
                decodeError.value = null
            } else {
                decodeError.value = 'Failed to decode frame'
            }
        },
    })

    onUnmounted(() => {
        stopFpsTimer()
    })

    return {
        connected,
        error,
        decodeError,
        frameData,
        dimensions,
        frameNumber,
        fps,
        connect,
        disconnect,
        clearFrameData,
    }
}
