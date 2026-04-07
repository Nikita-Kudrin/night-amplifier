import {ref, onUnmounted, shallowRef} from 'vue'
import lz4 from 'lz4js'
import {WS_RECONNECT, RGB8_MAGIC} from '../constants'

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
    const lastError = ref(null)
    const diskWriterWarning = ref(null)

    // Push-To state
    const pushDirection = ref(null)
    const currentTarget = ref(null)
    const plateSolving = ref({inProgress: false, targetName: null, lastResult: null})

    // ASTAP installation state
    const astapInstallProgress = ref(null)

    // Catalog installation state
    const catalogInstallProgress = ref(null)

    const {connected, error, connect, disconnect} = useWebSocket('/ws/events', {
        onMessage: (event) => {
            try {
                const data = JSON.parse(event.data)
                lastEvent.value = data

                switch (data.type) {
                    case 'state_changed':
                        captureState.value = data.state
                        // Reset session counters when a new capture session starts
                        if (data.state === 'Starting') {
                            frameCount.value = 0
                            stackedCount.value = 0
                        }
                        break
                    case 'frame_captured':
                        frameCount.value = data.frame_number
                        stackedCount.value = data.stacked_count
                        break
                    case 'frame_rejected':
                        frameCount.value = data.frame_number
                        stackedCount.value = data.stacked_count
                        break
                    case 'error':
                        lastError.value = data.message
                        break
                    case 'settings_updated':
                        // Settings updated externally, components should refresh
                        break
                    case 'disk_writer_warning':
                        diskWriterWarning.value = data.queue_depth
                        break
                    case 'disk_writer_warning_cleared':
                        diskWriterWarning.value = null
                        break
                    case 'plate_solving_started':
                        console.log('[EventStream] Plate solving started:', data.target_name)
                        plateSolving.value = {
                            inProgress: true,
                            targetName: data.target_name,
                            lastResult: null,
                        }
                        break
                    case 'position_solved':
                        console.log('[EventStream] Position solved:', data.ra_degrees, data.dec_degrees)
                        plateSolving.value = {
                            inProgress: false,
                            targetName: plateSolving.value.targetName,
                            lastResult: 'success',
                        }
                        break
                    case 'position_solve_failed':
                        console.log('[EventStream] Position solve failed:', data.reason)
                        plateSolving.value = {
                            inProgress: false,
                            targetName: plateSolving.value.targetName,
                            lastResult: 'failed',
                        }
                        break
                    case 'push_direction_updated':
                        console.log('[EventStream] Push direction updated:', data.angle_deg, data.distance_deg, data.direction_hint, 'fov:', data.fov_deg)
                        pushDirection.value = {
                            angleDeg: data.angle_deg,
                            distanceDeg: data.distance_deg,
                            directionHint: data.direction_hint,
                            isClose: data.is_close,
                            fovDeg: data.fov_deg || 0,
                        }
                        break
                    case 'target_changed':
                        console.log('[EventStream] Target changed:', data.designation, data.ra_degrees, data.dec_degrees)
                        currentTarget.value = {
                            designation: data.designation,
                            ra_degrees: data.ra_degrees,
                            dec_degrees: data.dec_degrees,
                        }
                        break
                    case 'target_cleared':
                        console.log('[EventStream] Target cleared')
                        currentTarget.value = null
                        pushDirection.value = null
                        break
                    case 'astap_install_starting':
                        astapInstallProgress.value = {
                            component: data.component,
                            stage: 'starting',
                            percent: null,
                            bytesDownloaded: 0,
                            totalBytes: null,
                            overallPercent: null,
                            error: null,
                        }
                        break
                    case 'astap_install_progress':
                        astapInstallProgress.value = {
                            component: data.component,
                            stage: 'downloading',
                            percent: data.percent,
                            bytesDownloaded: data.bytes_downloaded,
                            totalBytes: data.total_bytes,
                            stageName: data.stage,
                            overallPercent: data.overall_percent,
                            error: null,
                        }
                        break
                    case 'astap_install_extracting':
                        astapInstallProgress.value = {
                            component: data.component,
                            stage: 'extracting',
                            percent: data.progress,
                            stageName: data.stage,
                            overallPercent: data.overall_percent,
                            error: null,
                        }
                        break
                    case 'astap_install_completed':
                        astapInstallProgress.value = {
                            component: data.component,
                            stage: 'completed',
                            stageName: data.stage,
                            overallPercent: data.overall_percent,
                            error: null,
                        }
                        break
                    case 'astap_install_failed':
                        astapInstallProgress.value = {
                            component: data.component,
                            stage: 'failed',
                            error: data.error,
                        }
                        break
                    case 'catalog_install_starting':
                        catalogInstallProgress.value = {
                            stage: 'starting',
                            fileName: '',
                            percent: null,
                            bytesDownloaded: 0,
                            totalBytes: null,
                            error: null,
                        }
                        break
                    case 'catalog_install_progress':
                        catalogInstallProgress.value = {
                            stage: 'downloading',
                            fileName: data.file_name,
                            percent: data.percent,
                            bytesDownloaded: data.bytes_downloaded,
                            totalBytes: data.total_bytes,
                            error: null,
                        }
                        break
                    case 'catalog_file_completed':
                        catalogInstallProgress.value = {
                            ...catalogInstallProgress.value,
                            fileName: data.file_name,
                            stage: 'file_completed',
                        }
                        break
                    case 'catalog_install_completed':
                        catalogInstallProgress.value = {
                            stage: 'completed',
                            object_count: data.object_count,
                            error: null,
                        }
                        break
                    case 'catalog_install_failed':
                        catalogInstallProgress.value = {
                            stage: 'failed',
                            error: data.error,
                        }
                        break
                }
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
        lastError,
        diskWriterWarning,
        pushDirection,
        currentTarget,
        plateSolving,
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
 * Decode RGB8+LZ4 binary stream format
 *
 * Header layout (16 bytes):
 * - bytes 0-3:   Magic number "SA08" (0x53413038)
 * - bytes 4-7:   Width (u32, little-endian)
 * - bytes 8-11:  Height (u32, little-endian)
 * - bytes 12-15: Compressed size (u32, little-endian)
 *
 * Followed by LZ4-compressed RGB8 pixel data (3 bytes per pixel)
 *
 * @param {ArrayBuffer} buffer - Raw binary data from WebSocket
 * @returns {object|null} Decoded frame { width, height, frameData } or null if invalid
 */
function decodeRgb8Lz4(buffer) {
    const view = new DataView(buffer)

    if (buffer.byteLength < 16) return null;

    const magic = view.getUint32(0, true)
    if (magic !== RGB8_MAGIC) {
        console.error('Invalid magic number, expected SA08:', magic.toString(16))
        return null
    }

    const width = view.getUint32(4, true)
    const height = view.getUint32(8, true)
    const compressedSize = view.getUint32(12, true)

    const compressedData = new Uint8Array(buffer, 16, compressedSize)
    const decompressedSize =
        compressedData[0] | (compressedData[1] << 8) |
        (compressedData[2] << 16) | (compressedData[3] << 24)

    const lz4BlockData = new Uint8Array(buffer, 20, compressedSize - 4)

    let decompressedBuffer
    try {
        decompressedBuffer = lz4.makeBuffer(decompressedSize)
        lz4.decompressBlock(lz4BlockData, decompressedBuffer, 0, lz4BlockData.length, 0)
    } catch (e) {
        console.error('Decompression failed:', e)
        return null
    }

    // decompressedBuffer is a Uint8Array from lz4js
    return {width, height, frameData: decompressedBuffer}
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
    const decodeError = ref(null)

    /**
     * Clear frame data to reset the live view
     * Called when starting a new capture session
     */
    function clearFrameData() {
        frameData.value = null
        dimensions.value = {width: 0, height: 0}
        frameNumber.value = 0
        decodeError.value = null
    }

    const {connected, error, connect, disconnect} = useWebSocket('/ws/stream', {
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
            const decoded = decodeRgb8Lz4(buffer)

            if (decoded) {
                frameData.value = decoded.frameData
                dimensions.value = {width: decoded.width, height: decoded.height}
                frameNumber.value++
                decodeError.value = null
            } else {
                decodeError.value = 'Failed to decode frame'
            }
        },
    })

    return {
        connected,
        error,
        decodeError,
        frameData,
        dimensions,
        frameNumber,
        connect,
        disconnect,
        clearFrameData,
    }
}
