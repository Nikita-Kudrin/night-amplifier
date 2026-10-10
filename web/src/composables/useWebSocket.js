import {ref, onUnmounted, toValue} from 'vue'
import {WS_RECONNECT} from '../constants'

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
     * The attempt count used to be capped: a client unreachable for thirty seconds stayed
     * dead until reloaded — no keyboard to do that on the kiosk running `/eyepiece_quality`,
     * so a restarted server left it black till morning. The backoff ceiling (not a cap) is
     * what keeps a server down till morning from being polled every second instead.
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
