import {nextTick} from 'vue'
import {
    createTestContext,
    cleanupTestContext,
    getWebSocket,
    openWebSocket,
    createTestFrame,
    setupGlobalWebSocketMock,
} from './webSocketTestUtils.js'

setupGlobalWebSocketMock()

import {useImageStream} from '../useWebSocket.js'

describe('useImageStream FPS calculation', () => {
    beforeEach(createTestContext)
    afterEach(cleanupTestContext)

    it('calculates FPS correctly over 3 second interval', async () => {
        const {fps} = useImageStream()

        await openWebSocket()

        // Simulate 30 frames in 3 seconds (10 FPS)
        for (let i = 0; i < 30; i++) {
            getWebSocket().simulateMessage(createTestFrame())
            // Advance timers by 10ms to satisfy waitForAsyncProcessing's setTimeout
            vi.advanceTimersByTime(10)
            await nextTick()
        }

        // Initially FPS is 0
        expect(fps.value).toBe(0)

        // Advance time by 3 seconds
        vi.advanceTimersByTime(3000)

        // FPS should be 30 / 3 = 10
        expect(fps.value).toBe(10)

        // Simulate 15 frames in next 3 seconds (5 FPS)
        for (let i = 0; i < 15; i++) {
            getWebSocket().simulateMessage(createTestFrame())
            vi.advanceTimersByTime(10)
            await nextTick()
        }

        vi.advanceTimersByTime(3000)

        expect(fps.value).toBe(5)
    })

    it('resets FPS on disconnection', async () => {
        const {fps} = useImageStream()

        await openWebSocket()

        for (let i = 0; i < 30; i++) {
            getWebSocket().simulateMessage(createTestFrame())
            vi.advanceTimersByTime(10)
            await nextTick()
        }

        vi.advanceTimersByTime(3000)
        expect(fps.value).toBe(10)

        getWebSocket().simulateClose()
        vi.advanceTimersByTime(10)
        await nextTick()

        expect(fps.value).toBe(0)
    })

    it('rounds FPS to the nearest whole number', async () => {
        const {fps} = useImageStream()

        await openWebSocket()

        // Simulate 2 frames in 3 seconds (0.66 FPS -> 1 FPS)
        for (let i = 0; i < 2; i++) {
            getWebSocket().simulateMessage(createTestFrame())
            vi.advanceTimersByTime(10)
            await nextTick()
        }
        vi.advanceTimersByTime(3000)
        expect(fps.value).toBe(1)

        // Simulate 4 frames in 3 seconds (1.33 FPS -> 1 FPS)
        for (let i = 0; i < 4; i++) {
            getWebSocket().simulateMessage(createTestFrame())
            vi.advanceTimersByTime(10)
            await nextTick()
        }
        vi.advanceTimersByTime(3000)
        expect(fps.value).toBe(1)

        // Simulate 5 frames in 3 seconds (1.66 FPS -> 2 FPS)
        for (let i = 0; i < 5; i++) {
            getWebSocket().simulateMessage(createTestFrame())
            vi.advanceTimersByTime(10)
            await nextTick()
        }
        vi.advanceTimersByTime(3000)
        expect(fps.value).toBe(2)
    })
})
