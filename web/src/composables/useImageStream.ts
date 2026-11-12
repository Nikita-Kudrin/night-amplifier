import {ref, shallowRef, toValue, watch} from 'vue'
import type {MaybeRefOrGetter} from 'vue'
import {tryOnScopeDispose} from './lifecycle'
import {decodeFrame} from '../utils/frameDecoder'
import type {DecodedFrame} from '../utils/frameDecoder'
import {useWebSocket} from './useWebSocket'
import {WS} from '../constants/endpoints'

/** Text the server sends when an eyepiece socket switches to a camera with no frame yet. */
export const NO_FRAME = 'no_frame'

/**
 * How long an image socket may answer nothing before it is replaced. Far longer than the
 * events socket's: the `pong` queues behind the frame being sent, and one lossless frame
 * (megabytes) on weak Wi-Fi outlasted a minute — the socket was cut mid-frame, over and
 * over, and the page never showed a picture. Still bounded, so a dead peer is replaced.
 */
export const IMAGE_LIVENESS_MS = 5 * 60 * 1000

/** A stream's frame as it arrives: RGB8 pixels, or a JPEG blob still to be decoded. */
export type FrameData = DecodedFrame['frameData']

export interface FrameSize {
    width: number
    height: number
}

export interface ImageStreamOptions {
    /** WebSocket endpoint (default: `WS.STREAM`) */
    endpoint?: MaybeRefOrGetter<string>
}

/**
 * WebSocket composable for the image streams: receives JPEG (SA10) or RGB8+LZ4 (SA09)
 * frames, exposing `frameData`/`dimensions`. The server decides size from the Streaming
 * Resolution settings and sends every client of an endpoint the same frame — nothing to negotiate here.
 * @returns Image stream state and methods
 */
export function useImageStream(options: ImageStreamOptions = {}) {
    // May be a plain string, a ref or a getter — the live view passes a getter so the
    // Guide camera toggle can move this socket to the other source.
    const endpointSource = options.endpoint ?? WS.STREAM
    const resolveEndpoint = () => toValue(endpointSource) || WS.STREAM

    // Selects the decoder.
    const isDynamicJpeg = !resolveEndpoint().startsWith(WS.EYEPIECE_QUALITY)

    // Use shallowRef for large binary data to avoid deep reactivity overhead
    const frameData = shallowRef<FrameData | null>(null)
    const isJpeg = ref(isDynamicJpeg)
    const dimensions = ref<FrameSize>({width: 0, height: 0})
    const frameNumber = ref(0)
    const fps = ref(0)
    const decodeError = ref<string | null>(null)

    let framesSinceLastFPS = 0
    let fpsTimer: ReturnType<typeof setInterval> | null = null
    // The newest frame not yet decoded; one a newer frame replaces here is never decoded.
    let pending: ArrayBuffer | null = null
    let decodeTimer: ReturnType<typeof setTimeout> | undefined
    let pixelBuffers: Uint8Array[] = []

    /**
     * Clear frame data to reset the live view
     * Called when starting a new capture session
     */
    function clearFrameData() {
        dropPending()
        frameData.value = null
        dimensions.value = {width: 0, height: 0}
        frameNumber.value = 0
        fps.value = 0
        framesSinceLastFPS = 0
        decodeError.value = null
    }

    function dropPending() {
        pending = null
        clearTimeout(decodeTimer)
        decodeTimer = undefined
    }

    /**
     * Two lossless buffers taken in turn, never the one on screen, which the renderer may
     * draw again while the next frame decodes (a restored context). A new one a frame was
     * 27 MB of garbage per 3008x3008 frame on the kiosk.
     */
    function spareBuffer(size: number) {
        if (pixelBuffers[0]?.length !== size) pixelBuffers = [new Uint8Array(size), new Uint8Array(size)]
        return pixelBuffers[0] === frameData.value ? pixelBuffers[1] : pixelBuffers[0]
    }

    /**
     * Decodes the newest frame, as a task of its own: frames that queued up behind a slow
     * decode only replace `pending`. That holds for small frames (JPEG): Chromium hands a
     * megabyte frame over in several tasks, and this one runs first (2 MB, 60 ms decodes:
     * 30 of 30 decoded; 20 KB, 200 ms: 6 of 30). A lossless page that falls behind is paced
     * by the server instead, which sends each client only its newest frame once the last
     * one is through.
     */
    function decodePending() {
        decodeTimer = undefined
        const buffer = pending
        pending = null
        if (!buffer) return

        const decoded = decodeFrame(buffer, spareBuffer)
        if (!decoded) {
            decodeError.value = 'Failed to decode frame'
            return
        }
        frameData.value = decoded.frameData
        isJpeg.value = decoded.isJpeg || isDynamicJpeg
        dimensions.value = {width: decoded.width, height: decoded.height}
        frameNumber.value++
        framesSinceLastFPS++
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

    const {connected, error, connect, disconnect} = useWebSocket<ArrayBuffer | string>(resolveEndpoint, {
        livenessTimeout: IMAGE_LIVENESS_MS,
        // A Blob was read back into an ArrayBuffer: one more copy of every frame, made
        // through the browser's blob store.
        binaryType: 'arraybuffer',
        onOpen: () => {
            startFpsTimer()
        },
        onClose: () => {
            stopFpsTimer()
            fps.value = 0
            clearFrameData()
        },
        onMessage: (event) => {
            if (event.data === NO_FRAME) {
                // The server switched this socket to a camera with nothing to show yet, or
                // cleared its stream (a new session, a camera gone): the picture on screen
                // no longer belongs to anything.
                clearFrameData()
                return
            }
            if (!(event.data instanceof ArrayBuffer)) return // Ignore other text

            pending = event.data
            decodeTimer ??= setTimeout(decodePending, 0)
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

    tryOnScopeDispose(() => {
        stopFpsTimer()
        dropPending()
    })

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

export type ImageStream = ReturnType<typeof useImageStream>
