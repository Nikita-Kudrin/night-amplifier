import {
    MockWebSocket,
    RGB8_BYTES_PER_PIXEL,
    createTestContextWithoutTimers,
    cleanupTestContextWithoutTimers,
    getWebSocket,
    openWebSocket,
    waitForAsyncProcessing,
    suppressConsoleErrors,
    createTestFrame,
    createRgb8Lz4Buffer,
    createInvalidMagicBuffer,
    setupGlobalWebSocketMock,
} from './webSocketTestUtils.js'

setupGlobalWebSocketMock()

import {useImageStream as originalUseImageStream} from '../useWebSocket.js'
import { mount } from '@vue/test-utils'

let currentApp = null;
function useImageStream(options) {
    let result;
    currentApp = mount({
        setup() {
            result = originalUseImageStream(options)
            return () => {}
        }
    })
    return result
}

describe('useImageStream', () => {
    beforeEach(createTestContextWithoutTimers)
    afterEach(() => {
        cleanupTestContextWithoutTimers()
        if (currentApp) {
            currentApp.unmount()
            currentApp = null
        }
    })

    describe('resolution reporting', () => {
        // The lossless stream sizes its output from this report, exactly like
        // the JPEG stream does. Before it did, the server always sent a
        // near-native frame and left the browser to minify it with a four-tap
        // bilinear filter, which discards most of the noise averaging a
        // server-side box downsample delivers.
        it('reports the viewport on the lossless endpoint', async () => {
            const {sendResolution} = useImageStream({endpoint: '/ws/eyepiece_quality'})
            await openWebSocket()

            sendResolution(1440, 1440)

            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 1440, height: 1440})
        })

        it('reports the viewport on the JPEG endpoint', async () => {
            const {sendResolution} = useImageStream({endpoint: '/ws/eyepiece'})
            await openWebSocket()

            sendResolution(1920, 1080)

            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 1920, height: 1080})
        })

        it('rounds fractional device-pixel sizes', async () => {
            const {sendResolution} = useImageStream({endpoint: '/ws/eyepiece_quality'})
            await openWebSocket()

            sendResolution(719.5, 1439.4)

            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 720, height: 1439})
        })

        // The server registers a fresh tier for every connection, so a socket
        // that reconnects without re-reporting is served at the server's
        // default. The lossless stream used to lose its viewport this way and
        // spend the rest of the session at 1080p — below where it started.
        it('replays the viewport after a reconnect', async () => {
            const {sendResolution} = useImageStream({endpoint: '/ws/eyepiece_quality'})
            await openWebSocket()
            sendResolution(1440, 1440)
            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 1440, height: 1440})

            MockWebSocket.lastSent = null
            getWebSocket().simulateClose()
            await openWebSocket()

            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 1440, height: 1440})
        })

        // ...but on one socket a repeat is not re-sent, so a component may
        // report on every layout tick without putting a write on each one.
        it('does not repeat an unchanged viewport on the same socket', async () => {
            const {sendResolution} = useImageStream({endpoint: '/ws/eyepiece_quality'})
            await openWebSocket()
            sendResolution(1440, 1440)

            MockWebSocket.lastSent = null
            sendResolution(1440, 1440)
            expect(MockWebSocket.lastSent).toBeNull()

            sendResolution(1080, 1080)
            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 1080, height: 1080})
        })

        // A report made before the socket opened is dropped by `send`. Treating
        // it as delivered is what left the stream on the default tier.
        it('re-sends a viewport reported before the socket opened', async () => {
            const {sendResolution} = useImageStream({endpoint: '/ws/eyepiece_quality'})
            sendResolution(1440, 1440)
            expect(MockWebSocket.lastSent).toBeNull()

            await openWebSocket()
            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 1440, height: 1440})
        })

        // A caller that knows its viewport up front should not have to wait for
        // the socket before it counts.
        it('reports an initial viewport on open, on the lossless endpoint too', async () => {
            useImageStream({endpoint: '/ws/eyepiece_quality', width: 1440, height: 1440})
            await openWebSocket()

            expect(JSON.parse(MockWebSocket.lastSent)).toEqual({width: 1440, height: 1440})
        })

        // A canvas that has not been laid out yet reports 0, which would
        // otherwise be sent as a viewport and clamp the server to its smallest tier.
        it('ignores a zero-sized viewport', async () => {
            const {sendResolution} = useImageStream({endpoint: '/ws/eyepiece_quality'})
            await openWebSocket()
            MockWebSocket.lastSent = null

            sendResolution(0, 0)

            expect(MockWebSocket.lastSent).toBeNull()
        })
    })

    it('connects to /ws/stream', () => {
        useImageStream()

        expect(MockWebSocket.instances).toHaveLength(1)
        expect(getWebSocket().url).toContain('/ws/stream')
    })

    describe('frame decoding', () => {
        it('decodes valid RGB8+LZ4 ArrayBuffer message', async () => {
            const {frameData, dimensions, frameNumber, decodeError} = useImageStream()

            await openWebSocket()
            getWebSocket().simulateMessage(createTestFrame(2, 2))
            await waitForAsyncProcessing()

            expect(decodeError.value).toBe(null)
            expect(dimensions.value).toEqual({width: 2, height: 2})
            expect(frameNumber.value).toBe(1)
            expect(frameData.value).not.toBe(null)
            expect(frameData.value.length).toBe(2 * 2 * 3) // 3 bytes per pixel
        })

        it('decodes valid RGB8+LZ4 Blob message', async () => {
            const {dimensions, frameNumber} = useImageStream()

            const blob = new Blob([createTestFrame(2, 2, 200)])

            await openWebSocket()
            getWebSocket().simulateMessage(blob)
            await waitForAsyncProcessing()

            expect(dimensions.value).toEqual({width: 2, height: 2})
            expect(frameNumber.value).toBe(1)
        })

        it('increments frameNumber for each successful decode', async () => {
            const {frameNumber} = useImageStream()

            await openWebSocket()

            for (let i = 1; i <= 3; i++) {
                getWebSocket().simulateMessage(createTestFrame())
                await waitForAsyncProcessing()
                expect(frameNumber.value).toBe(i)
            }
        })

        it('updates dimensions when frame size changes', async () => {
            const {dimensions} = useImageStream()

            await openWebSocket()

            getWebSocket().simulateMessage(createTestFrame(2, 2))
            await waitForAsyncProcessing()
            expect(dimensions.value).toEqual({width: 2, height: 2})
        })

        it('ignores non-binary messages', async () => {
            const {frameData, frameNumber} = useImageStream()

            await openWebSocket()
            getWebSocket().simulateMessage('text message')
            await waitForAsyncProcessing()

            expect(frameData.value).toBe(null)
            expect(frameNumber.value).toBe(0)
        })
    })

    describe('error handling', () => {
        it('sets decodeError for invalid magic number', async () => {
            const {decodeError, frameData} = useImageStream()
            const consoleSpy = suppressConsoleErrors()

            await openWebSocket()
            getWebSocket().simulateMessage(createInvalidMagicBuffer())
            await waitForAsyncProcessing()

            expect(decodeError.value).toBe('Failed to decode frame')
            expect(frameData.value).toBe(null)
            expect(consoleSpy).toHaveBeenCalled()

            consoleSpy.mockRestore()
        })

        it('sets decodeError for buffer too small', async () => {
            const {decodeError} = useImageStream()
            const consoleSpy = suppressConsoleErrors()

            await openWebSocket()
            getWebSocket().simulateMessage(new ArrayBuffer(10))
            await waitForAsyncProcessing()

            expect(decodeError.value).toBe('Failed to decode frame')

            consoleSpy.mockRestore()
        })

        it('clears decodeError on successful decode after error', async () => {
            const {decodeError} = useImageStream()
            const consoleSpy = suppressConsoleErrors()

            await openWebSocket()

            getWebSocket().simulateMessage(new ArrayBuffer(5))
            await waitForAsyncProcessing()
            expect(decodeError.value).toBe('Failed to decode frame')

            getWebSocket().simulateMessage(createTestFrame())
            await waitForAsyncProcessing()
            expect(decodeError.value).toBe(null)

            consoleSpy.mockRestore()
        })
    })

    describe('pixel data integrity', () => {
        it('preserves RGB8 pixel data correctly', async () => {
            const {frameData, dimensions} = useImageStream()

            const width = 4
            const height = 4
            const pixelData = new Uint8Array(width * height * RGB8_BYTES_PER_PIXEL)
            pixelData.fill(0)

            const buffer = createRgb8Lz4Buffer(width, height, pixelData)

            await openWebSocket()
            getWebSocket().simulateMessage(buffer)
            await waitForAsyncProcessing()

            expect(frameData.value).not.toBe(null)
            expect(dimensions.value).toEqual({width, height})
            expect(frameData.value.length).toBe(width * height * 3) // 3 bytes per pixel
        })
    })
})
