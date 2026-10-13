import {ref, shallowRef, toValue, watch} from 'vue'
import {tryOnScopeDispose} from './lifecycle.js'
import {decodeFrame} from '../utils/frameDecoder.js'
import {useWebSocket} from './useWebSocket.js'
import {WS} from '../constants/endpoints.js'

/** Text the server sends when an eyepiece socket switches to a camera with no frame yet. */
export const NO_FRAME = 'no_frame'

/**
 * WebSocket composable for the image streams: receives JPEG (SA10) or RGB8+LZ4 (SA09)
 * frames, exposing `frameData`/`dimensions`. The server decides size from the Streaming
 * Resolution settings and sends every client of an endpoint the same frame — nothing to negotiate here.
 * @param {object} options - Stream options
 * @param {string} options.endpoint - WebSocket endpoint (default: `WS.STREAM`)
 * @returns {object} Image stream state and methods
 */
export function useImageStream(options = {}) {
    // May be a plain string, a ref or a getter — the live view passes a getter so the
    // Guide camera toggle can move this socket to the other source.
    const endpointSource = options.endpoint ?? WS.STREAM
    const resolveEndpoint = () => toValue(endpointSource) || WS.STREAM

    // Selects the decoder.
    const isDynamicJpeg = !resolveEndpoint().startsWith(WS.EYEPIECE_QUALITY)

    // Use shallowRef for large binary data to avoid deep reactivity overhead
    const frameData = shallowRef(null)
    const isJpeg = ref(isDynamicJpeg)
    const dimensions = ref({width: 0, height: 0})
    const frameNumber = ref(0)
    const fps = ref(0)
    const decodeError = ref(null)

    let framesSinceLastFPS = 0
    let fpsTimer = null
    // Bumped by every clear (`NO_FRAME`, a source switch, a close), so a Blob still being
    // read across one is dropped rather than shown as the source just left.
    let clearGeneration = 0

    /**
     * Clear frame data to reset the live view
     * Called when starting a new capture session
     */
    function clearFrameData() {
        clearGeneration++
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

    const {connected, error, connect, disconnect} = useWebSocket(resolveEndpoint, {
        onOpen: () => {
            startFpsTimer()
        },
        onClose: () => {
            stopFpsTimer()
            fps.value = 0
            clearFrameData()
        },
        onMessage: async (event) => {
            let buffer

            if (event.data === NO_FRAME) {
                // The server switched this socket to a camera with nothing to show yet: the
                // picture on screen is the other camera's.
                clearFrameData()
                return
            }

            // Convert Blob to ArrayBuffer if needed
            if (event.data instanceof Blob) {
                const generation = clearGeneration
                buffer = await event.data.arrayBuffer()
                // A frame that arrived before a clear must not land after it.
                if (generation !== clearGeneration) return
            } else if (event.data instanceof ArrayBuffer) {
                buffer = event.data
            } else {
                return // Ignore other text
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

    tryOnScopeDispose(stopFpsTimer)

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
    }
}
