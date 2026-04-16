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

import {useImageStream} from '../useWebSocket.js'

describe('useImageStream', () => {
    beforeEach(createTestContextWithoutTimers)
    afterEach(cleanupTestContextWithoutTimers)

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
