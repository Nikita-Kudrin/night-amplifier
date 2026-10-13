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

import {nextTick, ref} from 'vue'
import {IMAGE_LIVENESS_MS, NO_FRAME, useImageStream as originalUseImageStream} from '../useImageStream.js'
import {WS_RECONNECT} from '../../constants'
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

    // The server sizes every frame from its Streaming Resolution settings, so a stream
    // socket has nothing to tell it. Reports from older builds are ignored server-side,
    // but none may be sent: not on open, not after a reconnect, on no endpoint.
    describe('no viewport negotiation', () => {
        for (const endpoint of ['/ws/stream', '/ws/eyepiece', '/ws/eyepiece_quality']) {
            it(`sends nothing on open or reconnect to ${endpoint}`, async () => {
                const stream = useImageStream({endpoint})
                expect(stream).not.toHaveProperty('sendResolution')

                await openWebSocket()
                expect(MockWebSocket.lastSent).toBeNull()

                getWebSocket().simulateClose()
                await openWebSocket()
                expect(MockWebSocket.lastSent).toBeNull()
            })
        }
    })

    // One lossless frame on weak Wi-Fi outlasted the events socket's minute, which cut the
    // socket mid-frame, over and over: an image socket waits far longer, but not forever.
    it('keeps a silent socket through a slow frame, and drops it once that is all it does', async () => {
        vi.useFakeTimers()
        try {
            const {connected} = useImageStream({endpoint: '/ws/eyepiece_quality'})
            await openWebSocket()
            const ping = WS_RECONNECT.pingInterval

            vi.advanceTimersByTime(ping + 2 * ping)
            expect(connected.value).toBe(true)
            vi.advanceTimersByTime(IMAGE_LIVENESS_MS)

            expect(connected.value).toBe(false)
        } finally {
            vi.useRealTimers()
        }
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

        // The eyepiece sockets switch camera in place; with nothing from the new one yet,
        // the server says so rather than leave the old camera's picture up as its own.
        it('drops the picture on no_frame', async () => {
            const {frameData, dimensions} = useImageStream({endpoint: '/ws/eyepiece_quality'})

            await openWebSocket()
            getWebSocket().simulateMessage(createTestFrame(2, 2))
            await waitForAsyncProcessing()
            expect(frameData.value).not.toBe(null)

            getWebSocket().simulateMessage(NO_FRAME)
            await waitForAsyncProcessing()

            expect(frameData.value).toBe(null)
            expect(dimensions.value).toEqual({width: 0, height: 0})
        })

        it('does not let a frame still being read land after no_frame', async () => {
            const {frameData} = useImageStream({endpoint: '/ws/eyepiece_quality'})

            await openWebSocket()
            getWebSocket().simulateMessage(new Blob([createTestFrame(2, 2)]))
            getWebSocket().simulateMessage(NO_FRAME)
            await waitForAsyncProcessing()

            expect(frameData.value).toBe(null)
        })

        it('shows the next camera\'s frame after no_frame', async () => {
            const {dimensions} = useImageStream({endpoint: '/ws/eyepiece_quality'})

            await openWebSocket()
            getWebSocket().simulateMessage(NO_FRAME)
            getWebSocket().simulateMessage(new Blob([createTestFrame(4, 2)]))
            await waitForAsyncProcessing()

            expect(dimensions.value).toEqual({width: 4, height: 2})
        })

        // Frames arrive as Blobs: one still being read when the source switches (the live
        // view's Guide toggle) or the socket closes belongs to what was just left.
        describe('a frame read across a clear', () => {
            function slowBlob(buffer) {
                const blob = new Blob([buffer])
                let release
                const read = new Promise((resolve) => { release = () => resolve(buffer) })
                blob.arrayBuffer = () => read
                return {blob, release: () => release()}
            }

            it('drops a frame of the old source that finishes reading after the switch', async () => {
                const source = ref('/ws/eyepiece_quality')
                const {frameData} = useImageStream({endpoint: () => source.value})
                await openWebSocket()
                const pending = slowBlob(createTestFrame(2, 2))
                getWebSocket().simulateMessage(pending.blob)

                source.value = '/ws/eyepiece_quality?source=guide'
                await nextTick()
                pending.release()
                await waitForAsyncProcessing()

                expect(frameData.value).toBe(null)
            })

            it('drops a frame that finishes reading after the socket closed', async () => {
                const {frameData} = useImageStream({endpoint: '/ws/eyepiece_quality'})
                await openWebSocket()
                const pending = slowBlob(createTestFrame(2, 2))
                getWebSocket().simulateMessage(pending.blob)

                getWebSocket().simulateClose()
                pending.release()
                await waitForAsyncProcessing()

                expect(frameData.value).toBe(null)
            })
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
